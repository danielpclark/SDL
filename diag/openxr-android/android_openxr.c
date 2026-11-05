/* diag only: the C twin of sdl3/tests/android_openxr.rs: no OpenXR loader,
   then the repository's test runtime (libsdl3_openxr_test_runtime.so in
   the app's jniLibs) in the loader's place, on upstream's SDL. */
#include "common.h"

#define RUNTIME "libsdl3_openxr_test_runtime.so"
#define FAILURES "xrGetInstanceProcAddr:xrInitializeLoaderKHR@1,xrInitializeLoaderKHR@1,xrInitializeLoaderKHR@3"

static SDL_AssertState SDLCALL assertion(const SDL_AssertData *data, void *userdata)
{
    (void)userdata;
    t("assertion: %s", data->condition);
    return SDL_ASSERTION_IGNORE;
}

static void failed_device(const char *what)
{
    SDL_PropertiesID p = xr_props();
    SDL_GPUDevice *dev = SDL_CreateGPUDeviceWithProperties(p);
    SDL_DestroyProperties(p);
    if (dev) {
        t("%s: ok (%s)", what, SDL_GetGPUDeviceDriver(dev));
        SDL_DestroyGPUDevice(dev);
    } else {
        t("%s: error: %s", what, SDL_GetError());
    }
}

static SDL_FColor clear_color(int swapchain, int layer, int frame)
{
    float f = (float)frame / 4.0f;
    SDL_FColor c;
    if (swapchain == 0) { c.r = 1.0f; c.g = 0.5f; c.b = f; c.a = 1.0f; }
    else if (layer == 0) { c.r = 1.0f; c.g = 0.0f; c.b = f; c.a = 1.0f; }
    else { c.r = 0.0f; c.g = 1.0f; c.b = f; c.a = 0.75f; }
    return c;
}

static const char *pixel(const Uint8 *p)
{
    static char buf[8][32];
    static int i = 0;
    i = (i + 1) % 8;
    SDL_snprintf(buf[i], sizeof(buf[i]), "%u,%u,%u,%u", p[0], p[1], p[2], p[3]);
    return buf[i];
}

#define TRACED_FRAMES 4

