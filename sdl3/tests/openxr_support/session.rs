// The OpenXR hardware checks' session: a real runtime's session, swapchain
// and frames on a device made with OpenXR.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! [`run`]: what `gpu_openxr_hardware` (Linux, Windows) and
//! `android_openxr_loader`'s device check (a headset's runtime) do with
//! their device: its instance, system and name printed, the session, the
//! runtime's swapchain formats (printed; an 8-bit color one taken), a
//! two-layer color swapchain, the session's states and up to 120 rendered
//! frames (each eye's layer cleared, a projection layer submitted) or 10
//! s, the exit request, the session's end and every destruction (the
//! instance, the application's, after the device). It panics where the
//! runtime refuses a step.

use std::time::{Duration, Instant};

use super::xr::{self, Xr};
use sdl3::gpu::{
    self, ColorTargetInfo, Device, LoadOp, StoreOp, TextureFormat, XrSessionCreateInfo,
    XrSwapchainCreateInfo, XrSwapchainUsageFlags,
};
use sdl3::video::FColor;

/// The session and its frames on `device` (made with OpenXR), its lines
/// printed with `test` first; the device is dropped before its instance
/// is destroyed.
pub fn run(test: &str, device: Device) {
    let instance = device.xr_instance().expect("the device's instance");
    let system = device.xr_system_id().expect("the device's system");
    let name = device
        .properties()
        .get_string(gpu::PROP_GPU_DEVICE_NAME_STRING)
        .unwrap_or_default();
    println!(
        "{test}: {} device {name:?}, instance {instance:#x}, system {system:#x}",
        device.driver()
    );
    let gipa = gpu::openxr_get_xr_get_instance_proc_addr().expect("xrGetInstanceProcAddr");
    let x = Xr::load(gipa, instance);

    let session = device
        .create_xr_session(&XrSessionCreateInfo::default())
        .expect("the session");
    let formats = device
        .xr_swapchain_formats(session)
        .expect("the swapchain formats");
    println!(
        "{test}: swapchain formats {:?}",
        formats.iter().map(|f| f.0).collect::<Vec<_>>()
    );
    let format = [
        TextureFormat::R8G8B8A8_UNORM_SRGB,
        TextureFormat::B8G8R8A8_UNORM_SRGB,
        TextureFormat::R8G8B8A8_UNORM,
        TextureFormat::B8G8R8A8_UNORM,
    ]
    .into_iter()
    .find(|f| formats.contains(f))
    .expect("an 8-bit color swapchain format");
    let size = 512;
    let swapchain = device
        .create_xr_swapchain(
            session,
            &XrSwapchainCreateInfo {
                usage_flags: XrSwapchainUsageFlags::COLOR_ATTACHMENT,
                sample_count: 1,
                width: size,
                height: size,
                face_count: 1,
                array_size: 2,
                mip_count: 1,
                ..Default::default()
            },
            format,
        )
        .expect("the swapchain");
    println!(
        "{test}: a swapchain of {:?}, {} images",
        format,
        swapchain.textures().len()
    );
    assert!(!swapchain.textures().is_empty());

    let space_info = xr::XrReferenceSpaceCreateInfo {
        ty: xr::XR_TYPE_REFERENCE_SPACE_CREATE_INFO,
        next: std::ptr::null(),
        reference_space_type: xr::XR_REFERENCE_SPACE_TYPE_LOCAL,
        pose_in_reference_space: xr::XrPosef {
            orientation: [0.0, 0.0, 0.0, 1.0],
            position: [0.0; 3],
        },
    };
    let mut space = 0;
    // SAFETY: the runtime's function, the info and an out-pointer.
    assert_eq!(
        unsafe { (x.xrCreateReferenceSpace)(session, &space_info, &mut space) },
        0,
        "xrCreateReferenceSpace"
    );

    let mut states = Vec::new();
    let mut running = false;
    let mut exit_requested = false;
    let mut done = false;
    let mut frames = 0u32;
    let mut rendered = 0u32;
    let start = Instant::now();
    while !done && start.elapsed() < Duration::from_secs(30) {
        loop {
            let mut event = xr::XrEventDataBuffer {
                ty: xr::XR_TYPE_EVENT_DATA_BUFFER,
                next: std::ptr::null(),
                varying: [0; 4000],
            };
            // SAFETY: the runtime's function and a buffer.
            if unsafe { (x.xrPollEvent)(instance, &mut event) } != 0 {
                break;
            }
            if event.ty != xr::XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED {
                continue;
            }
            // SAFETY: the buffer holds that event.
            let state = unsafe {
                (*(&event as *const xr::XrEventDataBuffer)
                    .cast::<xr::XrEventDataSessionStateChanged>())
                .state
            };
            states.push(state);
            match state {
                xr::XR_SESSION_STATE_READY => {
                    let begin = xr::XrSessionBeginInfo {
                        ty: xr::XR_TYPE_SESSION_BEGIN_INFO,
                        next: std::ptr::null(),
                        primary_view_configuration_type:
                            xr::XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO,
                    };
                    // SAFETY: the runtime's function and the info.
                    assert_eq!(unsafe { (x.xrBeginSession)(session, &begin) }, 0);
                    running = true;
                }
                xr::XR_SESSION_STATE_STOPPING => {
                    // SAFETY: the runtime's function.
                    assert_eq!(unsafe { (x.xrEndSession)(session) }, 0);
                    running = false;
                }
                xr::XR_SESSION_STATE_EXITING | xr::XR_SESSION_STATE_LOSS_PENDING => done = true,
                _ => {}
            }
        }
        // (a runtime that never shows the session: give up after 10 s)
        if running
            && !exit_requested
            && (rendered >= 120 || start.elapsed() > Duration::from_secs(10))
        {
            // SAFETY: the runtime's function.
            assert_eq!(unsafe { (x.xrRequestExitSession)(session) }, 0);
            exit_requested = true;
        }
        if !running {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }

        let mut frame_state = xr::XrFrameState {
            ty: xr::XR_TYPE_FRAME_STATE,
            next: std::ptr::null_mut(),
            predicted_display_time: 0,
            predicted_display_period: 0,
            should_render: 0,
        };
        // SAFETY: the runtime's functions, infos and out-structures.
        unsafe {
            let wait = xr::XrHeader::new(xr::XR_TYPE_FRAME_WAIT_INFO);
            assert_eq!((x.xrWaitFrame)(session, &wait, &mut frame_state), 0);
            let begin = xr::XrHeader::new(xr::XR_TYPE_FRAME_BEGIN_INFO);
            assert!((x.xrBeginFrame)(session, &begin) >= 0);
        }
        frames += 1;
        let mut layers = Vec::new();
        let mut views = [xr::XrView {
            ty: xr::XR_TYPE_VIEW,
            next: std::ptr::null_mut(),
            pose: xr::XrPosef::default(),
            fov: xr::XrFovf::default(),
        }; 2];
        let projection_views: Vec<xr::XrCompositionLayerProjectionView>;
        let layer;
        if frame_state.should_render != 0 {
            let locate = xr::XrViewLocateInfo {
                ty: xr::XR_TYPE_VIEW_LOCATE_INFO,
                next: std::ptr::null(),
                view_configuration_type: xr::XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO,
                display_time: frame_state.predicted_display_time,
                space,
            };
            let mut view_state = xr::XrViewState {
                ty: xr::XR_TYPE_VIEW_STATE,
                next: std::ptr::null_mut(),
                view_state_flags: 0,
            };
            let mut count = 0;
            let mut index = 0;
            let wait = xr::XrSwapchainImageWaitInfo {
                ty: xr::XR_TYPE_SWAPCHAIN_IMAGE_WAIT_INFO,
                next: std::ptr::null(),
                timeout: xr::XR_INFINITE_DURATION,
            };
            // SAFETY: the runtime's functions, infos and out-pointers.
            unsafe {
                assert_eq!(
                    (x.xrLocateViews)(
                        session,
                        &locate,
                        &mut view_state,
                        2,
                        &mut count,
                        views.as_mut_ptr()
                    ),
                    0
                );
                let acquire = xr::XrHeader::new(xr::XR_TYPE_SWAPCHAIN_IMAGE_ACQUIRE_INFO);
                assert_eq!(
                    (x.xrAcquireSwapchainImage)(swapchain.handle(), &acquire, &mut index),
                    0
                );
                assert_eq!((x.xrWaitSwapchainImage)(swapchain.handle(), &wait), 0);
            }
            let texture = &swapchain.textures()[index as usize];
            let mut cb = device.acquire_command_buffer().expect("a command buffer");
            for eye in 0..2 {
                let mut target = ColorTargetInfo::new(texture);
                target.layer_or_depth_plane = eye;
                target.clear_color = if eye == 0 {
                    FColor::new(0.5, 0.0, 0.0, 1.0)
                } else {
                    FColor::new(0.0, 0.0, 0.5, 1.0)
                };
                target.load_op = LoadOp::Clear;
                target.store_op = StoreOp::Store;
                cb.begin_render_pass(&[target], None)
                    .expect("a render pass")
                    .end();
            }
            cb.submit().expect("the submission");
            let release = xr::XrHeader::new(xr::XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO);
            // SAFETY: the runtime's function and the info.
            assert_eq!(
                unsafe { (x.xrReleaseSwapchainImage)(swapchain.handle(), &release) },
                0
            );
            projection_views = (0..2)
                .map(|eye| xr::XrCompositionLayerProjectionView {
                    ty: xr::XR_TYPE_COMPOSITION_LAYER_PROJECTION_VIEW,
                    next: std::ptr::null(),
                    pose: views[eye].pose,
                    fov: views[eye].fov,
                    sub_image: xr::XrSwapchainSubImage {
                        swapchain: swapchain.handle(),
                        image_rect: [0, 0, size as i32, size as i32],
                        image_array_index: eye as u32,
                    },
                })
                .collect();
            layer = xr::XrCompositionLayerProjection {
                ty: xr::XR_TYPE_COMPOSITION_LAYER_PROJECTION,
                next: std::ptr::null(),
                layer_flags: 0,
                space,
                view_count: 2,
                views: projection_views.as_ptr(),
            };
            layers.push((&layer as *const xr::XrCompositionLayerProjection).cast::<xr::XrHeader>());
            rendered += 1;
        }
        let end = xr::XrFrameEndInfo {
            ty: xr::XR_TYPE_FRAME_END_INFO,
            next: std::ptr::null(),
            display_time: frame_state.predicted_display_time,
            environment_blend_mode: xr::XR_ENVIRONMENT_BLEND_MODE_OPAQUE,
            layer_count: layers.len() as u32,
            layers: layers.as_ptr(),
        };
        // SAFETY: the runtime's function, the info and its layers.
        assert_eq!(unsafe { (x.xrEndFrame)(session, &end) }, 0, "xrEndFrame");
    }
    println!(
        "{test}: states {states:?}, {frames} frames, {rendered} rendered, done {done} after {:?}",
        start.elapsed()
    );
    if rendered == 0 {
        println!("{test}: the runtime never made the session visible (a headset not worn?)");
    }
    assert!(frames > 0, "no frame");
    assert!(done, "the session didn't end");

    swapchain.destroy().expect("the swapchain's destruction");
    // SAFETY: the runtime's functions, with the space and the session.
    unsafe {
        assert_eq!((x.xrDestroySpace)(space), 0);
        assert_eq!((x.xrDestroySession)(session), 0);
    }
    drop(device);
    // SAFETY: the runtime's function, with the device's instance (the
    // application's, after the device).
    assert_eq!(unsafe { (x.xrDestroyInstance)(instance) }, 0);
}
