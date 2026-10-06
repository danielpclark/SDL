// Tests of the GPU front end: the format tables against upstream's C, the
// driver selection, and the dispatch and debug-mode validation over a mock
// backend that records its calls.

use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::sysgpu::{
    blit_common, fetch_blit_pipeline, BackendCommandBuffer, BackendDevice, BackendObject,
    BackendSwapchainTexture, BlitPipelineCache, BlitShaders, ComputePipelineHeader, GpuBootstrap,
    GpuDriver, GraphicsPipelineHeader,
};
use super::*;
use crate::assert::{self, AssertState};
use crate::events::window::WindowFlags;
use crate::init::{self, InitFlags};
use crate::video::sysvideo::VideoDriver;

// Assertions

/// Records the debug layer's assertions (and ignores them) while alive.
struct Asserts(Arc<Mutex<Vec<String>>>);

impl Asserts {
    fn catch() -> Asserts {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        assert::set_assertion_handler(move |data| {
            // `!"message"` -> `message`
            let c = data.condition();
            let msg = c
                .strip_prefix("!\"")
                .and_then(|c| c.strip_suffix('"'))
                .unwrap_or(c);
            s.lock().unwrap().push(msg.to_string());
            AssertState::Ignore
        });
        Asserts(seen)
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

impl Drop for Asserts {
    fn drop(&mut self) {
        assert::reset_assertion_handler();
        assert::reset_assertion_report();
    }
}

// The mock backend

/// What the mock backend does, set by each test.
struct MockConfig {
    /// Whether each mock's `PrepareDriver` succeeds.
    prepare: [bool; 2],
    shader_formats: ShaderFormat,
    /// Formats `SupportsTextureFormat` rejects.
    unsupported: Vec<TextureFormat>,
    /// The swapchain texture `AcquireSwapchainTexture` returns.
    swapchain: bool,
}

static CONFIG: Mutex<MockConfig> = Mutex::new(MockConfig {
    prepare: [true, true],
    shader_formats: ShaderFormat::SPIRV,
    unsupported: Vec::new(),
    swapchain: true,
});

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

fn config() -> MutexGuard<'static, MockConfig> {
    CONFIG.lock().unwrap_or_else(|e| e.into_inner())
}

fn log(line: String) {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push(line);
}

/// The calls since the last `take_log()`.
fn take_log() -> Vec<String> {
    std::mem::take(&mut *LOG.lock().unwrap_or_else(|e| e.into_inner()))
}

/// A backend object: a name for the log, and memory for transfer buffers.
struct MockObject {
    name: String,
    data: Mutex<Vec<u8>>,
}

/// Whether dropping a [`MockObject`] is logged (off but in the tests that
/// check the order of drops).
static LOG_DROPS: AtomicBool = AtomicBool::new(false);

impl Drop for MockObject {
    fn drop(&mut self) {
        if LOG_DROPS.load(Ordering::Relaxed) {
            log(format!("drop {}", self.name));
        }
    }
}

fn object(kind: &str, size: usize) -> BackendObject {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    BackendObject::new(MockObject {
        name: format!("{kind}#{id}"),
        data: Mutex::new(vec![0; size]),
    })
}

fn name(raw: &BackendObject) -> String {
    raw.downcast_ref::<MockObject>()
        .map(|o| o.name.clone())
        .unwrap_or_else(|| "?".into())
}

fn tex(t: &Texture) -> String {
    name(&t.raw)
}

struct MockCmd {
    id: u32,
}

fn cmd(c: &BackendCommandBuffer) -> u32 {
    c.downcast_ref::<MockCmd>().map(|c| c.id).unwrap_or(0)
}

/// The mock's own blit resources, for [`blit_common`].
struct BlitState {
    shaders: [Shader; 6],
    linear: Sampler,
    nearest: Sampler,
    cache: BlitPipelineCache,
}

struct MockDriver {
    name: &'static str,
    blit: Mutex<Option<BlitState>>,
}

fn prepare(index: usize) -> bool {
    let ok = config().prepare[index];
    log(format!("prepare {} {ok}", ["mock_a", "mock_b"][index]));
    ok
}

fn create(
    name: &'static str,
    debug_mode: bool,
    prefer_low_power: bool,
    props: &Properties,
) -> Result<BackendDevice> {
    let spirv = props
        .get_bool(PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN)
        .unwrap_or(false);
    let dxil = props
        .get_bool(PROP_GPU_DEVICE_CREATE_SHADERS_DXIL_BOOLEAN)
        .unwrap_or(false);
    log(format!(
        "create {name} debug={debug_mode} lowpower={prefer_low_power} spirv={spirv} dxil={dxil}"
    ));
    Ok(BackendDevice {
        driver: Box::new(MockDriver {
            name,
            blit: Mutex::new(None),
        }),
        shader_formats: config().shader_formats,
    })
}

static MOCK_A: GpuBootstrap = GpuBootstrap {
    name: "mock_a",
    prepare_driver: |_: &dyn VideoDriver, _: &Properties| prepare(0),
    create_device: |d, l, p| create("mock_a", d, l, p),
};

static MOCK_B: GpuBootstrap = GpuBootstrap {
    name: "mock_b",
    prepare_driver: |_: &dyn VideoDriver, _: &Properties| prepare(1),
    create_device: |d, l, p| create("mock_b", d, l, p),
};

fn names(textures: &[&Texture]) -> String {
    textures
        .iter()
        .map(|t| tex(t))
        .collect::<Vec<_>>()
        .join(",")
}

fn buffer_names(buffers: &[&Buffer]) -> String {
    buffers
        .iter()
        .map(|b| name(&b.raw))
        .collect::<Vec<_>>()
        .join(",")
}

fn sampler_names(bindings: &[TextureSamplerBinding<'_>]) -> String {
    bindings
        .iter()
        .map(|b| format!("{}+{}", tex(b.texture), name(&b.sampler.raw)))
        .collect::<Vec<_>>()
        .join(",")
}

impl GpuDriver for MockDriver {
    fn destroy(&mut self) {
        log(format!("destroy {}", self.name));
        // The backend releases its own objects.
        *self.blit.lock().unwrap() = None;
    }

    fn properties(&self) -> Properties {
        let props = Properties::new();
        props.set(PROP_GPU_DEVICE_NAME_STRING, "Mock GPU").unwrap();
        props
    }

    fn create_compute_pipeline(
        &self,
        c: &ComputePipelineCreateInfo<'_>,
    ) -> Result<(BackendObject, ComputePipelineHeader)> {
        let raw = object("compute", 0);
        log(format!("create_compute_pipeline {}", name(&raw)));
        Ok((
            raw,
            ComputePipelineHeader {
                num_samplers: c.num_samplers,
                num_readonly_storage_textures: c.num_readonly_storage_textures,
                num_readonly_storage_buffers: c.num_readonly_storage_buffers,
                num_readwrite_storage_textures: c.num_readwrite_storage_textures,
                num_readwrite_storage_buffers: c.num_readwrite_storage_buffers,
                num_uniform_buffers: c.num_uniform_buffers,
            },
        ))
    }

    fn create_graphics_pipeline(
        &self,
        c: &GraphicsPipelineCreateInfo<'_>,
    ) -> Result<(BackendObject, GraphicsPipelineHeader)> {
        let raw = object("pipeline", 0);
        let formats: Vec<String> = c
            .target_info
            .color_target_descriptions
            .iter()
            .map(|d| format!("{:?}", d.format))
            .collect();
        log(format!(
            "create_graphics_pipeline {} vs={} fs={} targets=[{}] depth_clip={}",
            name(&raw),
            name(&c.vertex_shader.raw),
            name(&c.fragment_shader.raw),
            formats.join(","),
            c.rasterizer_state.enable_depth_clip
        ));
        // (The counts the shaders' creation infos would give.)
        Ok((
            raw,
            GraphicsPipelineHeader {
                num_fragment_samplers: 1,
                ..Default::default()
            },
        ))
    }

    fn create_sampler(&self, _: &SamplerCreateInfo) -> Result<BackendObject> {
        let raw = object("sampler", 0);
        log(format!("create_sampler {}", name(&raw)));
        Ok(raw)
    }

    fn create_shader(&self, c: &ShaderCreateInfo<'_>) -> Result<BackendObject> {
        let raw = object("shader", 0);
        log(format!(
            "create_shader {} {:?} {}",
            name(&raw),
            c.stage,
            c.entrypoint
        ));
        Ok(raw)
    }

    fn create_texture(&self, c: &TextureCreateInfo) -> Result<BackendObject> {
        let raw = object("texture", 0);
        log(format!(
            "create_texture {} {:?} {}x{}",
            name(&raw),
            c.format,
            c.width,
            c.height
        ));
        Ok(raw)
    }

    fn create_buffer(
        &self,
        usage: BufferUsageFlags,
        size: u32,
        debug_name: Option<&str>,
    ) -> Result<BackendObject> {
        let raw = object("buffer", 0);
        log(format!(
            "create_buffer {} {usage:?} {size} {debug_name:?}",
            name(&raw)
        ));
        Ok(raw)
    }

    fn create_transfer_buffer(
        &self,
        usage: TransferBufferUsage,
        size: u32,
        debug_name: Option<&str>,
    ) -> Result<BackendObject> {
        let raw = object("transfer", size as usize);
        log(format!(
            "create_transfer_buffer {} {usage:?} {size} {debug_name:?}",
            name(&raw)
        ));
        Ok(raw)
    }

    fn set_buffer_name(&self, buffer: &BackendObject, text: &str) {
        log(format!("set_buffer_name {} {text}", name(buffer)));
    }

    fn set_texture_name(&self, texture: &BackendObject, text: &str) {
        log(format!("set_texture_name {} {text}", name(texture)));
    }

    fn insert_debug_label(&self, c: &mut BackendCommandBuffer, text: &str) {
        log(format!("insert_debug_label cmd{} {text}", cmd(c)));
    }

    fn push_debug_group(&self, c: &mut BackendCommandBuffer, text: &str) {
        log(format!("push_debug_group cmd{} {text}", cmd(c)));
    }

    fn pop_debug_group(&self, c: &mut BackendCommandBuffer) {
        log(format!("pop_debug_group cmd{}", cmd(c)));
    }

    fn release_texture(&self, raw: &BackendObject) {
        log(format!("release_texture {}", name(raw)));
    }

    fn release_sampler(&self, raw: &BackendObject) {
        log(format!("release_sampler {}", name(raw)));
    }

    fn release_buffer(&self, raw: &BackendObject) {
        log(format!("release_buffer {}", name(raw)));
    }

    fn release_transfer_buffer(&self, raw: &BackendObject) {
        log(format!("release_transfer_buffer {}", name(raw)));
    }

    fn release_shader(&self, raw: &BackendObject) {
        log(format!("release_shader {}", name(raw)));
    }

    fn release_compute_pipeline(&self, raw: &BackendObject) {
        log(format!("release_compute_pipeline {}", name(raw)));
    }

    fn release_graphics_pipeline(&self, raw: &BackendObject) {
        log(format!("release_graphics_pipeline {}", name(raw)));
    }

    fn begin_render_pass(
        &self,
        c: &mut BackendCommandBuffer,
        color_target_infos: &[ColorTargetInfo<'_>],
        depth: Option<&DepthStencilTargetInfo<'_>>,
    ) {
        let targets: Vec<&Texture> = color_target_infos.iter().map(|t| t.texture).collect();
        log(format!(
            "begin_render_pass cmd{} [{}] depth={}",
            cmd(c),
            names(&targets),
            depth.map(|d| tex(d.texture)).unwrap_or_default()
        ));
    }

    fn bind_graphics_pipeline(&self, c: &mut BackendCommandBuffer, raw: &BackendObject) {
        log(format!(
            "bind_graphics_pipeline cmd{} {}",
            cmd(c),
            name(raw)
        ));
    }

    fn set_viewport(&self, _: &mut BackendCommandBuffer, v: &Viewport) {
        log(format!(
            "set_viewport {} {} {} {} {} {}",
            v.x, v.y, v.w, v.h, v.min_depth, v.max_depth
        ));
    }

    fn set_scissor(&self, _: &mut BackendCommandBuffer, r: &Rect) {
        log(format!("set_scissor {} {} {} {}", r.x, r.y, r.w, r.h));
    }

    fn set_blend_constants(&self, _: &mut BackendCommandBuffer, c: FColor) {
        log(format!(
            "set_blend_constants {} {} {} {}",
            c.r, c.g, c.b, c.a
        ));
    }

    fn set_stencil_reference(&self, _: &mut BackendCommandBuffer, reference: u8) {
        log(format!("set_stencil_reference {reference}"));
    }

    fn bind_vertex_buffers(
        &self,
        _: &mut BackendCommandBuffer,
        first_slot: u32,
        bindings: &[BufferBinding<'_>],
    ) {
        let b: Vec<String> = bindings
            .iter()
            .map(|b| format!("{}@{}", name(&b.buffer.raw), b.offset))
            .collect();
        log(format!(
            "bind_vertex_buffers {first_slot} [{}]",
            b.join(",")
        ));
    }

    fn bind_index_buffer(
        &self,
        _: &mut BackendCommandBuffer,
        binding: &BufferBinding<'_>,
        size: IndexElementSize,
    ) {
        log(format!(
            "bind_index_buffer {}@{} {size:?} ({} bytes)",
            name(&binding.buffer.raw),
            binding.offset,
            super::sysgpu::index_size(size)
        ));
    }

    fn bind_vertex_samplers(
        &self,
        _: &mut BackendCommandBuffer,
        first_slot: u32,
        b: &[TextureSamplerBinding<'_>],
    ) {
        log(format!(
            "bind_vertex_samplers {first_slot} [{}]",
            sampler_names(b)
        ));
    }

    fn bind_vertex_storage_textures(
        &self,
        _: &mut BackendCommandBuffer,
        first_slot: u32,
        t: &[&Texture],
    ) {
        log(format!(
            "bind_vertex_storage_textures {first_slot} [{}]",
            names(t)
        ));
    }

    fn bind_vertex_storage_buffers(
        &self,
        _: &mut BackendCommandBuffer,
        first_slot: u32,
        b: &[&Buffer],
    ) {
        log(format!(
            "bind_vertex_storage_buffers {first_slot} [{}]",
            buffer_names(b)
        ));
    }

    fn bind_fragment_samplers(
        &self,
        _: &mut BackendCommandBuffer,
        first_slot: u32,
        b: &[TextureSamplerBinding<'_>],
    ) {
        log(format!(
            "bind_fragment_samplers {first_slot} [{}]",
            sampler_names(b)
        ));
    }

    fn bind_fragment_storage_textures(
        &self,
        _: &mut BackendCommandBuffer,
        first_slot: u32,
        t: &[&Texture],
    ) {
        log(format!(
            "bind_fragment_storage_textures {first_slot} [{}]",
            names(t)
        ));
    }

    fn bind_fragment_storage_buffers(
        &self,
        _: &mut BackendCommandBuffer,
        first_slot: u32,
        b: &[&Buffer],
    ) {
        log(format!(
            "bind_fragment_storage_buffers {first_slot} [{}]",
            buffer_names(b)
        ));
    }

    fn push_vertex_uniform_data(&self, _: &mut BackendCommandBuffer, slot: u32, data: &[u8]) {
        log(format!("push_vertex_uniform_data {slot} {data:?}"));
    }

    fn push_fragment_uniform_data(&self, _: &mut BackendCommandBuffer, slot: u32, data: &[u8]) {
        log(format!("push_fragment_uniform_data {slot} {data:?}"));
    }

    fn draw_indexed_primitives(
        &self,
        _: &mut BackendCommandBuffer,
        num_indices: u32,
        num_instances: u32,
        first_index: u32,
        vertex_offset: i32,
        first_instance: u32,
    ) {
        log(format!(
            "draw_indexed_primitives {num_indices} {num_instances} {first_index} {vertex_offset} {first_instance}"
        ));
    }

    fn draw_primitives(
        &self,
        _: &mut BackendCommandBuffer,
        num_vertices: u32,
        num_instances: u32,
        first_vertex: u32,
        first_instance: u32,
    ) {
        log(format!(
            "draw_primitives {num_vertices} {num_instances} {first_vertex} {first_instance}"
        ));
    }

    fn draw_primitives_indirect(
        &self,
        _: &mut BackendCommandBuffer,
        buffer: &BackendObject,
        offset: u32,
        draw_count: u32,
    ) {
        log(format!(
            "draw_primitives_indirect {} {offset} {draw_count}",
            name(buffer)
        ));
    }

    fn draw_indexed_primitives_indirect(
        &self,
        _: &mut BackendCommandBuffer,
        buffer: &BackendObject,
        offset: u32,
        draw_count: u32,
    ) {
        log(format!(
            "draw_indexed_primitives_indirect {} {offset} {draw_count}",
            name(buffer)
        ));
    }

    fn end_render_pass(&self, c: &mut BackendCommandBuffer) {
        log(format!("end_render_pass cmd{}", cmd(c)));
    }

    fn begin_compute_pass(
        &self,
        c: &mut BackendCommandBuffer,
        t: &[StorageTextureReadWriteBinding<'_>],
        b: &[StorageBufferReadWriteBinding<'_>],
    ) {
        let t: Vec<&Texture> = t.iter().map(|t| t.texture).collect();
        let b: Vec<&Buffer> = b.iter().map(|b| b.buffer).collect();
        log(format!(
            "begin_compute_pass cmd{} [{}] [{}]",
            cmd(c),
            names(&t),
            buffer_names(&b)
        ));
    }

    fn bind_compute_pipeline(&self, _: &mut BackendCommandBuffer, raw: &BackendObject) {
        log(format!("bind_compute_pipeline {}", name(raw)));
    }

    fn bind_compute_samplers(
        &self,
        _: &mut BackendCommandBuffer,
        first_slot: u32,
        b: &[TextureSamplerBinding<'_>],
    ) {
        log(format!(
            "bind_compute_samplers {first_slot} [{}]",
            sampler_names(b)
        ));
    }

    fn bind_compute_storage_textures(
        &self,
        _: &mut BackendCommandBuffer,
        first_slot: u32,
        t: &[&Texture],
    ) {
        log(format!(
            "bind_compute_storage_textures {first_slot} [{}]",
            names(t)
        ));
    }

    fn bind_compute_storage_buffers(
        &self,
        _: &mut BackendCommandBuffer,
        first_slot: u32,
        b: &[&Buffer],
    ) {
        log(format!(
            "bind_compute_storage_buffers {first_slot} [{}]",
            buffer_names(b)
        ));
    }

    fn push_compute_uniform_data(&self, _: &mut BackendCommandBuffer, slot: u32, data: &[u8]) {
        log(format!("push_compute_uniform_data {slot} {data:?}"));
    }

    fn dispatch_compute(&self, _: &mut BackendCommandBuffer, x: u32, y: u32, z: u32) {
        log(format!("dispatch_compute {x} {y} {z}"));
    }

    fn dispatch_compute_indirect(
        &self,
        _: &mut BackendCommandBuffer,
        buffer: &BackendObject,
        offset: u32,
    ) {
        log(format!(
            "dispatch_compute_indirect {} {offset}",
            name(buffer)
        ));
    }

    fn end_compute_pass(&self, c: &mut BackendCommandBuffer) {
        log(format!("end_compute_pass cmd{}", cmd(c)));
    }

    fn map_transfer_buffer(&self, raw: &BackendObject, cycle: bool) -> Result<NonNull<u8>> {
        log(format!("map_transfer_buffer {} {cycle}", name(raw)));
        let object = raw.downcast_ref::<MockObject>().unwrap();
        let mut data = object.data.lock().unwrap();
        // (The memory stays put: the vector is never resized.)
        Ok(NonNull::new(data.as_mut_ptr()).unwrap())
    }

    fn unmap_transfer_buffer(&self, raw: &BackendObject) {
        log(format!("unmap_transfer_buffer {}", name(raw)));
    }

    fn begin_copy_pass(&self, c: &mut BackendCommandBuffer) {
        log(format!("begin_copy_pass cmd{}", cmd(c)));
    }

    fn upload_to_texture(
        &self,
        _: &mut BackendCommandBuffer,
        s: &TextureTransferInfo<'_>,
        d: &TextureRegion<'_>,
        cycle: bool,
    ) {
        log(format!(
            "upload_to_texture {}@{} -> {} {}x{}x{} {cycle}",
            name(&s.transfer_buffer.raw),
            s.offset,
            tex(d.texture),
            d.w,
            d.h,
            d.d
        ));
    }

    fn upload_to_buffer(
        &self,
        _: &mut BackendCommandBuffer,
        s: &TransferBufferLocation<'_>,
        d: &BufferRegion<'_>,
        cycle: bool,
    ) {
        log(format!(
            "upload_to_buffer {}@{} -> {}@{}+{} {cycle}",
            name(&s.transfer_buffer.raw),
            s.offset,
            name(&d.buffer.raw),
            d.offset,
            d.size
        ));
    }

    fn copy_texture_to_texture(
        &self,
        _: &mut BackendCommandBuffer,
        s: &TextureLocation<'_>,
        d: &TextureLocation<'_>,
        w: u32,
        h: u32,
        depth: u32,
        cycle: bool,
    ) {
        log(format!(
            "copy_texture_to_texture {} -> {} {w}x{h}x{depth} {cycle}",
            tex(s.texture),
            tex(d.texture)
        ));
    }

    fn copy_buffer_to_buffer(
        &self,
        _: &mut BackendCommandBuffer,
        s: &BufferLocation<'_>,
        d: &BufferLocation<'_>,
        size: u32,
        cycle: bool,
    ) {
        log(format!(
            "copy_buffer_to_buffer {} -> {} {size} {cycle}",
            name(&s.buffer.raw),
            name(&d.buffer.raw)
        ));
    }

    fn generate_mipmaps(&self, c: &mut CommandBuffer, texture: &Texture) {
        let id = c.backend_mut::<MockCmd>().map(|c| c.id).unwrap_or(0);
        log(format!(
            "generate_mipmaps cmd{id} {} ignore_validation={}",
            tex(texture),
            c.header.ignore_render_pass_texture_validation
        ));
    }

    fn download_from_texture(
        &self,
        _: &mut BackendCommandBuffer,
        s: &TextureRegion<'_>,
        d: &TextureTransferInfo<'_>,
    ) {
        log(format!(
            "download_from_texture {} -> {}@{}",
            tex(s.texture),
            name(&d.transfer_buffer.raw),
            d.offset
        ));
    }

    fn download_from_buffer(
        &self,
        _: &mut BackendCommandBuffer,
        s: &BufferRegion<'_>,
        d: &TransferBufferLocation<'_>,
    ) {
        log(format!(
            "download_from_buffer {} -> {}@{}",
            name(&s.buffer.raw),
            name(&d.transfer_buffer.raw),
            d.offset
        ));
    }

    fn end_copy_pass(&self, c: &mut BackendCommandBuffer) {
        log(format!("end_copy_pass cmd{}", cmd(c)));
    }

    fn blit(&self, c: &mut CommandBuffer, info: &BlitInfo<'_>) {
        log(format!(
            "blit {} -> {}",
            tex(info.source.texture),
            tex(info.destination.texture)
        ));
        let mut state = self.blit.lock().unwrap();
        let state = state.get_or_insert_with(|| BlitState {
            shaders: std::array::from_fn(|_| Shader::from_backend(object("blitshader", 0))),
            linear: Sampler::from_backend(object("linear", 0)),
            nearest: Sampler::from_backend(object("nearest", 0)),
            cache: BlitPipelineCache::PerFormat(Vec::new()),
        });
        let [vertex, from_2d, from_2d_array, from_3d, from_cube, from_cube_array] = &state.shaders;
        let shaders = BlitShaders {
            vertex,
            from_2d,
            from_2d_array,
            from_3d,
            from_cube,
            from_cube_array,
        };
        if let Err(e) = blit_common(
            c,
            info,
            &state.linear,
            &state.nearest,
            &shaders,
            &mut state.cache,
        ) {
            log(format!("blit failed: {e}"));
        }
    }

    fn supports_swapchain_composition(&self, _: Window, c: SwapchainComposition) -> bool {
        log(format!("supports_swapchain_composition {c:?}"));
        c == SwapchainComposition::Sdr
    }

    fn supports_present_mode(&self, _: Window, mode: PresentMode) -> bool {
        log(format!("supports_present_mode {mode:?}"));
        mode != PresentMode::Mailbox
    }

    fn claim_window(&self, _: Window) -> Result<()> {
        log("claim_window".into());
        Ok(())
    }

    fn release_window(&self, _: Window) {
        log("release_window".into());
    }

    fn set_swapchain_parameters(
        &self,
        _: Window,
        c: SwapchainComposition,
        mode: PresentMode,
    ) -> Result<()> {
        log(format!("set_swapchain_parameters {c:?} {mode:?}"));
        Ok(())
    }

    fn set_allowed_frames_in_flight(&self, n: u32) -> Result<()> {
        log(format!("set_allowed_frames_in_flight {n}"));
        Ok(())
    }

    fn swapchain_texture_format(&self, _: Window) -> Result<TextureFormat> {
        log("swapchain_texture_format".into());
        Ok(TextureFormat::B8G8R8A8_UNORM)
    }

    fn acquire_command_buffer(&self) -> Result<Box<BackendCommandBuffer>> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        log(format!("acquire_command_buffer cmd{id}"));
        Ok(Box::new(MockCmd { id }))
    }

    fn acquire_swapchain_texture(
        &self,
        c: &mut BackendCommandBuffer,
        _: Window,
    ) -> Result<Option<BackendSwapchainTexture>> {
        log(format!("acquire_swapchain_texture cmd{}", cmd(c)));
        Ok(self.swapchain())
    }

    fn wait_for_swapchain(&self, _: Window) -> Result<()> {
        log("wait_for_swapchain".into());
        Ok(())
    }

    fn wait_and_acquire_swapchain_texture(
        &self,
        c: &mut BackendCommandBuffer,
        _: Window,
    ) -> Result<Option<BackendSwapchainTexture>> {
        log(format!("wait_and_acquire_swapchain_texture cmd{}", cmd(c)));
        Ok(self.swapchain())
    }

    fn submit(&self, c: Box<BackendCommandBuffer>) -> Result<()> {
        log(format!("submit cmd{}", cmd(&*c)));
        Ok(())
    }

    fn submit_and_acquire_fence(&self, c: Box<BackendCommandBuffer>) -> Result<BackendObject> {
        let fence = object("fence", 0);
        log(format!(
            "submit_and_acquire_fence cmd{} {}",
            cmd(&*c),
            name(&fence)
        ));
        Ok(fence)
    }

    fn cancel(&self, c: Box<BackendCommandBuffer>) -> Result<()> {
        log(format!("cancel cmd{}", cmd(&*c)));
        Ok(())
    }

    fn wait(&self) -> Result<()> {
        log("wait".into());
        Ok(())
    }

    fn wait_for_fences(&self, wait_all: bool, fences: &[&BackendObject]) -> Result<()> {
        let f: Vec<String> = fences.iter().map(|f| name(f)).collect();
        log(format!("wait_for_fences {wait_all} [{}]", f.join(",")));
        Ok(())
    }

    fn query_fence(&self, fence: &BackendObject) -> bool {
        log(format!("query_fence {}", name(fence)));
        true
    }

    fn release_fence(&self, fence: &BackendObject) {
        log(format!("release_fence {}", name(fence)));
    }

    fn supports_texture_format(
        &self,
        format: TextureFormat,
        texture_type: TextureType,
        usage: TextureUsageFlags,
    ) -> bool {
        log(format!(
            "supports_texture_format {format:?} {texture_type:?} {usage:?}"
        ));
        !config().unsupported.contains(&format)
    }

    fn supports_sample_count(&self, format: TextureFormat, count: SampleCount) -> bool {
        log(format!("supports_sample_count {format:?} {count:?}"));
        count != SampleCount::Eight
    }
}

impl MockDriver {
    fn swapchain(&self) -> Option<BackendSwapchainTexture> {
        if !config().swapchain {
            return None;
        }
        Some(BackendSwapchainTexture {
            raw: object("swapchain", 0),
            info: TextureCreateInfo {
                texture_type: TextureType::Texture2D,
                format: TextureFormat::B8G8R8A8_UNORM,
                usage: TextureUsageFlags::COLOR_TARGET,
                width: 64,
                height: 48,
                layer_count_or_depth: 1,
                num_levels: 1,
                sample_count: SampleCount::One,
                props: None,
            },
            width: 64,
            height: 48,
        })
    }
}

// Fixtures

/// The test lock, the dummy video driver, the mock backends in the driver
/// list (unless `mocks` is false) and a fresh mock configuration; undone
/// when dropped.
struct Fixture {
    _lock: MutexGuard<'static, ()>,
}

impl Fixture {
    fn new(mocks: bool) -> Fixture {
        let lock = crate::test_support::test_lock();
        hints::set(hints::VIDEO_DRIVER, "dummy").unwrap();
        init::init(InitFlags::VIDEO).unwrap();
        *TEST_BACKENDS.lock().unwrap() = mocks.then(|| vec![&MOCK_A, &MOCK_B]);
        *config() = MockConfig {
            prepare: [true, true],
            shader_formats: ShaderFormat::SPIRV,
            unsupported: Vec::new(),
            swapchain: true,
        };
        take_log();
        Fixture { _lock: lock }
    }

    /// A debug-mode device on the first mock.
    fn device(&self) -> Device {
        let device = Device::new(ShaderFormat::SPIRV, true, None).unwrap();
        take_log();
        device
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        *TEST_BACKENDS.lock().unwrap() = None;
        init::quit();
        hints::reset(hints::VIDEO_DRIVER);
        hints::reset(hints::GPU_DRIVER);
        take_log();
    }
}

fn texture_info(format: TextureFormat, usage: TextureUsageFlags) -> TextureCreateInfo {
    TextureCreateInfo {
        texture_type: TextureType::Texture2D,
        format,
        usage,
        width: 64,
        height: 32,
        layer_count_or_depth: 1,
        num_levels: 1,
        sample_count: SampleCount::One,
        props: None,
    }
}

fn color_texture(device: &Device) -> Texture {
    device
        .create_texture(&texture_info(
            TextureFormat::R8G8B8A8_UNORM,
            TextureUsageFlags::COLOR_TARGET | TextureUsageFlags::SAMPLER,
        ))
        .unwrap()
}

fn shader(device: &Device, stage: ShaderStage) -> Shader {
    device
        .create_shader(&ShaderCreateInfo {
            code: &[0; 4],
            entrypoint: "main",
            format: ShaderFormat::SPIRV,
            stage,
            ..Default::default()
        })
        .unwrap()
}

fn err_message<T: std::fmt::Debug>(r: Result<T>) -> String {
    r.unwrap_err().message().to_string()
}

// The format tables

#[test]
fn format_tables_match_c() {
    let _l = crate::test_support::test_lock();
    let asserts = Asserts::catch();
    let expected = include_str!("testdata/formats.txt");
    let mut checked = 0;
    for line in expected.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = line.split(' ').collect();
        let num = |i: usize| f[i].parse::<i64>().unwrap();
        match f[0] {
            "fmt" => {
                let format = TextureFormat(num(1) as u32);
                let got = format!(
                    "fmt {} {} {} {} {} {} {} {} {} {} 0x{:08x} {}",
                    format.0,
                    format.block_width(),
                    format.block_height(),
                    format.texel_block_size(),
                    format.is_depth_format() as i32,
                    format.is_stencil_format() as i32,
                    format.is_integer_format() as i32,
                    format.is_compressed_format() as i32,
                    format.has_alpha() as i32,
                    format.is_compute_writable() as i32,
                    format.pixel_format().map_or(0, |p| p.0),
                    asserts.take().len()
                );
                assert_eq!(got, line);
            }
            "size" => {
                let format = TextureFormat(num(1) as u32);
                let size = format.calculate_size(num(2) as u32, num(3) as u32, num(4) as u32);
                let got = format!(
                    "size {} {} {} {} {size} {}",
                    f[1],
                    f[2],
                    f[3],
                    f[4],
                    asserts.take().len()
                );
                assert_eq!(got, line);
            }
            "bpr" => {
                let format = TextureFormat(num(1) as u32);
                let bpr = format.bytes_per_row(num(2) as i32);
                let got = format!("bpr {} {} {bpr} {}", f[1], f[2], asserts.take().len());
                assert_eq!(got, line);
            }
            "pix" => {
                let pixel = PixelFormat(u32::from_str_radix(&f[1][2..], 16).unwrap());
                let got = TextureFormat::from_pixel_format(pixel).map_or(0, |t| t.0);
                assert_eq!(got as i64, num(2), "{line}");
            }
            other => panic!("unknown line {other}"),
        }
        checked += 1;
    }
    assert_eq!(checked, 1860);

    // The unrecognized format asserts with upstream's message.
    assert_eq!(TextureFormat::INVALID.texel_block_size(), 0);
    assert_eq!(asserts.take(), ["Unrecognized TextureFormat!"]);
}

#[test]
fn texture_format_names_and_defaults() {
    assert_eq!(
        format!("{:?}", TextureFormat::ASTC_10x8_UNORM_SRGB),
        "TextureFormat::ASTC_10x8_UNORM_SRGB"
    );
    assert_eq!(format!("{:?}", TextureFormat(500)), "TextureFormat(500)");
    assert_eq!(TextureFormat::default(), TextureFormat::INVALID);
    assert_eq!(super::sysgpu::TEXTUREFORMAT_MAX_ENUM_VALUE, 105);
    assert_eq!(
        format!(
            "{:?}",
            TextureUsageFlags::SAMPLER | TextureUsageFlags(0x100)
        ),
        "TextureUsageFlags(SAMPLER | 0x100)"
    );
    assert_eq!(format!("{:?}", ShaderFormat::INVALID), "ShaderFormat(0x0)");
    // The C enums' values, which the backends' tables are indexed by.
    assert_eq!(VertexElementFormat::Half4 as u32, 30);
    assert_eq!(CompareOp::Always as u32, 8);
    assert_eq!(StencilOp::DecrementAndWrap as u32, 8);
    assert_eq!(BlendOp::Max as u32, 5);
    assert_eq!(BlendFactor::SrcAlphaSaturate as u32, 13);
    assert_eq!(SwapchainComposition::Hdr10St2084 as u32, 3);
    assert_eq!(PresentMode::Mailbox as u32, 2);
    assert_eq!(SampleCount::Eight as u32, 3);
    assert_eq!(TextureType::CubeArray as u32, 4);
}

// Driver selection

#[test]
fn handles_drop_their_backend_object_before_the_device() {
    let fixture = Fixture::new(true);
    let device = fixture.device();
    let buffer = device
        .create_buffer(&BufferCreateInfo {
            usage: BufferUsageFlags::VERTEX,
            size: 16,
            props: None,
        })
        .unwrap();
    let raw = name(&buffer.raw);
    take_log();

    // The buffer holds the last reference to the device: its backend object
    // goes before the device is destroyed (which, on Direct3D 12, unloads
    // the driver the object's vtable is in).
    LOG_DROPS.store(true, Ordering::Relaxed);
    drop(device);
    drop(buffer);
    LOG_DROPS.store(false, Ordering::Relaxed);
    assert_eq!(
        take_log(),
        [
            format!("release_buffer {raw}"),
            format!("drop {raw}"),
            "destroy mock_a".to_string()
        ]
    );
}

#[test]
fn no_backend_fails_cleanly() {
    let _l = crate::test_support::test_lock();
    *TEST_BACKENDS.lock().unwrap() = None;
    // (Direct3D 12, on Windows, and Vulkan are the backends translated;
    // neither takes SPIR-V on the dummy video driver, which has no Vulkan
    // surfaces, below.)
    let expected: &[&str] = if cfg!(windows) {
        &["direct3d12", "vulkan"]
    } else {
        &["vulkan"]
    };
    assert_eq!(num_gpu_drivers(), expected.len());
    for (i, name) in expected.iter().enumerate() {
        assert_eq!(gpu_driver(i).unwrap(), *name);
    }
    let e = gpu_driver(expected.len()).unwrap_err();
    assert_eq!(e.kind(), crate::ErrorKind::InvalidParam);
    assert_eq!(e.message(), "Parameter 'index' is invalid");

    // Without video
    assert_eq!(
        err_message(Device::new(ShaderFormat::SPIRV, true, None)),
        "Video subsystem not initialized"
    );
    assert!(!supports_shader_formats(ShaderFormat::SPIRV, None));
    drop(_l);

    let _f = Fixture::new(false);
    assert_eq!(
        err_message(Device::new(ShaderFormat::SPIRV, true, None)),
        "No supported SDL_GPU backend found!"
    );
    assert_eq!(
        err_message(Device::new(ShaderFormat::SPIRV, true, Some("vulkan"))),
        "SDL_HINT_GPU_DRIVER vulkan unsupported!"
    );
    hints::set(hints::GPU_DRIVER, "metal").unwrap();
    assert_eq!(
        err_message(Device::new(ShaderFormat::SPIRV, true, Some("vulkan"))),
        "SDL_HINT_GPU_DRIVER metal unsupported!"
    );
    hints::reset(hints::GPU_DRIVER);
    let props = Properties::new();
    props
        .set(PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN, true)
        .unwrap();
    assert_eq!(
        err_message(Device::with_properties(&props)),
        "OpenXR is not enabled in this build of SDL"
    );
    assert!(!supports_properties(&props));
    assert!(!supports_shader_formats(ShaderFormat::SPIRV, None));
}

#[test]
fn driver_selection_by_hint_and_property() {
    let _f = Fixture::new(true);
    assert_eq!(num_gpu_drivers(), 2);
    assert_eq!(gpu_driver(0).unwrap(), "mock_a");
    assert_eq!(gpu_driver(1).unwrap(), "mock_b");
    assert!(gpu_driver(2).is_err());

    // The first backend that prepares.
    let device = Device::new(ShaderFormat::SPIRV | ShaderFormat::DXIL, true, None).unwrap();
    assert_eq!(device.driver(), "mock_a");
    assert_eq!(device.shader_formats(), ShaderFormat::SPIRV);
    assert_eq!(
        take_log(),
        [
            "prepare mock_a true",
            "create mock_a debug=true lowpower=false spirv=true dxil=true"
        ]
    );
    drop(device);
    assert_eq!(take_log(), ["destroy mock_a"]);

    config().prepare = [false, true];
    let device = Device::new(ShaderFormat::SPIRV, false, None).unwrap();
    assert_eq!(device.driver(), "mock_b");
    assert_eq!(
        take_log(),
        [
            "prepare mock_a false",
            "prepare mock_b true",
            "create mock_b debug=false lowpower=false spirv=true dxil=false"
        ]
    );
    drop(device);
    take_log();

    // By name (case-insensitively); a named backend that doesn't prepare
    // fails, without trying the others.
    let device = Device::new(ShaderFormat::SPIRV, true, Some("MOCK_B")).unwrap();
    assert_eq!(device.driver(), "mock_b");
    drop(device);
    take_log();
    assert_eq!(
        err_message(Device::new(ShaderFormat::SPIRV, true, Some("mock_a"))),
        "SDL_HINT_GPU_DRIVER mock_a unsupported!"
    );
    assert_eq!(take_log(), ["prepare mock_a false"]);

    // The hint overrides the property.
    config().prepare = [true, true];
    hints::set(hints::GPU_DRIVER, "mock_b").unwrap();
    let device = Device::new(ShaderFormat::SPIRV, true, Some("mock_a")).unwrap();
    assert_eq!(device.driver(), "mock_b");
    drop(device);
    hints::reset(hints::GPU_DRIVER);
    take_log();

    // The debug mode defaults to true, low power to false.
    let props = Properties::new();
    let device = Device::with_properties(&props).unwrap();
    assert!(device.shared.debug_mode);
    drop(device);
    props
        .set(PROP_GPU_DEVICE_CREATE_PREFERLOWPOWER_BOOLEAN, true)
        .unwrap();
    props
        .set(PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN, false)
        .unwrap();
    let device = Device::with_properties(&props).unwrap();
    assert!(!device.shared.debug_mode);
    assert!(take_log()
        .contains(&"create mock_a debug=false lowpower=true spirv=false dxil=false".to_string()));
    drop(device);

    assert!(supports_shader_formats(ShaderFormat::SPIRV, Some("mock_b")));
    assert!(!supports_shader_formats(
        ShaderFormat::SPIRV,
        Some("mock_c")
    ));
    assert!(supports_properties(&Properties::new()));

    // The device's properties come from the backend.
    let device = Device::new(ShaderFormat::SPIRV, true, None).unwrap();
    assert_eq!(
        device
            .properties()
            .get_string(PROP_GPU_DEVICE_NAME_STRING)
            .as_deref(),
        Some("Mock GPU")
    );
}

#[test]
fn feature_properties_set_validation() {
    let _f = Fixture::new(true);
    let asserts = Asserts::catch();
    let props = Properties::new();
    props
        .set(PROP_GPU_DEVICE_CREATE_FEATURE_DEPTH_CLAMPING_BOOLEAN, false)
        .unwrap();
    props
        .set(PROP_GPU_DEVICE_CREATE_FEATURE_ANISOTROPY_BOOLEAN, false)
        .unwrap();
    let device = Device::with_properties(&props).unwrap();
    assert!(device.shared.default_enable_depth_clip);
    assert_eq!(
        err_message(device.create_sampler(&SamplerCreateInfo {
            enable_anisotropy: true,
            ..Default::default()
        })),
        "enable_anisotropy must be set to false (FEATURE_ANISOTROPY disabled)"
    );
    let vs = shader(&device, ShaderStage::Vertex);
    let fs = shader(&device, ShaderStage::Fragment);
    let info = GraphicsPipelineCreateInfo {
        vertex_shader: &vs,
        fragment_shader: &fs,
        vertex_input_state: Default::default(),
        primitive_type: PrimitiveType::TriangleList,
        rasterizer_state: Default::default(),
        multisample_state: Default::default(),
        depth_stencil_state: Default::default(),
        target_info: Default::default(),
        props: None,
    };
    assert_eq!(
        err_message(device.create_graphics_pipeline(&info)),
        "Rasterizer state enable_depth_clip must be set to true (FEATURE_DEPTH_CLAMPING disabled)"
    );
    let info = GraphicsPipelineCreateInfo {
        rasterizer_state: RasterizerState {
            enable_depth_clip: true,
            ..Default::default()
        },
        ..info
    };
    assert!(device.create_graphics_pipeline(&info).is_ok());
    assert_eq!(
        asserts.take(),
        [
            "enable_anisotropy must be set to false (FEATURE_ANISOTROPY disabled)",
            "Rasterizer state enable_depth_clip must be set to true (FEATURE_DEPTH_CLAMPING disabled)"
        ]
    );

    // With the features on (the defaults), depth clip defaults off.
    let device = Device::new(ShaderFormat::SPIRV, true, None).unwrap();
    assert!(!device.shared.default_enable_depth_clip);
    assert!(device
        .create_sampler(&SamplerCreateInfo {
            enable_anisotropy: true,
            ..Default::default()
        })
        .is_ok());
}

// Resource creation and release

#[test]
fn resources_dispatch_and_release() {
    let f = Fixture::new(true);
    let device = f.device();

    let props = Properties::new();
    props
        .set(PROP_GPU_BUFFER_CREATE_NAME_STRING, "vertices")
        .unwrap();
    let buffer = device
        .create_buffer(&BufferCreateInfo {
            usage: BufferUsageFlags::VERTEX,
            size: 64,
            props: Some(props),
        })
        .unwrap();
    buffer.set_name("renamed");
    let texture = color_texture(&device);
    texture.set_name("target");
    let props = Properties::new();
    props
        .set(PROP_GPU_TRANSFERBUFFER_CREATE_NAME_STRING, "staging")
        .unwrap();
    let mut transfer = device
        .create_transfer_buffer(&TransferBufferCreateInfo {
            usage: TransferBufferUsage::Upload,
            size: 8,
            props: Some(props),
        })
        .unwrap();
    assert_eq!(transfer.size(), 8);
    {
        let mut map = transfer.map(true).unwrap();
        map.copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
    }
    {
        let map = transfer.map(false).unwrap();
        assert_eq!(&*map, &[1, 2, 3, 4, 5, 6, 7, 8]);
    }
    let sampler = device
        .create_sampler(&SamplerCreateInfo::default())
        .unwrap();
    let compute = device
        .create_compute_pipeline(&ComputePipelineCreateInfo {
            code: &[0; 4],
            entrypoint: "main",
            format: ShaderFormat::SPIRV,
            threadcount_x: 1,
            threadcount_y: 1,
            threadcount_z: 1,
            ..Default::default()
        })
        .unwrap();
    let vs = shader(&device, ShaderStage::Vertex);
    let fs = shader(&device, ShaderStage::Fragment);
    let log = take_log();
    let b = name(&buffer.raw);
    let t = tex(&texture);
    let tb = name(&transfer.raw);
    assert_eq!(
        log[0],
        format!("create_buffer {b} BufferUsageFlags(VERTEX) 64 Some(\"vertices\")")
    );
    assert_eq!(log[1], format!("set_buffer_name {b} renamed"));
    assert!(log[2].starts_with("supports_texture_format TextureFormat::R8G8B8A8_UNORM Texture2D"));
    assert_eq!(
        log[3],
        format!("create_texture {t} TextureFormat::R8G8B8A8_UNORM 64x32")
    );
    assert_eq!(log[4], format!("set_texture_name {t} target"));
    assert_eq!(
        log[5],
        format!("create_transfer_buffer {tb} Upload 8 Some(\"staging\")")
    );
    assert_eq!(
        log[6..10],
        [
            format!("map_transfer_buffer {tb} true"),
            format!("unmap_transfer_buffer {tb}"),
            format!("map_transfer_buffer {tb} false"),
            format!("unmap_transfer_buffer {tb}"),
        ]
    );
    assert!(log[10].starts_with("create_sampler"));
    assert!(log[11].starts_with("create_compute_pipeline"));
    assert!(log[12].ends_with("Vertex main"));
    assert!(log[13].ends_with("Fragment main"));

    let s = name(&sampler.raw);
    let c = name(&compute.raw);
    let (v, fr) = (name(&vs.raw), name(&fs.raw));
    drop((buffer, texture, transfer, sampler, compute, vs, fs));
    assert_eq!(
        take_log(),
        [
            format!("release_buffer {b}"),
            format!("release_texture {t}"),
            format!("release_transfer_buffer {tb}"),
            format!("release_sampler {s}"),
            format!("release_compute_pipeline {c}"),
            format!("release_shader {v}"),
            format!("release_shader {fr}"),
        ]
    );

    // The device is destroyed once the last resource is gone.
    let texture = color_texture(&device);
    drop(device);
    take_log();
    drop(texture);
    let log = take_log();
    assert_eq!(log.len(), 2);
    assert!(log[0].starts_with("release_texture"));
    assert_eq!(log[1], "destroy mock_a");
}

#[test]
fn shader_and_compute_pipeline_validation() {
    let f = Fixture::new(true);
    let device = f.device();
    let asserts = Asserts::catch();

    let base = ShaderCreateInfo {
        code: &[0; 4],
        entrypoint: "main",
        format: ShaderFormat::SPIRV,
        ..Default::default()
    };
    let cases: Vec<(ShaderCreateInfo<'_>, &str)> = vec![
        (
            ShaderCreateInfo {
                format: ShaderFormat::INVALID,
                ..base.clone()
            },
            "Shader format cannot be INVALID!",
        ),
        (
            ShaderCreateInfo {
                format: ShaderFormat::DXIL | ShaderFormat::MSL,
                ..base.clone()
            },
            "Incompatible shader format for GPU backend",
        ),
        (
            ShaderCreateInfo {
                num_samplers: 17,
                ..base.clone()
            },
            "Shader sampler count cannot be higher than 16!",
        ),
        (
            ShaderCreateInfo {
                num_storage_textures: 9,
                ..base.clone()
            },
            "Shader storage texture count cannot be higher than 8!",
        ),
        (
            ShaderCreateInfo {
                num_storage_buffers: 9,
                ..base.clone()
            },
            "Shader storage buffer count cannot be higher than 8!",
        ),
        (
            ShaderCreateInfo {
                num_uniform_buffers: 5,
                ..base.clone()
            },
            "Shader uniform buffer count cannot be higher than 4!",
        ),
    ];
    for (info, msg) in cases {
        assert_eq!(err_message(device.create_shader(&info)), msg);
        assert_eq!(asserts.take(), [msg]);
    }
    // At the limits it is created; with the format among others too.
    let ok = ShaderCreateInfo {
        format: ShaderFormat::SPIRV | ShaderFormat::DXIL,
        num_samplers: 16,
        num_storage_textures: 8,
        num_storage_buffers: 8,
        num_uniform_buffers: 4,
        ..base.clone()
    };
    assert!(device.create_shader(&ok).is_ok());
    assert!(asserts.take().is_empty());

    let base = ComputePipelineCreateInfo {
        code: &[0; 4],
        entrypoint: "main",
        format: ShaderFormat::SPIRV,
        threadcount_x: 1,
        threadcount_y: 1,
        threadcount_z: 1,
        ..Default::default()
    };
    let cases: Vec<(ComputePipelineCreateInfo<'_>, &str)> = vec![
        (
            ComputePipelineCreateInfo {
                format: ShaderFormat::INVALID,
                ..base.clone()
            },
            "Shader format cannot be INVALID!",
        ),
        (
            ComputePipelineCreateInfo {
                format: ShaderFormat::MSL,
                ..base.clone()
            },
            "Incompatible shader format for GPU backend",
        ),
        (
            ComputePipelineCreateInfo {
                num_readwrite_storage_textures: 9,
                ..base.clone()
            },
            "Compute pipeline write-only texture count cannot be higher than 8!",
        ),
        (
            ComputePipelineCreateInfo {
                num_readwrite_storage_buffers: 9,
                ..base.clone()
            },
            "Compute pipeline write-only buffer count cannot be higher than 8!",
        ),
        (
            ComputePipelineCreateInfo {
                num_samplers: 17,
                ..base.clone()
            },
            "Compute pipeline sampler count cannot be higher than 16!",
        ),
        (
            ComputePipelineCreateInfo {
                num_readonly_storage_textures: 9,
                ..base.clone()
            },
            "Compute pipeline readonly storage texture count cannot be higher than 8!",
        ),
        (
            ComputePipelineCreateInfo {
                num_readonly_storage_buffers: 9,
                ..base.clone()
            },
            "Compute pipeline readonly storage buffer count cannot be higher than 8!",
        ),
        (
            ComputePipelineCreateInfo {
                num_uniform_buffers: 5,
                ..base.clone()
            },
            "Compute pipeline uniform buffer count cannot be higher than 4!",
        ),
        (
            ComputePipelineCreateInfo {
                threadcount_y: 0,
                ..base.clone()
            },
            "Compute pipeline threadCount dimensions must be at least 1!",
        ),
    ];
    for (info, msg) in cases {
        assert_eq!(err_message(device.create_compute_pipeline(&info)), msg);
        assert_eq!(asserts.take(), [msg]);
    }
    take_log();

    // Without debug mode nothing is checked.
    let device = Device::new(ShaderFormat::SPIRV, false, None).unwrap();
    assert!(device
        .create_shader(&ShaderCreateInfo {
            format: ShaderFormat::INVALID,
            num_samplers: 100,
            ..Default::default()
        })
        .is_ok());
    assert!(device
        .create_compute_pipeline(&ComputePipelineCreateInfo::default())
        .is_ok());
    assert!(asserts.take().is_empty());
}

#[test]
fn graphics_pipeline_validation() {
    let f = Fixture::new(true);
    let device = f.device();
    let asserts = Asserts::catch();
    let vs = shader(&device, ShaderStage::Vertex);
    let fs = shader(&device, ShaderStage::Fragment);
    config().unsupported = vec![TextureFormat::R16_FLOAT, TextureFormat::D32_FLOAT];
    take_log();

    let color = |format| ColorTargetDescription {
        format,
        blend_state: Default::default(),
    };
    let check = |targets: &[ColorTargetDescription],
                 depth: Option<TextureFormat>,
                 vertex: VertexInputState<'_>,
                 multisample: MultisampleState|
     -> Result<GraphicsPipeline> {
        device.create_graphics_pipeline(&GraphicsPipelineCreateInfo {
            vertex_shader: &vs,
            fragment_shader: &fs,
            vertex_input_state: vertex,
            primitive_type: PrimitiveType::TriangleList,
            rasterizer_state: Default::default(),
            multisample_state: multisample,
            depth_stencil_state: Default::default(),
            target_info: GraphicsPipelineTargetInfo {
                color_target_descriptions: targets,
                depth_stencil_format: depth.unwrap_or_default(),
                has_depth_stencil_target: depth.is_some(),
            },
            props: None,
        })
    };
    let rgba = [color(TextureFormat::R8G8B8A8_UNORM)];
    let none = VertexInputState::default();
    let ms = MultisampleState::default();

    let pipeline = check(&rgba, Some(TextureFormat::D16_UNORM), none, ms).unwrap();
    let log = take_log();
    assert_eq!(
        log[..2],
        [
            "supports_texture_format TextureFormat::R8G8B8A8_UNORM Texture2D TextureUsageFlags(COLOR_TARGET)",
            "supports_texture_format TextureFormat::D16_UNORM Texture2D TextureUsageFlags(DEPTH_STENCIL_TARGET)",
        ]
    );
    assert!(log[2].starts_with(&format!("create_graphics_pipeline {}", name(&pipeline.raw))));
    drop(pipeline);
    assert_eq!(take_log().len(), 1);

    let attrs = |locations: &[u32]| -> Vec<VertexAttribute> {
        locations
            .iter()
            .map(|&location| VertexAttribute {
                location,
                buffer_slot: 0,
                format: VertexElementFormat::Float2,
                offset: 0,
            })
            .collect()
    };
    let dup = attrs(&[0, 1, 0]);
    let many_attrs = attrs(&(0..17).collect::<Vec<_>>());
    let ok_attrs = attrs(&(0..16).collect::<Vec<_>>());
    let many_buffers = vec![VertexBufferDescription::default(); 17];
    let step = [VertexBufferDescription {
        instance_step_rate: 1,
        ..Default::default()
    }];
    let rgb565 = [color(TextureFormat::B5G6R5_UNORM)];

    let expect = |result: Result<GraphicsPipeline>, msg: &str| {
        assert_eq!(err_message(result), msg);
        assert_eq!(asserts.take(), [msg]);
    };
    expect(
        check(&[color(TextureFormat::INVALID)], None, none, ms),
        "Invalid texture format enum!",
    );
    expect(
        check(&[color(TextureFormat(105))], None, none, ms),
        "Invalid texture format enum!",
    );
    expect(
        check(&[color(TextureFormat::D24_UNORM)], None, none, ms),
        "Color target formats cannot be a depth format!",
    );
    expect(
        check(&[color(TextureFormat::R16_FLOAT)], None, none, ms),
        "Format is not supported for color targets on this device!",
    );
    expect(
        check(&rgba, Some(TextureFormat::INVALID), none, ms),
        "Invalid texture format enum!",
    );
    expect(
        check(&rgba, Some(TextureFormat::R8_UNORM), none, ms),
        "Depth-stencil target format must be a depth format!",
    );
    expect(
        check(&rgba, Some(TextureFormat::D32_FLOAT), none, ms),
        "Format is not supported for depth targets on this device!",
    );
    expect(
        check(
            &[],
            None,
            none,
            MultisampleState {
                enable_alpha_to_coverage: true,
                ..ms
            },
        ),
        "Alpha-to-coverage enabled but no color targets present!",
    );
    expect(
        check(
            &rgb565,
            None,
            none,
            MultisampleState {
                enable_alpha_to_coverage: true,
                ..ms
            },
        ),
        "Format is not compatible with alpha-to-coverage!",
    );
    expect(
        check(
            &rgba,
            None,
            VertexInputState {
                vertex_buffer_descriptions: &many_buffers,
                vertex_attributes: &[],
            },
            ms,
        ),
        "The number of vertex buffer descriptions in a vertex input state must not exceed 16!",
    );
    expect(
        check(
            &rgba,
            None,
            VertexInputState {
                vertex_buffer_descriptions: &[],
                vertex_attributes: &many_attrs,
            },
            ms,
        ),
        "The number of vertex attributes in a vertex input state must not exceed 16!",
    );
    expect(
        check(
            &rgba,
            None,
            VertexInputState {
                vertex_buffer_descriptions: &step,
                vertex_attributes: &[],
            },
            ms,
        ),
        "For all vertex buffer descriptions, instance_step_rate must be 0!",
    );
    expect(
        check(
            &rgba,
            None,
            VertexInputState {
                vertex_buffer_descriptions: &[],
                vertex_attributes: &dup,
            },
            ms,
        ),
        "Each vertex attribute location in a vertex input state must be unique!",
    );
    expect(
        check(
            &rgba,
            None,
            none,
            MultisampleState {
                enable_mask: true,
                ..ms
            },
        ),
        "For multisample states, enable_mask must be false!",
    );
    expect(
        check(
            &rgba,
            None,
            none,
            MultisampleState {
                sample_mask: 1,
                ..ms
            },
        ),
        "For multisample states, sample_mask must be 0!",
    );
    take_log();
    assert!(check(
        &rgba,
        None,
        VertexInputState {
            vertex_buffer_descriptions: &[VertexBufferDescription::default(); 16],
            vertex_attributes: &ok_attrs,
        },
        MultisampleState {
            enable_alpha_to_coverage: true,
            ..ms
        },
    )
    .is_ok());
    assert!(asserts.take().is_empty());
}

#[test]
fn texture_validation() {
    let f = Fixture::new(true);
    let device = f.device();
    let asserts = Asserts::catch();
    let usage = TextureUsageFlags::SAMPLER;
    let base = texture_info(TextureFormat::R8G8B8A8_UNORM, usage);

    let cube = TextureCreateInfo {
        texture_type: TextureType::Cube,
        width: 32,
        height: 32,
        layer_count_or_depth: 6,
        ..base.clone()
    };
    let cube_array = TextureCreateInfo {
        texture_type: TextureType::CubeArray,
        layer_count_or_depth: 12,
        ..cube.clone()
    };
    let three_d = TextureCreateInfo {
        texture_type: TextureType::Texture3D,
        layer_count_or_depth: 4,
        ..base.clone()
    };
    let array = TextureCreateInfo {
        texture_type: TextureType::Texture2DArray,
        layer_count_or_depth: 4,
        ..base.clone()
    };
    let multi = SampleCount::Four;
    let cases: Vec<(TextureCreateInfo, &[&str])> = vec![
        (
            TextureCreateInfo {
                format: TextureFormat::INVALID,
                ..base.clone()
            },
            &["Invalid texture format enum!"],
        ),
        (
            TextureCreateInfo {
                width: 0,
                num_levels: 0,
                ..base.clone()
            },
            &[
                "For any texture: width, height, and layer_count_or_depth must be >= 1",
                "For any texture: num_levels must be >= 1",
            ],
        ),
        (
            TextureCreateInfo {
                layer_count_or_depth: 2,
                ..base.clone()
            },
            &["2D textures must have a layer count of 1"],
        ),
        (
            TextureCreateInfo {
                usage: usage | TextureUsageFlags::GRAPHICS_STORAGE_READ,
                ..base.clone()
            },
            &["For any texture: usage cannot contain both GRAPHICS_STORAGE_READ and SAMPLER"],
        ),
        (
            TextureCreateInfo {
                usage: TextureUsageFlags::COMPUTE_STORAGE_WRITE,
                sample_count: multi,
                ..base.clone()
            },
            &[
                "For multisample textures: usage cannot contain COMPUTE_STORAGE_WRITE flag",
            ],
        ),
        (
            TextureCreateInfo {
                format: TextureFormat::D16_UNORM,
                usage: usage | TextureUsageFlags::COLOR_TARGET,
                ..base.clone()
            },
            &["For depth textures: usage cannot contain any flags except for DEPTH_STENCIL_TARGET and SAMPLER"],
        ),
        (
            TextureCreateInfo {
                format: TextureFormat::R8_UINT,
                ..base.clone()
            },
            &["For any texture: usage cannot contain SAMPLER for textures with an integer format"],
        ),
        (
            TextureCreateInfo {
                width: 20000,
                height: 32,
                layer_count_or_depth: 5,
                sample_count: multi,
                ..cube.clone()
            },
            &[
                "For cube textures: width and height must be identical",
                "For cube textures: width and height must be <= 16384",
                "For cube textures: layer_count_or_depth must be 6",
                "For cube textures: sample_count must be SDL_GPU_SAMPLECOUNT_1",
            ],
        ),
        (
            TextureCreateInfo {
                format: TextureFormat::R16_FLOAT,
                ..cube.clone()
            },
            &["For cube textures: the format is unsupported for the given usage"],
        ),
        (
            TextureCreateInfo {
                width: 16,
                height: 20000,
                layer_count_or_depth: 7,
                sample_count: multi,
                format: TextureFormat::R16_FLOAT,
                ..cube_array.clone()
            },
            &[
                "For cube array textures: width and height must be identical",
                "For cube array textures: width and height must be <= 16384",
                "For cube array textures: layer_count_or_depth must be a multiple of 6",
                "For cube array textures: sample_count must be SDL_GPU_SAMPLECOUNT_1",
                "For cube array textures: the format is unsupported for the given usage",
            ],
        ),
        (
            TextureCreateInfo {
                layer_count_or_depth: 2049,
                usage: usage | TextureUsageFlags::DEPTH_STENCIL_TARGET,
                sample_count: multi,
                format: TextureFormat::R16_FLOAT,
                ..three_d.clone()
            },
            &[
                "For 3D textures: width, height, and layer_count_or_depth must be <= 2048",
                "For 3D textures: usage must not contain DEPTH_STENCIL_TARGET",
                "For 3D textures: sample_count must be SDL_GPU_SAMPLECOUNT_1",
                "For 3D textures: the format is unsupported for the given usage",
            ],
        ),
        (
            TextureCreateInfo {
                sample_count: multi,
                num_levels: 2,
                ..array.clone()
            },
            &[
                "For array textures: sample_count must be SDL_GPU_SAMPLECOUNT_1",
                "For 2D multisample textures: num_levels must be 1",
            ],
        ),
        (
            TextureCreateInfo {
                format: TextureFormat::R16_FLOAT,
                ..base.clone()
            },
            &["For 2D textures: the format is unsupported for the given usage"],
        ),
        (
            // Compute-writable usage is checked against the format table
            // before the backend.
            TextureCreateInfo {
                format: TextureFormat::B8G8R8A8_UNORM,
                usage: TextureUsageFlags::COMPUTE_STORAGE_WRITE,
                ..base.clone()
            },
            &["For 2D textures: the format is unsupported for the given usage"],
        ),
    ];
    config().unsupported = vec![TextureFormat::R16_FLOAT];
    for (info, msgs) in cases {
        // The first failed check is the error, every one asserts.
        assert_eq!(err_message(device.create_texture(&info)), msgs[0]);
        assert_eq!(asserts.take(), msgs);
    }
    take_log();
    for info in [base, cube, cube_array, three_d, array] {
        assert!(device.create_texture(&info).is_ok());
    }
    assert!(asserts.take().is_empty());

    // The compute-writable table rejects without asking the backend.
    assert!(!device.texture_supports_format(
        TextureFormat::B8G8R8A8_UNORM,
        TextureType::Texture2D,
        TextureUsageFlags::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE
    ));
    take_log();
    assert!(device.texture_supports_format(
        TextureFormat::R8G8B8A8_UNORM,
        TextureType::Texture2D,
        TextureUsageFlags::COMPUTE_STORAGE_WRITE
    ));
    assert_eq!(take_log().len(), 1);
    assert!(!device.texture_supports_format(
        TextureFormat(200),
        TextureType::Texture2D,
        TextureUsageFlags::SAMPLER
    ));
    assert!(!device.texture_supports_sample_count(TextureFormat::INVALID, SampleCount::One));
    assert_eq!(
        asserts.take(),
        [
            "Invalid texture format enum!",
            "Invalid texture format enum!"
        ]
    );
    assert!(device.texture_supports_sample_count(TextureFormat::R8_UNORM, SampleCount::Four));
    assert!(!device.texture_supports_sample_count(TextureFormat::R8_UNORM, SampleCount::Eight));
    take_log();

    // A small buffer only asserts.
    assert!(device
        .create_buffer(&BufferCreateInfo {
            usage: BufferUsageFlags::INDEX,
            size: 2,
            props: None
        })
        .is_ok());
    assert_eq!(
        asserts.take(),
        ["Cannot create a buffer with size less than 4 bytes!"]
    );

    // Without debug mode the backend gets everything.
    let device = Device::new(ShaderFormat::SPIRV, false, None).unwrap();
    take_log();
    assert!(device.create_texture(&TextureCreateInfo::default()).is_ok());
    assert!(take_log()[0].starts_with("create_texture"));
    assert!(device.texture_supports_sample_count(TextureFormat(300), SampleCount::One));
    assert!(asserts.take().is_empty());
}

// Command buffers and passes

#[test]
fn render_pass_dispatch_and_validation() {
    let f = Fixture::new(true);
    let device = f.device();
    let asserts = Asserts::catch();
    let target = color_texture(&device);
    let small = device
        .create_texture(&TextureCreateInfo {
            width: 16,
            height: 8,
            ..texture_info(
                TextureFormat::R8G8B8A8_UNORM,
                TextureUsageFlags::COLOR_TARGET | TextureUsageFlags::SAMPLER,
            )
        })
        .unwrap();
    let depth = device
        .create_texture(&texture_info(
            TextureFormat::D16_UNORM,
            TextureUsageFlags::DEPTH_STENCIL_TARGET,
        ))
        .unwrap();
    let vs = shader(&device, ShaderStage::Vertex);
    let fs = shader(&device, ShaderStage::Fragment);
    let rgba = [ColorTargetDescription {
        format: TextureFormat::R8G8B8A8_UNORM,
        blend_state: Default::default(),
    }];
    let pipeline = device
        .create_graphics_pipeline(&GraphicsPipelineCreateInfo {
            vertex_shader: &vs,
            fragment_shader: &fs,
            vertex_input_state: Default::default(),
            primitive_type: PrimitiveType::TriangleList,
            rasterizer_state: Default::default(),
            multisample_state: Default::default(),
            depth_stencil_state: Default::default(),
            target_info: GraphicsPipelineTargetInfo {
                color_target_descriptions: &rgba,
                ..Default::default()
            },
            props: None,
        })
        .unwrap();
    let sampler = device
        .create_sampler(&SamplerCreateInfo::default())
        .unwrap();
    let vertices = device
        .create_buffer(&BufferCreateInfo {
            usage: BufferUsageFlags::VERTEX | BufferUsageFlags::INDIRECT,
            size: 64,
            props: None,
        })
        .unwrap();
    take_log();

    let mut cb = device.acquire_command_buffer().unwrap();
    let id = take_log()[0]
        .trim_start_matches("acquire_command_buffer ")
        .to_string();
    cb.insert_debug_label("label");
    cb.push_debug_group("group");
    cb.pop_debug_group();
    cb.push_vertex_uniform_data(3, &[1, 2]).unwrap();
    assert_eq!(
        err_message(cb.push_fragment_uniform_data(4, &[1])),
        "slot_index exceeds MAX_UNIFORM_BUFFERS_PER_STAGE"
    );
    assert_eq!(
        err_message(cb.push_compute_uniform_data(4, &[1])),
        "slot_index exceeds MAX_UNIFORM_BUFFERS_PER_STAGE"
    );
    assert_eq!(
        take_log(),
        [
            format!("insert_debug_label {id} label"),
            format!("push_debug_group {id} group"),
            format!("pop_debug_group {id}"),
            "push_vertex_uniform_data 3 [1, 2]".to_string(),
        ]
    );

    {
        let targets = [ColorTargetInfo::new(&target), ColorTargetInfo::new(&small)];
        let mut rp = cb
            .begin_render_pass(&targets, Some(&DepthStencilTargetInfo::new(&depth)))
            .unwrap();
        // Drawing needs a pipeline.
        assert_eq!(
            err_message(rp.draw_primitives(3, 1, 0, 0)),
            "Graphics pipeline not bound!"
        );
        assert_eq!(asserts.take(), ["Graphics pipeline not bound!"]);
        rp.bind_graphics_pipeline(&pipeline);
        // The viewport may not exceed the smallest target (16x8).
        rp.set_viewport(&Viewport {
            x: 0.0,
            y: 0.0,
            w: 16.0,
            h: 8.0,
            min_depth: 0.0,
            max_depth: 1.0,
        })
        .unwrap();
        assert_eq!(
            err_message(rp.set_viewport(&Viewport {
                x: 1.0,
                w: 16.0,
                ..Default::default()
            })),
            "Viewport size exceeds current render target dimensions"
        );
        rp.set_scissor(&Rect::new(8, 4, 8, 4)).unwrap();
        assert_eq!(
            err_message(rp.set_scissor(&Rect::new(0, 0, 8, 9))),
            "Scissor rectangle size exceeds current render target dimensions"
        );
        // A negative scissor wraps to a huge one, as in C.
        assert!(rp.set_scissor(&Rect::new(-10, 0, 1, 1)).is_err());
        rp.set_blend_constants(FColor::new(1.0, 0.5, 0.25, 0.0));
        rp.set_stencil_reference(7);
        rp.bind_vertex_buffers(
            1,
            &[BufferBinding {
                buffer: &vertices,
                offset: 4,
            }],
        );
        rp.bind_index_buffer(
            &BufferBinding {
                buffer: &vertices,
                offset: 0,
            },
            IndexElementSize::Bits32,
        );
        // The pipeline needs a fragment sampler (the mock's header).
        rp.draw_primitives(3, 1, 0, 0).unwrap();
        assert_eq!(asserts.take().len(), 4);
        let binding = [TextureSamplerBinding {
            texture: &target,
            sampler: &sampler,
        }];
        rp.bind_fragment_samplers(0, &binding).unwrap();
        rp.draw_primitives(3, 2, 1, 0).unwrap();
        rp.draw_indexed_primitives(6, 1, 0, -2, 0).unwrap();
        rp.draw_primitives_indirect(&vertices, 16, 2).unwrap();
        rp.draw_indexed_primitives_indirect(&vertices, 0, 1)
            .unwrap();
        assert!(asserts.take().is_empty());
        rp.bind_vertex_samplers(15, &binding).unwrap();
        // Empty sampler bindings are recorded but not sent to the backend.
        rp.bind_vertex_samplers(16, &[]).unwrap();
        rp.bind_vertex_storage_textures(7, &[&target]).unwrap();
        rp.bind_vertex_storage_buffers(0, &[&vertices, &vertices])
            .unwrap();
        rp.bind_fragment_storage_textures(0, &[]).unwrap();
        rp.bind_fragment_storage_buffers(7, &[&vertices]).unwrap();
        // Slot ranges.
        assert_eq!(
            err_message(rp.bind_vertex_samplers(16, &binding)),
            "first_slot + num_bindings exceeds MAX_TEXTURE_SAMPLERS_PER_STAGE"
        );
        assert_eq!(
            err_message(rp.bind_fragment_samplers(u32::MAX, &binding)),
            "first_slot + num_bindings exceeds MAX_TEXTURE_SAMPLERS_PER_STAGE"
        );
        assert_eq!(
            err_message(rp.bind_vertex_storage_textures(8, &[&target])),
            "first_slot + num_bindings exceeds MAX_STORAGE_TEXTURES_PER_STAGE"
        );
        assert_eq!(
            err_message(rp.bind_fragment_storage_textures(u32::MAX, &[&target])),
            "first_slot + num_bindings exceeds MAX_STORAGE_TEXTURES_PER_STAGE"
        );
        assert_eq!(
            err_message(rp.bind_vertex_storage_buffers(4, &[&vertices; 5])),
            "first_slot + num_bindings exceeds MAX_STORAGE_BUFFERS_PER_STAGE"
        );
        assert_eq!(
            err_message(rp.bind_fragment_storage_buffers(8, &[&vertices])),
            "first_slot + num_bindings exceeds MAX_STORAGE_BUFFERS_PER_STAGE"
        );
        // Another pass can't begin, nor a swapchain texture be acquired.
        let cb2 = rp.command_buffer();
        assert_eq!(
            err_message(cb2.begin_copy_pass()),
            "Cannot begin copy pass during another pass!"
        );
        assert_eq!(
            err_message(cb2.begin_render_pass(&[], None)),
            "Cannot begin render pass during another pass!"
        );
        assert_eq!(
            err_message(cb2.begin_compute_pass(&[], &[])),
            "Cannot begin compute pass during another pass!"
        );
        assert_eq!(
            err_message(cb2.generate_mipmaps(&target)),
            "Cannot generate mipmaps during a pass!"
        );
        cb2.push_fragment_uniform_data(0, &[9]).unwrap();
        assert_eq!(asserts.take().len(), 4);
        rp.end();
    }
    let log = take_log();
    let t = tex(&target);
    let s = name(&sampler.raw);
    let v = name(&vertices.raw);
    assert_eq!(
        log,
        [
            format!(
                "begin_render_pass {id} [{t},{}] depth={}",
                tex(&small),
                tex(&depth)
            ),
            format!("bind_graphics_pipeline {id} {}", name(&pipeline.raw)),
            "set_viewport 0 0 16 8 0 1".to_string(),
            "set_scissor 8 4 8 4".to_string(),
            "set_blend_constants 1 0.5 0.25 0".to_string(),
            "set_stencil_reference 7".to_string(),
            format!("bind_vertex_buffers 1 [{v}@4]"),
            format!("bind_index_buffer {v}@0 Bits32 (4 bytes)"),
            "draw_primitives 3 1 0 0".to_string(),
            format!("bind_fragment_samplers 0 [{t}+{s}]"),
            "draw_primitives 3 2 1 0".to_string(),
            "draw_indexed_primitives 6 1 0 -2 0".to_string(),
            format!("draw_primitives_indirect {v} 16 2"),
            format!("draw_indexed_primitives_indirect {v} 0 1"),
            format!("bind_vertex_samplers 15 [{t}+{s}]"),
            format!("bind_vertex_storage_textures 7 [{t}]"),
            format!("bind_vertex_storage_buffers 0 [{v},{v}]"),
            "bind_fragment_storage_textures 0 []".to_string(),
            format!("bind_fragment_storage_buffers 7 [{v}]"),
            "push_fragment_uniform_data 0 [9]".to_string(),
            format!("end_render_pass {id}"),
        ]
    );
    // The pass state is reset.
    assert!(!cb.header.render_pass.in_progress);
    assert!(cb.header.render_pass.graphics_pipeline.is_none());
    assert!(!cb.header.render_pass.fragment_sampler_bound[0]);

    // Too many color targets.
    let nine = [ColorTargetInfo::new(&target); 9];
    assert_eq!(
        err_message(cb.begin_render_pass(&nine, None)),
        "num_color_targets exceeds MAX_COLOR_TARGET_BINDINGS"
    );

    // Color target checks.
    let multisampled = device
        .create_texture(&TextureCreateInfo {
            sample_count: SampleCount::Four,
            ..texture_info(
                TextureFormat::R8G8B8A8_UNORM,
                TextureUsageFlags::COLOR_TARGET,
            )
        })
        .unwrap();
    let resolve_bad_samples = device
        .create_texture(&TextureCreateInfo {
            sample_count: SampleCount::Two,
            ..texture_info(
                TextureFormat::R8G8B8A8_UNORM,
                TextureUsageFlags::COLOR_TARGET,
            )
        })
        .unwrap();
    let resolve_bad_format = device
        .create_texture(&texture_info(
            TextureFormat::B8G8R8A8_UNORM,
            TextureUsageFlags::COLOR_TARGET,
        ))
        .unwrap();
    let resolve_3d = device
        .create_texture(&TextureCreateInfo {
            texture_type: TextureType::Texture3D,
            ..texture_info(
                TextureFormat::R8G8B8A8_UNORM,
                TextureUsageFlags::COLOR_TARGET,
            )
        })
        .unwrap();
    let resolve_sampler_only = device
        .create_texture(&texture_info(
            TextureFormat::R8G8B8A8_UNORM,
            TextureUsageFlags::SAMPLER,
        ))
        .unwrap();
    let cases: Vec<(ColorTargetInfo<'_>, &str)> = vec![
        (
            ColorTargetInfo {
                cycle: true,
                ..ColorTargetInfo::new(&target)
            },
            "Cannot cycle color target when load op is LOAD!",
        ),
        (
            ColorTargetInfo {
                store_op: StoreOp::Resolve,
                ..ColorTargetInfo::new(&multisampled)
            },
            "Store op is RESOLVE or RESOLVE_AND_STORE but resolve_texture is NULL!",
        ),
        (
            ColorTargetInfo {
                store_op: StoreOp::ResolveAndStore,
                resolve_texture: Some(&small),
                ..ColorTargetInfo::new(&target)
            },
            "Store op is RESOLVE or RESOLVE_AND_STORE but texture is not multisample!",
        ),
        (
            ColorTargetInfo {
                store_op: StoreOp::Resolve,
                resolve_texture: Some(&resolve_bad_samples),
                ..ColorTargetInfo::new(&multisampled)
            },
            "Resolve texture must have a sample count of 1!",
        ),
        (
            ColorTargetInfo {
                store_op: StoreOp::Resolve,
                resolve_texture: Some(&resolve_bad_format),
                ..ColorTargetInfo::new(&multisampled)
            },
            "Resolve texture must have the same format as its corresponding color target!",
        ),
        (
            ColorTargetInfo {
                store_op: StoreOp::Resolve,
                resolve_texture: Some(&resolve_3d),
                ..ColorTargetInfo::new(&multisampled)
            },
            "Resolve texture must not be of TEXTURETYPE_3D!",
        ),
        (
            ColorTargetInfo {
                store_op: StoreOp::Resolve,
                resolve_texture: Some(&resolve_sampler_only),
                ..ColorTargetInfo::new(&multisampled)
            },
            "Resolve texture usage must include COLOR_TARGET!",
        ),
        (
            ColorTargetInfo {
                layer_or_depth_plane: 1,
                ..ColorTargetInfo::new(&target)
            },
            "Color target layer index must be less than the texture's layer count!",
        ),
        (
            ColorTargetInfo {
                mip_level: 1,
                ..ColorTargetInfo::new(&target)
            },
            "Color target mip level must be less than the texture's level count!",
        ),
    ];
    for (info, msg) in cases {
        assert_eq!(err_message(cb.begin_render_pass(&[info], None)), msg);
        assert_eq!(asserts.take(), [msg]);
    }
    // A cycled color target that clears is fine, and so is a resolve.
    let ok = [
        ColorTargetInfo {
            cycle: true,
            load_op: LoadOp::Clear,
            ..ColorTargetInfo::new(&target)
        },
        ColorTargetInfo {
            store_op: StoreOp::Resolve,
            resolve_texture: Some(&small),
            ..ColorTargetInfo::new(&multisampled)
        },
    ];
    cb.begin_render_pass(&ok, None).unwrap().end();

    // Depth-stencil target checks.
    let deep = device
        .create_texture(&TextureCreateInfo {
            texture_type: TextureType::Texture2DArray,
            layer_count_or_depth: 256,
            ..texture_info(
                TextureFormat::D16_UNORM,
                TextureUsageFlags::DEPTH_STENCIL_TARGET,
            )
        })
        .unwrap();
    let cases: Vec<(DepthStencilTargetInfo<'_>, &str)> = vec![
        (
            DepthStencilTargetInfo::new(&target),
            "Depth target must have been created with the DEPTH_STENCIL_TARGET usage flag!",
        ),
        (
            DepthStencilTargetInfo::new(&deep),
            "Cannot bind a depth texture with more than 255 layers!",
        ),
        (
            DepthStencilTargetInfo {
                cycle: true,
                load_op: LoadOp::Clear,
                ..DepthStencilTargetInfo::new(&depth)
            },
            "Cannot cycle depth target when load op or stencil load op is LOAD!",
        ),
        (
            DepthStencilTargetInfo {
                stencil_store_op: StoreOp::ResolveAndStore,
                ..DepthStencilTargetInfo::new(&depth)
            },
            "RESOLVE store ops are not supported for depth-stencil targets!",
        ),
    ];
    for (info, msg) in cases {
        assert_eq!(err_message(cb.begin_render_pass(&[], Some(&info))), msg);
        assert_eq!(asserts.take(), [msg]);
    }
    let cycled = DepthStencilTargetInfo {
        cycle: true,
        load_op: LoadOp::Clear,
        stencil_load_op: LoadOp::DontCare,
        ..DepthStencilTargetInfo::new(&depth)
    };
    cb.begin_render_pass(&[], Some(&cycled)).unwrap().end();
    take_log();

    // A pass left in progress (forgotten) blocks submitting.
    std::mem::forget(cb.begin_copy_pass().unwrap());
    assert_eq!(
        err_message(cb.begin_render_pass(&[], None)),
        "Cannot begin render pass during another pass!"
    );
    asserts.take();
    assert_eq!(
        err_message(cb.submit()),
        "Cannot submit command buffer while a pass is in progress!"
    );
    // (The dropped command buffer is cancelled.)
    assert_eq!(
        take_log(),
        [format!("begin_copy_pass {id}"), format!("cancel {id}")]
    );
    assert_eq!(
        asserts.take(),
        ["Cannot submit command buffer while a pass is in progress!"]
    );
}

#[test]
fn bindings_are_checked_before_draws_and_dispatches() {
    let f = Fixture::new(true);
    let device = f.device();
    let asserts = Asserts::catch();
    let target = color_texture(&device);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut rp = cb
            .begin_render_pass(&[ColorTargetInfo::new(&target)], None)
            .unwrap();
        let mut pipeline_header = GraphicsPipelineHeader {
            num_vertex_samplers: 1,
            num_vertex_storage_textures: 1,
            num_vertex_storage_buffers: 1,
            num_vertex_uniform_buffers: 1,
            num_fragment_samplers: 2,
            num_fragment_storage_textures: 1,
            num_fragment_storage_buffers: 1,
            num_fragment_uniform_buffers: 1,
        };
        rp.cmd.header.render_pass.graphics_pipeline = Some(pipeline_header);
        rp.draw_primitives(1, 1, 0, 0).unwrap();
        assert_eq!(
            asserts.take(),
            [
                "Missing vertex sampler binding!",
                "Missing vertex storage texture binding!",
                "Missing vertex storage buffer binding!",
                "Missing fragment sampler binding!",
                "Missing fragment sampler binding!",
                "Missing fragment storage texture binding!",
                "Missing fragment storage buffer binding!",
            ]
        );
        // Counts past the binding arrays don't read past them.
        pipeline_header.num_vertex_samplers = 17;
        rp.cmd.header.render_pass.graphics_pipeline = Some(pipeline_header);
        rp.draw_indexed_primitives(1, 1, 0, 0, 0).unwrap();
        assert_eq!(asserts.take().len(), 23);
    }
    {
        let mut cp = cb.begin_compute_pass(&[], &[]).unwrap();
        assert_eq!(
            err_message(cp.dispatch(1, 1, 1)),
            "Compute pipeline not bound!"
        );
        assert_eq!(asserts.take(), ["Compute pipeline not bound!"]);
        cp.cmd.header.compute_pass.compute_pipeline = Some(ComputePipelineHeader {
            num_samplers: 1,
            num_readonly_storage_textures: 1,
            num_readonly_storage_buffers: 1,
            num_readwrite_storage_textures: 1,
            num_readwrite_storage_buffers: 1,
            num_uniform_buffers: 1,
        });
        cp.dispatch(1, 1, 1).unwrap();
        assert_eq!(
            asserts.take(),
            [
                "Missing compute sampler binding!",
                "Missing compute readonly storage texture binding!",
                "Missing compute readonly storage buffer binding!",
                "Missing compute read-write storage texture binding!",
                "Missing compute read-write storage buffer bbinding!",
            ]
        );
    }
    cb.cancel().unwrap();
}

#[test]
fn compute_pass_dispatch_and_validation() {
    let f = Fixture::new(true);
    let device = f.device();
    let asserts = Asserts::catch();
    let storage = device
        .create_texture(&TextureCreateInfo {
            num_levels: 2,
            ..texture_info(
                TextureFormat::R8G8B8A8_UNORM,
                TextureUsageFlags::COMPUTE_STORAGE_WRITE,
            )
        })
        .unwrap();
    let simultaneous = device
        .create_texture(&texture_info(
            TextureFormat::R32_FLOAT,
            TextureUsageFlags::COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE,
        ))
        .unwrap();
    let sampled = color_texture(&device);
    let multisampled = device
        .create_texture(&TextureCreateInfo {
            sample_count: SampleCount::Two,
            ..texture_info(
                TextureFormat::R8G8B8A8_UNORM,
                TextureUsageFlags::COLOR_TARGET,
            )
        })
        .unwrap();
    let buffer = device
        .create_buffer(&BufferCreateInfo {
            usage: BufferUsageFlags::COMPUTE_STORAGE_WRITE | BufferUsageFlags::INDIRECT,
            size: 64,
            props: None,
        })
        .unwrap();
    let sampler = device
        .create_sampler(&SamplerCreateInfo::default())
        .unwrap();
    let pipeline = device
        .create_compute_pipeline(&ComputePipelineCreateInfo {
            code: &[0; 4],
            entrypoint: "main",
            format: ShaderFormat::SPIRV,
            num_samplers: 1,
            num_readwrite_storage_textures: 2,
            num_readwrite_storage_buffers: 1,
            threadcount_x: 8,
            threadcount_y: 8,
            threadcount_z: 1,
            ..Default::default()
        })
        .unwrap();
    take_log();

    let window = Window::create("gpu", 8, 8, WindowFlags::default()).unwrap();
    let rw = |texture, mip_level, layer| StorageTextureReadWriteBinding {
        texture,
        mip_level,
        layer,
        cycle: false,
    };
    let buffer_rw = StorageBufferReadWriteBinding {
        buffer: &buffer,
        cycle: true,
    };
    let mut cb = device.acquire_command_buffer().unwrap();
    take_log();
    assert_eq!(
        err_message(cb.begin_compute_pass(&[rw(&storage, 0, 0); 9], &[])),
        "Parameter 'num_storage_texture_bindings' is invalid"
    );
    assert_eq!(
        err_message(cb.begin_compute_pass(&[], &[buffer_rw; 9])),
        "Parameter 'num_storage_buffer_bindings' is invalid"
    );
    let cases = [
        (
            rw(&sampled, 0, 0),
            "Texture must be created with COMPUTE_STORAGE_WRITE or COMPUTE_STORAGE_SIMULTANEOUS_READ_WRITE flag",
        ),
        (
            rw(&storage, 0, 1),
            "Storage texture layer index must be less than the texture's layer count!",
        ),
        (
            rw(&storage, 2, 0),
            "Storage texture mip level must be less than the texture's level count!",
        ),
    ];
    for (binding, msg) in cases {
        assert_eq!(err_message(cb.begin_compute_pass(&[binding], &[])), msg);
        assert_eq!(asserts.take(), [msg]);
    }
    assert!(take_log().is_empty());

    {
        let mut cp = cb
            .begin_compute_pass(&[rw(&storage, 1, 0), rw(&simultaneous, 0, 0)], &[buffer_rw])
            .unwrap();
        cp.bind_compute_pipeline(&pipeline);
        // The read-write bindings of the pass count; the sampler is missing.
        cp.dispatch(4, 2, 1).unwrap();
        assert_eq!(asserts.take(), ["Missing compute sampler binding!"]);
        let multisample_binding = [TextureSamplerBinding {
            texture: &multisampled,
            sampler: &sampler,
        }];
        cp.bind_compute_samplers(0, &multisample_binding).unwrap();
        assert_eq!(
            asserts.take(),
            ["Multisample textures cannot be bound as samplers!"]
        );
        cp.bind_compute_storage_textures(1, &[&sampled]).unwrap();
        cp.bind_compute_storage_buffers(0, &[&buffer]).unwrap();
        cp.dispatch_indirect(&buffer, 12).unwrap();
        assert!(asserts.take().is_empty());
        cp.command_buffer()
            .push_compute_uniform_data(1, &[5, 6])
            .unwrap();
        assert_eq!(
            err_message(cp.bind_compute_samplers(16, &multisample_binding)),
            "first_slot + num_bindings exceeds MAX_TEXTURE_SAMPLERS_PER_STAGE"
        );
        assert_eq!(
            err_message(cp.bind_compute_storage_textures(8, &[&sampled])),
            "first_slot + num_bindings exceeds MAX_STORAGE_TEXTURES_PER_STAGE"
        );
        assert_eq!(
            err_message(cp.bind_compute_storage_buffers(u32::MAX - 1, &[&buffer; 3])),
            "first_slot + num_bindings exceeds MAX_STORAGE_BUFFERS_PER_STAGE"
        );
        assert_eq!(
            err_message(cp.command_buffer().acquire_swapchain_texture(&window)),
            "Cannot acquire a swapchain texture during a pass!"
        );
        assert_eq!(asserts.take().len(), 1);
    }
    let id = cmd(cb.raw.as_deref().unwrap());
    let (st, si, sa, b) = (
        tex(&storage),
        tex(&simultaneous),
        tex(&sampled),
        name(&buffer.raw),
    );
    assert_eq!(
        take_log(),
        [
            format!("begin_compute_pass cmd{id} [{st},{si}] [{b}]"),
            format!("bind_compute_pipeline {}", name(&pipeline.raw)),
            "dispatch_compute 4 2 1".to_string(),
            format!(
                "bind_compute_samplers 0 [{}+{}]",
                tex(&multisampled),
                name(&sampler.raw)
            ),
            format!("bind_compute_storage_textures 1 [{sa}]"),
            format!("bind_compute_storage_buffers 0 [{b}]"),
            format!("dispatch_compute_indirect {b} 12"),
            "push_compute_uniform_data 1 [5, 6]".to_string(),
            format!("end_compute_pass cmd{id}"),
        ]
    );
    assert_eq!(cb.header.compute_pass.compute_pipeline, None);
    assert!(!cb.header.compute_pass.read_write_storage_texture_bound[0]);
    let fence = cb.submit_and_acquire_fence().unwrap();
    let fe = name(&fence.raw);
    assert_eq!(
        take_log(),
        [format!("submit_and_acquire_fence cmd{id} {fe}")]
    );
    assert!(fence.is_signaled());
    device.wait_for_fences(true, &[]).unwrap();
    device.wait_for_fences(false, &[&fence, &fence]).unwrap();
    device.wait_for_idle().unwrap();
    drop(fence);
    assert_eq!(
        take_log(),
        [
            format!("query_fence {fe}"),
            format!("wait_for_fences false [{fe},{fe}]"),
            "wait".to_string(),
            format!("release_fence {fe}"),
        ]
    );
}

#[test]
fn copy_pass_mipmaps_and_blits() {
    let f = Fixture::new(true);
    let device = f.device();
    let asserts = Asserts::catch();
    let usage = TextureUsageFlags::SAMPLER | TextureUsageFlags::COLOR_TARGET;
    let a = device
        .create_texture(&TextureCreateInfo {
            num_levels: 3,
            ..texture_info(TextureFormat::R8G8B8A8_UNORM, usage)
        })
        .unwrap();
    let b = color_texture(&device);
    let other_format = device
        .create_texture(&texture_info(TextureFormat::B8G8R8A8_UNORM, usage))
        .unwrap();
    let depth = device
        .create_texture(&texture_info(
            TextureFormat::D16_UNORM,
            TextureUsageFlags::SAMPLER,
        ))
        .unwrap();
    let multisampled = device
        .create_texture(&TextureCreateInfo {
            sample_count: SampleCount::Two,
            ..texture_info(
                TextureFormat::R8G8B8A8_UNORM,
                TextureUsageFlags::COLOR_TARGET,
            )
        })
        .unwrap();
    let buffer = device
        .create_buffer(&BufferCreateInfo {
            usage: BufferUsageFlags::VERTEX,
            size: 64,
            props: None,
        })
        .unwrap();
    let transfer = device
        .create_transfer_buffer(&TransferBufferCreateInfo {
            usage: TransferBufferUsage::Download,
            size: 64,
            props: None,
        })
        .unwrap();
    take_log();

    let mut cb = device.acquire_command_buffer().unwrap();
    let id = cmd(cb.raw.as_deref().unwrap());
    take_log();
    {
        let mut cp = cb.begin_copy_pass().unwrap();
        let region = TextureRegion {
            texture: &a,
            mip_level: 0,
            layer: 0,
            x: 0,
            y: 0,
            z: 0,
            w: 4,
            h: 2,
            d: 1,
        };
        let transfer_info = TextureTransferInfo {
            transfer_buffer: &transfer,
            offset: 8,
            pixels_per_row: 0,
            rows_per_layer: 0,
        };
        cp.upload_to_texture(&transfer_info, &region, true);
        cp.upload_to_buffer(
            &TransferBufferLocation {
                transfer_buffer: &transfer,
                offset: 4,
            },
            &BufferRegion {
                buffer: &buffer,
                offset: 16,
                size: 32,
            },
            false,
        );
        let location = |texture| TextureLocation {
            texture,
            mip_level: 0,
            layer: 0,
            x: 0,
            y: 0,
            z: 0,
        };
        cp.copy_texture_to_texture(&location(&a), &location(&b), 4, 4, 1, false)
            .unwrap();
        assert_eq!(
            err_message(cp.copy_texture_to_texture(
                &location(&a),
                &location(&other_format),
                4,
                4,
                1,
                false
            )),
            "Source and destination textures must have the same format!"
        );
        let buffer_location = BufferLocation {
            buffer: &buffer,
            offset: 0,
        };
        cp.copy_buffer_to_buffer(&buffer_location, &buffer_location, 8, true);
        cp.download_from_texture(&region, &transfer_info);
        cp.download_from_buffer(
            &BufferRegion {
                buffer: &buffer,
                offset: 0,
                size: 4,
            },
            &TransferBufferLocation {
                transfer_buffer: &transfer,
                offset: 0,
            },
        );
        cp.command_buffer().insert_debug_label("copied");
    }
    let (ta, tb, tr, bu) = (tex(&a), tex(&b), name(&transfer.raw), name(&buffer.raw));
    assert_eq!(
        take_log(),
        [
            format!("begin_copy_pass cmd{id}"),
            format!("upload_to_texture {tr}@8 -> {ta} 4x2x1 true"),
            format!("upload_to_buffer {tr}@4 -> {bu}@16+32 false"),
            format!("copy_texture_to_texture {ta} -> {tb} 4x4x1 false"),
            format!("copy_buffer_to_buffer {bu} -> {bu} 8 true"),
            format!("download_from_texture {ta} -> {tr}@8"),
            format!("download_from_buffer {bu} -> {tr}@0"),
            format!("insert_debug_label cmd{id} copied"),
            format!("end_copy_pass cmd{id}"),
        ]
    );
    assert_eq!(
        asserts.take(),
        ["Source and destination textures must have the same format!"]
    );

    // Mipmaps
    cb.generate_mipmaps(&a).unwrap();
    assert_eq!(
        take_log(),
        [format!(
            "generate_mipmaps cmd{id} {ta} ignore_validation=true"
        )]
    );
    assert!(!cb.header.ignore_render_pass_texture_validation);
    assert_eq!(
        err_message(cb.generate_mipmaps(&b)),
        "Cannot generate mipmaps for texture with num_levels <= 1!"
    );
    let no_sampler = device
        .create_texture(&TextureCreateInfo {
            num_levels: 2,
            ..texture_info(
                TextureFormat::R8G8B8A8_UNORM,
                TextureUsageFlags::COLOR_TARGET,
            )
        })
        .unwrap();
    assert_eq!(
        err_message(cb.generate_mipmaps(&no_sampler)),
        "GenerateMipmaps texture must be created with SAMPLER and COLOR_TARGET usage flags!"
    );
    asserts.take();
    take_log();

    // Blits, through the shared blit helpers of the mock.
    let region = |texture, w, h| BlitRegion {
        texture,
        mip_level: 0,
        layer_or_depth_plane: 0,
        x: 8,
        y: 4,
        w,
        h,
    };
    let info = BlitInfo {
        source: BlitRegion {
            mip_level: 1,
            ..region(&a, 16, 8)
        },
        destination: region(&b, 32, 16),
        load_op: LoadOp::DontCare,
        clear_color: FColor::default(),
        flip_mode: FlipMode::Horizontal,
        filter: Filter::Linear,
        cycle: false,
    };
    cb.blit_texture(&info).unwrap();
    let log = take_log();
    let pipeline = log[2].split(' ').nth(1).unwrap().to_string();
    assert_eq!(log[0], format!("blit {ta} -> {tb}"));
    assert!(log[1].starts_with("supports_texture_format TextureFormat::R8G8B8A8_UNORM"));
    assert!(
        log[2].ends_with("targets=[TextureFormat::R8G8B8A8_UNORM] depth_clip=false"),
        "{}",
        log[2]
    );
    // The uniforms: left, top, width, height in texcoords of mip level 1
    // (32x16), flipped horizontally; the mip level; the layer.
    let uniforms = super::sysgpu::BlitFragmentUniforms {
        left: 8.0 / 32.0 + 16.0 / 32.0,
        top: 4.0 / 16.0,
        width: -16.0 / 32.0,
        height: 8.0 / 16.0,
        mip_level: 1,
        layer_or_depth: 0.0,
    };
    assert_eq!(
        log[3..],
        [
            format!("begin_render_pass cmd{id} [{tb}] depth="),
            "set_viewport 8 4 32 16 0 1".to_string(),
            format!("bind_graphics_pipeline cmd{id} {pipeline}"),
            format!("bind_fragment_samplers 0 [{ta}+linear#{}]", {
                // (the linear sampler was made after the six shaders)
                let first: u32 = log[2]
                    .split("vs=blitshader#")
                    .nth(1)
                    .unwrap()
                    .split(' ')
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap();
                first + 6
            }),
            format!("push_fragment_uniform_data 0 {:?}", uniforms.to_bytes()),
            "draw_primitives 3 1 0 0".to_string(),
            format!("end_render_pass cmd{id}"),
        ]
    );
    // The second blit to the same format reuses the cached pipeline.
    cb.blit_texture(&BlitInfo {
        flip_mode: FlipMode::None,
        filter: Filter::Nearest,
        ..info
    })
    .unwrap();
    let log = take_log();
    assert!(!log
        .iter()
        .any(|l| l.starts_with("create_graphics_pipeline")));
    assert!(log.iter().any(|l| l.contains("+nearest#")));
    assert!(asserts.take().is_empty());

    let blit = |source, destination| BlitInfo {
        source,
        destination,
        ..info
    };
    let cases: Vec<(BlitInfo<'_>, &[&str])> = vec![
        (
            blit(region(&multisampled, 4, 4), region(&b, 4, 4)),
            &[
                "Blit source texture must have a sample count of 1",
                "Blit source texture must be created with the SAMPLER usage flag",
            ],
        ),
        (
            blit(region(&depth, 4, 4), region(&depth, 0, 4)),
            &[
                "Blit destination texture must be created with the COLOR_TARGET usage flag",
                "Blit source texture cannot have a depth format",
                "Blit source/destination regions must have non-zero width, height, and depth",
            ],
        ),
    ];
    for (info, msgs) in cases {
        assert_eq!(err_message(cb.blit_texture(&info)), msgs[0]);
        assert_eq!(asserts.take(), msgs);
    }
    {
        let mut rp = cb.begin_render_pass(&[], None).unwrap();
        assert_eq!(
            err_message(rp.command_buffer().blit_texture(&info)),
            "Cannot blit during a pass!"
        );
    }
    asserts.take();
    take_log();
    cb.submit().unwrap();
    assert_eq!(take_log(), [format!("submit cmd{id}")]);

    // The cache helper on its own: per format, and format-agnostic.
    let shaders: Vec<Shader> = (0..6)
        .map(|_| Shader::from_backend(object("s", 0)))
        .collect();
    let blit_shaders = BlitShaders {
        vertex: &shaders[0],
        from_2d: &shaders[1],
        from_2d_array: &shaders[2],
        from_3d: &shaders[3],
        from_cube: &shaders[4],
        from_cube_array: &shaders[5],
    };
    let mut cache = BlitPipelineCache::PerFormat(Vec::new());
    for (texture_type, fragment) in [
        (TextureType::Texture2D, 1),
        (TextureType::Texture2DArray, 2),
        (TextureType::Texture3D, 3),
        (TextureType::Cube, 4),
        (TextureType::CubeArray, 5),
    ] {
        fetch_blit_pipeline(
            &device.shared,
            texture_type,
            TextureFormat::R8G8B8A8_UNORM,
            &blit_shaders,
            &mut cache,
        )
        .unwrap();
        let log = take_log();
        assert!(
            log.last()
                .unwrap()
                .contains(&format!("fs={} ", name(&shaders[fragment].raw))),
            "{log:?}"
        );
    }
    // Cached: no new pipeline.
    fetch_blit_pipeline(
        &device.shared,
        TextureType::Cube,
        TextureFormat::R8G8B8A8_UNORM,
        &blit_shaders,
        &mut cache,
    )
    .unwrap();
    assert!(take_log().is_empty());
    // A pipeline the front end refuses.
    assert_eq!(
        err_message(fetch_blit_pipeline(
            &device.shared,
            TextureType::Cube,
            TextureFormat::D16_UNORM,
            &blit_shaders,
            &mut cache,
        )),
        "Failed to create GPU pipeline for blit"
    );
    assert_eq!(
        asserts.take(),
        ["Color target formats cannot be a depth format!"]
    );
    // The cached pipelines belong to the backend: dropping the cache
    // releases nothing.
    let BlitPipelineCache::PerFormat(entries) = cache else {
        unreachable!()
    };
    let agnostic = BlitPipelineCache::FormatAgnostic(entries);
    let mut agnostic = agnostic;
    let p = fetch_blit_pipeline(
        &device.shared,
        TextureType::Texture3D,
        TextureFormat::B8G8R8A8_UNORM,
        &blit_shaders,
        &mut agnostic,
    )
    .unwrap();
    // (indexed by source texture type, whatever the format)
    assert_eq!(name(&p.raw), {
        let BlitPipelineCache::FormatAgnostic(e) = &agnostic else {
            unreachable!()
        };
        name(&e[2].pipeline.raw)
    });
    drop(agnostic);
    drop(shaders);
    assert!(take_log().is_empty());
}

#[test]
fn swapchains_and_windows() {
    let f = Fixture::new(true);
    let device = f.device();
    let asserts = Asserts::catch();
    let window = Window::create("gpu", 64, 48, WindowFlags::default()).unwrap();
    let transparent = Window::create("gpu", 64, 48, WindowFlags::TRANSPARENT).unwrap();

    assert_eq!(
        err_message(device.claim_window(&transparent)),
        "The GPU API doesn't support transparent windows"
    );
    device.claim_window(&window).unwrap();
    assert!(device.window_supports_swapchain_composition(&window, SwapchainComposition::Sdr));
    assert!(!device.window_supports_present_mode(&window, PresentMode::Mailbox));
    device
        .set_swapchain_parameters(
            &window,
            SwapchainComposition::SdrLinear,
            PresentMode::Immediate,
        )
        .unwrap();
    assert_eq!(
        device.swapchain_texture_format(&window).unwrap(),
        TextureFormat::B8G8R8A8_UNORM
    );
    device.wait_for_swapchain(&window).unwrap();
    device.set_allowed_frames_in_flight(2).unwrap();
    device.set_allowed_frames_in_flight(0).unwrap();
    device.set_allowed_frames_in_flight(9).unwrap();
    assert_eq!(
        asserts.take(),
        [
            "allowed_frames_in_flight value must be between 1 and 3!",
            "allowed_frames_in_flight value must be between 1 and 3!"
        ]
    );
    assert_eq!(
        take_log(),
        [
            "claim_window",
            "supports_swapchain_composition Sdr",
            "supports_present_mode Mailbox",
            "set_swapchain_parameters SdrLinear Immediate",
            "swapchain_texture_format",
            "wait_for_swapchain",
            "set_allowed_frames_in_flight 2",
            "set_allowed_frames_in_flight 1",
            "set_allowed_frames_in_flight 3",
        ]
    );

    // A swapchain texture can't be cancelled; it isn't released.
    let mut cb = device.acquire_command_buffer().unwrap();
    let id = cmd(cb.raw.as_deref().unwrap());
    let swapchain = cb.acquire_swapchain_texture(&window).unwrap().unwrap();
    assert_eq!((swapchain.width, swapchain.height), (64, 48));
    {
        let mut rp = cb
            .begin_render_pass(&[ColorTargetInfo::new(&swapchain.texture)], None)
            .unwrap();
        rp.set_viewport(&Viewport {
            w: 64.0,
            h: 48.0,
            ..Default::default()
        })
        .unwrap();
    }
    drop(swapchain);
    assert_eq!(
        err_message(cb.cancel()),
        "Cannot cancel command buffer after a swapchain texture has been acquired!"
    );
    // (and so the dropped command buffer is submitted)
    let log = take_log();
    assert_eq!(log[0], format!("acquire_command_buffer cmd{id}"));
    assert_eq!(log[1], format!("acquire_swapchain_texture cmd{id}"));
    assert!(log[2].starts_with(&format!("begin_render_pass cmd{id} [swapchain#")));
    assert_eq!(log[3], "set_viewport 0 0 64 48 0 0");
    assert_eq!(log[4], format!("end_render_pass cmd{id}"));
    assert_eq!(log[5], format!("submit cmd{id}"));
    assert_eq!(log.len(), 6);
    asserts.take();

    // No texture available: no error, and the buffer can be cancelled.
    config().swapchain = false;
    let mut cb = device.acquire_command_buffer().unwrap();
    assert!(cb
        .wait_and_acquire_swapchain_texture(&window)
        .unwrap()
        .is_none());
    assert!(!cb.header.swapchain_texture_acquired);
    cb.cancel().unwrap();
    config().swapchain = true;
    let mut cb = device.acquire_command_buffer().unwrap();
    assert!(cb
        .wait_and_acquire_swapchain_texture(&window)
        .unwrap()
        .is_some());
    drop(cb);
    let log = take_log();
    assert!(log[1].starts_with("wait_and_acquire_swapchain_texture"));
    assert!(log[2].starts_with("cancel"));
    assert!(log.last().unwrap().starts_with("submit"));

    // A dropped command buffer is cancelled.
    drop(device.acquire_command_buffer().unwrap());
    let log = take_log();
    assert!(log[1].starts_with("cancel"));

    device.release_window(&window);
    assert_eq!(take_log(), ["release_window"]);
    assert!(asserts.take().is_empty());

    // An invalid window.
    window.destroy();
    assert_eq!(err_message(device.claim_window(&window)), "Invalid window");
}

#[test]
fn passes_are_not_tracked_without_debug_mode() {
    let _f = Fixture::new(true);
    let device = Device::new(ShaderFormat::SPIRV, false, None).unwrap();
    let asserts = Asserts::catch();
    let target = color_texture(&device);
    let mut cb = device.acquire_command_buffer().unwrap();
    {
        let mut rp = cb
            .begin_render_pass(
                &[ColorTargetInfo {
                    cycle: true,
                    layer_or_depth_plane: 5,
                    ..ColorTargetInfo::new(&target)
                }],
                None,
            )
            .unwrap();
        assert!(!rp.cmd.header.render_pass.in_progress);
        rp.draw_primitives(3, 1, 0, 0).unwrap();
        rp.set_viewport(&Viewport {
            w: 100000.0,
            ..Default::default()
        })
        .unwrap();
        // The parameter checks stay.
        assert!(rp.bind_vertex_samplers(17, &[]).is_err());
        // And nothing stops a nested pass.
        rp.command_buffer().begin_copy_pass().unwrap().end();
    }
    assert!(asserts.take().is_empty());
    let log = take_log();
    assert!(log.iter().any(|l| l == "draw_primitives 3 1 0 0"));
    assert!(log.iter().any(|l| l == "set_viewport 0 0 100000 0 0 0"));
    cb.submit().unwrap();
}