static void main_scenario(void)
{
    xr_instance = XR_NULL_HANDLE;
    xr_system = XR_NULL_SYSTEM_ID;
    SDL_PropertiesID p = xr_props();
    SDL_GPUDevice *dev = SDL_CreateGPUDeviceWithProperties(p);
    SDL_DestroyProperties(p);
    if (!dev) { t("device: error: %s", SDL_GetError()); return; }
    t("device: ok (%s)", SDL_GetGPUDeviceDriver(dev));
    XrInstance inst = xr_instance;
    t("instance: %s, system: %s", inst ? "non-null" : "null", xr_system ? "non-null" : "null");
    {
        const char *name = SDL_GetStringProperty(SDL_GetGPUDeviceProperties(dev), SDL_PROP_GPU_DEVICE_NAME_STRING, "");
        char first[64];
        size_t n = strcspn(name, " \t\n");
        if (n >= sizeof(first)) n = sizeof(first) - 1;
        memcpy(first, name, n); first[n] = 0;
        t("device name: %s", first);
    }

    PFN_xrGetInstanceProcAddr gipa = (PFN_xrGetInstanceProcAddr)SDL_OpenXR_GetXrGetInstanceProcAddr();
    if (!gipa) { t("xrGetInstanceProcAddr: error: %s", SDL_GetError()); return; }
#define LOADI(name) PFN_##name name = NULL; if (gipa(inst, #name, (PFN_xrVoidFunction *)&name) != XR_SUCCESS || !name) { SDL_Log("c: no %s", #name); abort(); }
    LOADI(xrPollEvent); LOADI(xrBeginSession); LOADI(xrEndSession); LOADI(xrRequestExitSession);
    LOADI(xrDestroySession); LOADI(xrWaitFrame); LOADI(xrBeginFrame); LOADI(xrEndFrame);
    LOADI(xrLocateViews); LOADI(xrAcquireSwapchainImage); LOADI(xrWaitSwapchainImage);
    LOADI(xrReleaseSwapchainImage); LOADI(xrCreateReferenceSpace); LOADI(xrDestroySpace);
    LOADI(xrDestroyInstance); LOADI(xrEnumerateSwapchainImages);

    XrSessionCreateInfo sci = { XR_TYPE_SESSION_CREATE_INFO };
    XrSession session = XR_NULL_HANDLE;
    SDL_ClearError();
    XrResult r = SDL_CreateGPUXRSession(dev, &sci, &session);
    if (r != XR_SUCCESS) { t("session: %s", xr_err(r)); return; }
    t("session: ok");

    {
        int nf = -1;
        SDL_GPUTextureFormat *fmts = SDL_GetGPUXRSwapchainFormats(dev, session, &nf);
        if (!fmts) t("formats: error: %s", SDL_GetError());
        else {
            char line[1024] = "";
            for (int i = 0; i < nf; i++) {
                char one[16];
                SDL_snprintf(one, sizeof(one), i ? " %d" : "%d", (int)fmts[i]);
                SDL_strlcat(line, one, sizeof(line));
            }
            t("formats: %s", line);
        }
        SDL_free(fmts);
    }

    const char *names[2] = { "one layer", "two layers" };
    const Uint32 sizes[2] = { 64, 32 };
    const Uint32 nlayers[2] = { 1, 2 };
    XrSwapchain sc[2];
    SDL_GPUTexture **tex[2];
    for (int k = 0; k < 2; k++) {
        XrSwapchainCreateInfo c = { XR_TYPE_SWAPCHAIN_CREATE_INFO };
        c.usageFlags = XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT | XR_SWAPCHAIN_USAGE_TRANSFER_SRC_BIT;
        c.sampleCount = 1; c.width = sizes[k]; c.height = sizes[k]; c.faceCount = 1; c.arraySize = nlayers[k]; c.mipCount = 1;
        SDL_ClearError();
        r = SDL_CreateGPUXRSwapchain(dev, session, &c, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, &sc[k], &tex[k]);
        if (r != XR_SUCCESS) { t("swapchain %s: %s", names[k], xr_err(r)); return; }
        Uint32 n = 0;
        xrEnumerateSwapchainImages(sc[k], 0, &n, NULL);
        t("swapchain %s: ok, %u images", names[k], n);
    }

    XrReferenceSpaceCreateInfo rsci = { XR_TYPE_REFERENCE_SPACE_CREATE_INFO };
    rsci.referenceSpaceType = XR_REFERENCE_SPACE_TYPE_LOCAL;
    rsci.poseInReferenceSpace.orientation.w = 1;
    XrSpace space = XR_NULL_HANDLE;
    t("reference space: %d", (int)xrCreateReferenceSpace(session, &rsci, &space));

    const Uint32 one = 64 * 64 * 4, two = 32 * 32 * 4;
    SDL_GPUTransferBufferCreateInfo tbi = { SDL_GPU_TRANSFERBUFFERUSAGE_DOWNLOAD, one + 2 * two, 0 };
    SDL_GPUTransferBuffer *tb = SDL_CreateGPUTransferBuffer(dev, &tbi);

    bool running = false, exit_requested = false, done = false;
    int state = 0;
    Uint32 traced = 0;
    for (int iter = 0; iter < 3000 && !done; iter++) {
        for (;;) {
            XrEventDataBuffer ev = { XR_TYPE_EVENT_DATA_BUFFER };
            if (xrPollEvent(inst, &ev) != XR_SUCCESS) break;
            if (ev.type != XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED) continue;
            state = (int)((XrEventDataSessionStateChanged *)&ev)->state;
            t("state %d", state);
            if (state == XR_SESSION_STATE_READY) {
                XrSessionBeginInfo sbi = { XR_TYPE_SESSION_BEGIN_INFO };
                sbi.primaryViewConfigurationType = XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO;
                r = xrBeginSession(session, &sbi);
                t("begin session: %d", (int)r);
                running = r == XR_SUCCESS;
            } else if (state == XR_SESSION_STATE_STOPPING) {
                t("end session: %d", (int)xrEndSession(session));
                running = false;
            } else if (state == XR_SESSION_STATE_EXITING || state == XR_SESSION_STATE_LOSS_PENDING) {
                done = true;
            }
        }
        if (!running) { SDL_Delay(5); continue; }

        XrFrameWaitInfo fwi = { XR_TYPE_FRAME_WAIT_INFO };
        XrFrameState fs = { XR_TYPE_FRAME_STATE };
        if (xrWaitFrame(session, &fwi, &fs) != XR_SUCCESS) { SDL_Log("c: xrWaitFrame"); abort(); }
        XrFrameBeginInfo fbi = { XR_TYPE_FRAME_BEGIN_INFO };
        if (XR_FAILED(xrBeginFrame(session, &fbi))) { SDL_Log("c: xrBeginFrame"); abort(); }
        XrViewLocateInfo vli = { XR_TYPE_VIEW_LOCATE_INFO };
        vli.viewConfigurationType = XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO;
        vli.displayTime = fs.predictedDisplayTime;
        vli.space = space;
        XrViewState vs = { XR_TYPE_VIEW_STATE };
        XrView views[2] = { { XR_TYPE_VIEW }, { XR_TYPE_VIEW } };
        Uint32 nv = 0;
        xrLocateViews(session, &vli, &vs, 2, &nv, views);

        bool traced_frame = state == XR_SESSION_STATE_FOCUSED && traced < TRACED_FRAMES;
        Uint32 frame = traced_frame ? traced : 0;

        Uint32 idx[2];
        for (int k = 0; k < 2; k++) {
            XrSwapchainImageAcquireInfo ai = { XR_TYPE_SWAPCHAIN_IMAGE_ACQUIRE_INFO };
            XrSwapchainImageWaitInfo wi = { XR_TYPE_SWAPCHAIN_IMAGE_WAIT_INFO };
            wi.timeout = XR_INFINITE_DURATION;
            if (xrAcquireSwapchainImage(sc[k], &ai, &idx[k]) != XR_SUCCESS) { SDL_Log("c: acquire"); abort(); }
            if (xrWaitSwapchainImage(sc[k], &wi) != XR_SUCCESS) { SDL_Log("c: wait"); abort(); }
        }

        SDL_GPUCommandBuffer *cb = SDL_AcquireGPUCommandBuffer(dev);
        for (int k = 0; k < 2; k++) {
            for (Uint32 layer = 0; layer < nlayers[k]; layer++) {
                SDL_GPUColorTargetInfo ct;
                SDL_zero(ct);
                ct.texture = tex[k][idx[k]];
                ct.layer_or_depth_plane = layer;
                ct.clear_color = clear_color(k, (int)layer, (int)frame);
                ct.load_op = SDL_GPU_LOADOP_CLEAR;
                ct.store_op = SDL_GPU_STOREOP_STORE;
                SDL_GPURenderPass *rp = SDL_BeginGPURenderPass(cb, &ct, 1, NULL);
                if (rp) SDL_EndGPURenderPass(rp);
                else t("render pass %d %u: error: %s", k, layer, SDL_GetError());
            }
        }
        {
            SDL_GPUCopyPass *cp = SDL_BeginGPUCopyPass(cb);
            const int rk[3] = { 0, 1, 1 };
            const Uint32 rl[3] = { 0, 0, 1 }, ro[3] = { 0, one, one + two }, rs[3] = { 64, 32, 32 };
            for (int q = 0; q < 3; q++) {
                SDL_GPUTextureRegion reg;
                SDL_zero(reg);
                reg.texture = tex[rk[q]][idx[rk[q]]];
                reg.layer = rl[q]; reg.w = rs[q]; reg.h = rs[q]; reg.d = 1;
                SDL_GPUTextureTransferInfo ti;
                SDL_zero(ti);
                ti.transfer_buffer = tb; ti.offset = ro[q];
                SDL_DownloadFromGPUTexture(cp, &reg, &ti);
            }
            SDL_EndGPUCopyPass(cp);
        }
        SDL_GPUFence *fence = SDL_SubmitGPUCommandBufferAndAcquireFence(cb);
        SDL_WaitForGPUFences(dev, true, &fence, 1);
        SDL_ReleaseGPUFence(dev, fence);
        if (traced_frame) {
            Uint8 *m = SDL_MapGPUTransferBuffer(dev, tb, false);
            t("frame %u: one layer %s %s; two layers %s %s, %s %s", frame,
               pixel(m), pixel(m + one - 4), pixel(m + one), pixel(m + one + two - 4),
               pixel(m + one + two), pixel(m + one + two + two - 4));
            SDL_UnmapGPUTransferBuffer(dev, tb);
        }

        for (int k = 0; k < 2; k++) {
            XrSwapchainImageReleaseInfo ri = { XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO };
            if (xrReleaseSwapchainImage(sc[k], &ri) != XR_SUCCESS) { SDL_Log("c: release"); abort(); }
        }

        XrCompositionLayerProjectionView pv[2];
        for (int e = 0; e < 2; e++) {
            memset(&pv[e], 0, sizeof(pv[e]));
            pv[e].type = XR_TYPE_COMPOSITION_LAYER_PROJECTION_VIEW;
            pv[e].pose = views[e].pose;
            pv[e].fov = views[e].fov;
            pv[e].subImage.swapchain = sc[1];
            pv[e].subImage.imageRect.extent.width = 32;
            pv[e].subImage.imageRect.extent.height = 32;
            pv[e].subImage.imageArrayIndex = (Uint32)e;
        }
        XrCompositionLayerProjection layer = { XR_TYPE_COMPOSITION_LAYER_PROJECTION };
        layer.space = space; layer.viewCount = 2; layer.views = pv;
        const XrCompositionLayerBaseHeader *lp = (const XrCompositionLayerBaseHeader *)&layer;
        XrFrameEndInfo fei = { XR_TYPE_FRAME_END_INFO };
        fei.displayTime = fs.predictedDisplayTime;
        fei.environmentBlendMode = XR_ENVIRONMENT_BLEND_MODE_OPAQUE;
        fei.layerCount = fs.shouldRender ? 1 : 0;
        fei.layers = &lp;
        if (xrEndFrame(session, &fei) != XR_SUCCESS) { SDL_Log("c: xrEndFrame"); abort(); }

        if (traced_frame) {
            traced++;
            if (traced == TRACED_FRAMES && !exit_requested) {
                t("request exit: %d", (int)xrRequestExitSession(session));
                exit_requested = true;
            }
        }
    }
    t("session done: %s", done ? "true" : "false");

    for (int k = 0; k < 2; k++) {
        SDL_ClearError();
        r = SDL_DestroyGPUXRSwapchain(dev, sc[k], tex[k]);
        if (r == XR_SUCCESS) t("destroy swapchain %s: ok", names[k]);
        else t("destroy swapchain %s: %s", names[k], xr_err(r));
    }
    SDL_ReleaseGPUTransferBuffer(dev, tb);
    t("destroy space: %d", (int)xrDestroySpace(space));
    t("destroy session: %d", (int)xrDestroySession(session));
    SDL_DestroyGPUDevice(dev);
    t("device destroyed");
    t("destroy instance: %d", (int)xrDestroyInstance(inst));
}

