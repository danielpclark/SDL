//! EGL/GLES and Vulkan probes.

use crate::dl::Lib;
use crate::{cstr, f, fixed_cstr, say};
use std::ffi::{c_char, c_void};

type P = *mut c_void;

const EGL_NONE: i32 = 0x3038;
const EGL_SURFACE_TYPE: i32 = 0x3033;
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
const EGL_RED_SIZE: i32 = 0x3024;
const EGL_GREEN_SIZE: i32 = 0x3023;
const EGL_BLUE_SIZE: i32 = 0x3022;
const EGL_PBUFFER_BIT: i32 = 1;
const EGL_WINDOW_BIT: i32 = 4;
const EGL_WIDTH: i32 = 0x3057;
const EGL_HEIGHT: i32 = 0x3056;
const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;
const EGL_OPENGL_ES_API: u32 = 0x30A0;

/// EGL with pbuffer contexts of ES 1, 2 and 3; with `window` (an
/// ANativeWindow), an ES 3 window surface too, cleared and swapped.
pub fn egl(label: &str, window: P) -> Result<(), String> {
    let egl = Lib::open("libEGL.so")?;
    let gl2 = Lib::open("libGLESv2.so")?;
    let gl1 = Lib::open("libGLESv1_CM.so");
    let get_display = f!(egl, "eglGetDisplay", fn(P) -> P);
    let initialize = f!(egl, "eglInitialize", fn(P, *mut i32, *mut i32) -> u32);
    let query_string = f!(egl, "eglQueryString", fn(P, i32) -> *const c_char);
    let get_configs = f!(egl, "eglGetConfigs", fn(P, *mut P, i32, *mut i32) -> u32);
    let choose_config = f!(egl, "eglChooseConfig", fn(P, *const i32, *mut P, i32, *mut i32) -> u32);
    let create_pbuffer = f!(egl, "eglCreatePbufferSurface", fn(P, P, *const i32) -> P);
    let create_window = f!(egl, "eglCreateWindowSurface", fn(P, P, P, *const i32) -> P);
    let bind_api = f!(egl, "eglBindAPI", fn(u32) -> u32);
    let create_context = f!(egl, "eglCreateContext", fn(P, P, P, *const i32) -> P);
    let make_current = f!(egl, "eglMakeCurrent", fn(P, P, P, P) -> u32);
    let swap = f!(egl, "eglSwapBuffers", fn(P, P) -> u32);
    let destroy_surface = f!(egl, "eglDestroySurface", fn(P, P) -> u32);
    let destroy_context = f!(egl, "eglDestroyContext", fn(P, P) -> u32);
    let get_error = f!(egl, "eglGetError", fn() -> i32);
    let terminate = f!(egl, "eglTerminate", fn(P) -> u32);
    let get_string2 = f!(gl2, "glGetString", fn(u32) -> *const c_char);
    let get_integer = f!(gl2, "glGetIntegerv", fn(u32, *mut i32));
    let clear_color = f!(gl2, "glClearColor", fn(f32, f32, f32, f32));
    let clear = f!(gl2, "glClear", fn(u32));
    let read_pixels = f!(gl2, "glReadPixels", fn(i32, i32, i32, i32, u32, u32, *mut c_void));
    let finish = f!(gl2, "glFinish", fn());

    unsafe {
        let d = get_display(std::ptr::null_mut());
        let (mut maj, mut min) = (0, 0);
        if initialize(d, &mut maj, &mut min) == 0 {
            return Err(format!("eglInitialize failed 0x{:x}", get_error()));
        }
        say!("[{label}] EGL {maj}.{min} vendor={:?} version={:?} client_apis={:?}",
            cstr(query_string(d, 0x3053)), cstr(query_string(d, 0x3054)), cstr(query_string(d, 0x308D)));
        say!("[{label}] EGL extensions: {}", cstr(query_string(d, 0x3055)));
        let mut n = 0;
        get_configs(d, std::ptr::null_mut(), 0, &mut n);
        say!("[{label}] EGL configs: {n}");
        for (ver, bit) in [(1, 1), (2, 4), (3, 0x40)] {
            let surf_bit = if window.is_null() || ver != 3 { EGL_PBUFFER_BIT } else { EGL_WINDOW_BIT };
            let attrs = [EGL_SURFACE_TYPE, surf_bit, EGL_RENDERABLE_TYPE, bit, EGL_RED_SIZE, 8,
                EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_NONE];
            let mut cfg: P = std::ptr::null_mut();
            let mut nc = 0;
            if choose_config(d, attrs.as_ptr(), &mut cfg, 1, &mut nc) == 0 || nc == 0 {
                say!("[{label}] ES{ver}: no config (0x{:x})", get_error());
                continue;
            }
            let s = if surf_bit == EGL_WINDOW_BIT {
                create_window(d, cfg, window, [EGL_NONE].as_ptr())
            } else {
                create_pbuffer(d, cfg, [EGL_WIDTH, 16, EGL_HEIGHT, 16, EGL_NONE].as_ptr())
            };
            if s.is_null() {
                say!("[{label}] ES{ver}: surface failed (0x{:x})", get_error());
                continue;
            }
            bind_api(EGL_OPENGL_ES_API);
            let c = create_context(d, cfg, std::ptr::null_mut(), [EGL_CONTEXT_CLIENT_VERSION, ver, EGL_NONE].as_ptr());
            if c.is_null() {
                say!("[{label}] ES{ver}: eglCreateContext failed (0x{:x})", get_error());
                destroy_surface(d, s);
                continue;
            }
            if make_current(d, s, s, c) == 0 {
                say!("[{label}] ES{ver}: eglMakeCurrent failed (0x{:x})", get_error());
            } else {
                let gs = if ver == 1 {
                    match &gl1 {
                        Ok(l) => l.sym::<unsafe extern "C" fn(u32) -> *const c_char>("glGetString").unwrap_or(get_string2),
                        Err(_) => get_string2,
                    }
                } else {
                    get_string2
                };
                say!("[{label}] ES{ver}: GL_VERSION={:?} GL_RENDERER={:?} GL_VENDOR={:?}",
                    cstr(gs(0x1F02)), cstr(gs(0x1F01)), cstr(gs(0x1F00)));
                if ver >= 2 {
                    say!("[{label}] ES{ver}: GLSL={:?}", cstr(gs(0x8B8C)));
                }
                if ver == 3 {
                    let (mut a, mut b, mut t) = (0, 0, 0);
                    get_integer(0x821B, &mut a);
                    get_integer(0x821C, &mut b);
                    get_integer(0x0D33, &mut t);
                    say!("[{label}] ES3: GL_MAJOR/MINOR={a}.{b} MAX_TEXTURE_SIZE={t}");
                    let ext = cstr(gs(0x1F03));
                    say!("[{label}] ES3: {} GL extensions: {ext}", ext.split_whitespace().count());
                    clear_color(0.25, 0.5, 0.75, 1.0);
                    clear(0x4000);
                    finish();
                    let mut px = [0u8; 4];
                    read_pixels(0, 0, 1, 1, 0x1908, 0x1401, px.as_mut_ptr().cast());
                    say!("[{label}] ES3: clear(0.25,0.5,0.75) read back {px:?}");
                    if surf_bit == EGL_WINDOW_BIT {
                        let r = swap(d, s);
                        say!("[{label}] ES3 window surface: eglSwapBuffers={r} (0x{:x})", get_error());
                    }
                }
                make_current(d, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
            }
            destroy_context(d, c);
            destroy_surface(d, s);
        }
        terminate(d);
    }
    Ok(())
}

const FEATURES: [&str; 55] = [
    "robustBufferAccess", "fullDrawIndexUint32", "imageCubeArray", "independentBlend", "geometryShader",
    "tessellationShader", "sampleRateShading", "dualSrcBlend", "logicOp", "multiDrawIndirect",
    "drawIndirectFirstInstance", "depthClamp", "depthBiasClamp", "fillModeNonSolid", "depthBounds",
    "wideLines", "largePoints", "alphaToOne", "multiViewport", "samplerAnisotropy", "textureCompressionETC2",
    "textureCompressionASTC_LDR", "textureCompressionBC", "occlusionQueryPrecise", "pipelineStatisticsQuery",
    "vertexPipelineStoresAndAtomics", "fragmentStoresAndAtomics", "shaderTessellationAndGeometryPointSize",
    "shaderImageGatherExtended", "shaderStorageImageExtendedFormats", "shaderStorageImageMultisample",
    "shaderStorageImageReadWithoutFormat", "shaderStorageImageWriteWithoutFormat",
    "shaderUniformBufferArrayDynamicIndexing", "shaderSampledImageArrayDynamicIndexing",
    "shaderStorageBufferArrayDynamicIndexing", "shaderStorageImageArrayDynamicIndexing", "shaderClipDistance",
    "shaderCullDistance", "shaderFloat64", "shaderInt64", "shaderInt16", "shaderResourceResidency",
    "shaderResourceMinLod", "sparseBinding", "sparseResidencyBuffer", "sparseResidencyImage2D",
    "sparseResidencyImage3D", "sparseResidency2Samples", "sparseResidency4Samples", "sparseResidency8Samples",
    "sparseResidency16Samples", "sparseResidencyAliased", "variableMultisampleRate", "inheritedQueries",
];

/// What the GPU API's Vulkan backend requires (`is_device_suitable`).
const GPU_API_NEEDS: [&str; 7] = [
    "independentBlend", "imageCubeArray", "depthClamp", "shaderClipDistance", "drawIndirectFirstInstance",
    "sampleRateShading", "samplerAnisotropy",
];

#[repr(C)]
struct AppInfo {
    s_type: u32,
    p_next: *const c_void,
    name: *const c_char,
    app_version: u32,
    engine: *const c_char,
    engine_version: u32,
    api_version: u32,
}

#[repr(C)]
struct InstanceInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: u32,
    app: *const AppInfo,
    layer_count: u32,
    layers: *const *const c_char,
    ext_count: u32,
    exts: *const *const c_char,
}

fn ver(v: u32) -> String {
    format!("{}.{}.{}", (v >> 22) & 0x7F, (v >> 12) & 0x3FF, v & 0xFFF)
}

pub fn vulkan(label: &str) -> Result<(), String> {
    let vk = Lib::open("libvulkan.so")?;
    let gipa = f!(vk, "vkGetInstanceProcAddr", fn(P, *const c_char) -> P);
    macro_rules! g {
        ($inst:expr, $name:literal, fn($($a:ty),*) $(-> $r:ty)?) => {{
            let p = unsafe { gipa($inst, concat!($name, "\0").as_ptr().cast()) };
            if p.is_null() {
                return Err(format!("vkGetInstanceProcAddr({}) = NULL", $name));
            }
            unsafe { std::mem::transmute::<P, unsafe extern "C" fn($($a),*) $(-> $r)?>(p) }
        }};
    }
    let null = std::ptr::null_mut();
    let mut api = 0x400000u32; // 1.0
    let eiv = unsafe { gipa(null, c"vkEnumerateInstanceVersion".as_ptr()) };
    if !eiv.is_null() {
        let eiv: unsafe extern "C" fn(*mut u32) -> i32 = unsafe { std::mem::transmute(eiv) };
        unsafe { eiv(&mut api) };
    }
    say!("[{label}] Vulkan loader instance version {}", ver(api));
    let enum_ext = g!(null, "vkEnumerateInstanceExtensionProperties", fn(*const c_char, *mut u32, *mut u8) -> i32);
    let mut n = 0u32;
    unsafe { enum_ext(std::ptr::null(), &mut n, std::ptr::null_mut()) };
    let mut buf = vec![0u8; 260 * n as usize];
    unsafe { enum_ext(std::ptr::null(), &mut n, buf.as_mut_ptr()) };
    let exts: Vec<String> = buf.chunks(260).take(n as usize).map(|c| fixed_cstr(&c[..256])).collect();
    say!("[{label}] Vulkan instance extensions ({n}): {}", exts.join(" "));
    let enum_layers = g!(null, "vkEnumerateInstanceLayerProperties", fn(*mut u32, *mut u8) -> i32);
    let mut nl = 0u32;
    unsafe { enum_layers(&mut nl, std::ptr::null_mut()) };
    say!("[{label}] Vulkan instance layers: {nl}");

    let app = AppInfo {
        s_type: 0,
        p_next: std::ptr::null(),
        name: c"spike".as_ptr(),
        app_version: 1,
        engine: c"spike".as_ptr(),
        engine_version: 1,
        api_version: api.min((1 << 22) | (3 << 12)),
    };
    let want_ext: Vec<*const c_char> = ["VK_KHR_get_physical_device_properties2"]
        .iter()
        .filter(|e| exts.iter().any(|x| x == *e))
        .map(|_| c"VK_KHR_get_physical_device_properties2".as_ptr())
        .collect();
    let info = InstanceInfo {
        s_type: 1,
        p_next: std::ptr::null(),
        flags: 0,
        app: &app,
        layer_count: 0,
        layers: std::ptr::null(),
        ext_count: want_ext.len() as u32,
        exts: want_ext.as_ptr(),
    };
    let create = g!(null, "vkCreateInstance", fn(*const InstanceInfo, P, *mut P) -> i32);
    let mut inst: P = null;
    let r = unsafe { create(&info, null, &mut inst) };
    if r != 0 {
        return Err(format!("vkCreateInstance = {r}"));
    }
    let enum_dev = g!(inst, "vkEnumeratePhysicalDevices", fn(P, *mut u32, *mut P) -> i32);
    let props = g!(inst, "vkGetPhysicalDeviceProperties", fn(P, *mut u8));
    let feats = g!(inst, "vkGetPhysicalDeviceFeatures", fn(P, *mut u32));
    let dev_ext = g!(inst, "vkEnumerateDeviceExtensionProperties", fn(P, *const c_char, *mut u32, *mut u8) -> i32);
    let qf = g!(inst, "vkGetPhysicalDeviceQueueFamilyProperties", fn(P, *mut u32, *mut u32));
    let destroy = g!(inst, "vkDestroyInstance", fn(P, P));
    let mut nd = 0u32;
    unsafe { enum_dev(inst, &mut nd, std::ptr::null_mut()) };
    let mut devs = vec![null; nd as usize];
    unsafe { enum_dev(inst, &mut nd, devs.as_mut_ptr()) };
    say!("[{label}] Vulkan physical devices: {nd}");
    for (i, &d) in devs.iter().enumerate() {
        let mut p = vec![0u64; 128]; // VkPhysicalDeviceProperties is 824 bytes
        unsafe { props(d, p.as_mut_ptr().cast()) };
        let b: &[u8] = unsafe { std::slice::from_raw_parts(p.as_ptr().cast(), 1024) };
        let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        say!("[{label}] device {i}: name={:?} api={} driver=0x{:x} vendor=0x{:x} device=0x{:x} type={} maxImageDimension2D={} maxImageArrayLayers={}",
            fixed_cstr(&b[20..276]), ver(u(0)), u(4), u(8), u(12), u(16), u(300), u(312));
        let mut f = [0u32; 55];
        unsafe { feats(d, f.as_mut_ptr()) };
        let on: Vec<&str> = FEATURES.iter().zip(f.iter()).filter(|(_, &v)| v != 0).map(|(n, _)| *n).collect();
        let off: Vec<&str> = FEATURES.iter().zip(f.iter()).filter(|(_, &v)| v == 0).map(|(n, _)| *n).collect();
        say!("[{label}] device {i} features ON: {}", on.join(" "));
        say!("[{label}] device {i} features OFF: {}", off.join(" "));
        let missing: Vec<&&str> = GPU_API_NEEDS.iter().filter(|n| off.contains(n)).collect();
        say!("[{label}] device {i} GPU-API-required features missing: {missing:?}");
        let mut ne = 0u32;
        unsafe { dev_ext(d, std::ptr::null(), &mut ne, std::ptr::null_mut()) };
        let mut eb = vec![0u8; 260 * ne as usize];
        unsafe { dev_ext(d, std::ptr::null(), &mut ne, eb.as_mut_ptr()) };
        let de: Vec<String> = eb.chunks(260).take(ne as usize).map(|c| fixed_cstr(&c[..256])).collect();
        say!("[{label}] device {i} extensions ({ne}): {}", de.join(" "));
        for need in ["VK_KHR_swapchain", "VK_KHR_maintenance1", "VK_KHR_driver_properties", "VK_ANDROID_external_memory_android_hardware_buffer", "VK_KHR_sampler_ycbcr_conversion"] {
            say!("[{label}] device {i} has {need}: {}", de.iter().any(|x| x == need));
        }
        let mut nq = 0u32;
        unsafe { qf(d, &mut nq, std::ptr::null_mut()) };
        let mut q = vec![0u32; 6 * nq as usize];
        unsafe { qf(d, &mut nq, q.as_mut_ptr()) };
        for k in 0..nq as usize {
            say!("[{label}] device {i} queue family {k}: flags=0x{:x} count={}", q[6 * k], q[6 * k + 1]);
        }
    }
    unsafe { destroy(inst, null) };
    Ok(())
}