int main(int argc, char **argv)
{
    (void)argc;
    (void)argv;
    start_logs();
    SDL_SetAssertionHandler(assertion, NULL);
    /* (before the runtime is loaded: it reads the list once) */
    setenv("SDL3_OPENXR_TEST_RUNTIME_FAIL", FAILURES, 1);
    if (!SDL_Init(SDL_INIT_VIDEO)) {
        t("init: error: %s", SDL_GetError());
        SDL_Log("sdl3-android-runner: exit 1");
        return 1;
    }
    t("init: ok (%s)", SDL_GetCurrentVideoDriver());

    /* no loader */
    t("xrGetInstanceProcAddr: %s", result(SDL_OpenXR_GetXrGetInstanceProcAddr() != NULL));
    t("load library: %s", result(SDL_OpenXR_LoadLibrary()));
    t("supports xr: %s", yes(supports_xr()));
    failed_device("no loader");

    /* the runtime in the loader's place */
    SDL_SetHint(SDL_HINT_OPENXR_LIBRARY, RUNTIME);
    SDL_SharedObject *held = SDL_LoadObject(RUNTIME);
    t("the program's reference: %s", result(held != NULL));
    failed_device("lookup fails");
    failed_device("initialization fails");
    t("load library: %s", result(SDL_OpenXR_LoadLibrary()));
    t("xrGetInstanceProcAddr: %s", result(SDL_OpenXR_GetXrGetInstanceProcAddr() != NULL));
    SDL_OpenXR_UnloadLibrary();
    t("library unloaded");
    t("xrGetInstanceProcAddr after the unload: %s", result(SDL_OpenXR_GetXrGetInstanceProcAddr() != NULL));
    SDL_SetHint(SDL_HINT_OPENXR_LIBRARY, "libsdl3_test_no_such_library.so");
    t("load library with another hint: %s", result(SDL_OpenXR_LoadLibrary()));
    SDL_OpenXR_UnloadLibrary();
    t("library unloaded");

    main_scenario();
    if (held) {
        SDL_UnloadObject(held);
    }

    SDL_Quit();
    t("quit");
    SDL_Log("sdl3-android-runner: exit 0");
    return 0;
}
