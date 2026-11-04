/* diag only (never committed to a PR branch): upstream SDL's OpenXR API at
 * ea7a2da on Monado's null compositor (Vulkan on lavapipe): the device in
 * debug mode, HANDHELD, the formats list, COLOR_TARGET-only and
 * TRANSFER_SRC swapchains, renders and downloads of each eye's layer, and
 * the XR functions on a device without XR (in a child process). */
#include <SDL3/SDL.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>
#include <vulkan/vulkan.h>
#define XR_USE_GRAPHICS_API_VULKAN 1
#include <openxr/openxr.h>
#include <openxr/openxr_platform.h>
#include <SDL3/SDL_openxr.h>

static PFN_xrGetInstanceProcAddr gipa;
#define LOADI(inst, name) PFN_##name name = NULL; gipa(inst, #name, (PFN_xrVoidFunction *)&name)
#define CHK(r, what) do { XrResult _r = (r); printf("%-36s %d\n", what, (int)_r); if (XR_FAILED(_r)) return 1; } while (0)

static void SDLCALL log_out(void *u, int category, SDL_LogPriority priority, const char *message)
{
    (void)u;
    printf("log[%d/%d]: %s\n", category, (int)priority, message);
    fflush(stdout);
}

static SDL_GPUDevice *make_device(int handheld, int debug, XrInstance *inst, XrSystemId *sys)
{
    SDL_PropertiesID props = SDL_CreateProperties();
    SDL_SetBooleanProperty(props, SDL_PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN, true);
    SDL_SetBooleanProperty(props, SDL_PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN, debug);
    SDL_SetBooleanProperty(props, SDL_PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN, true);
    SDL_SetPointerProperty(props, SDL_PROP_GPU_DEVICE_CREATE_XR_INSTANCE_POINTER, inst);
    SDL_SetPointerProperty(props, SDL_PROP_GPU_DEVICE_CREATE_XR_SYSTEM_ID_POINTER, sys);
    if (handheld) {
        SDL_SetNumberProperty(props, SDL_PROP_GPU_DEVICE_CREATE_XR_FORM_FACTOR_NUMBER, XR_FORM_FACTOR_HANDHELD_DISPLAY);
    }
    SDL_GPUDevice *d = SDL_CreateGPUDeviceWithProperties(props);
    SDL_DestroyProperties(props);
    return d;
}

static void no_xr_child(int which)
{
    fflush(stdout);
    pid_t pid = fork();
    if (pid == 0) {
        SDL_GPUDevice *d = SDL_CreateGPUDevice(SDL_GPU_SHADERFORMAT_SPIRV, false, NULL);
        if (!d) { printf("no-xr device: %s\n", SDL_GetError()); _exit(3); }
        int n = -1;
        XrSession s = XR_NULL_HANDLE;
        XrSessionCreateInfo sci = { XR_TYPE_SESSION_CREATE_INFO };
        printf("no-xr %d calling\n", which); fflush(stdout);
        if (which == 0) printf("no-xr CreateGPUXRSession %d\n", (int)SDL_CreateGPUXRSession(d, &sci, &s));
        if (which == 1) printf("no-xr GetGPUXRSwapchainFormats %p %d\n", (void *)SDL_GetGPUXRSwapchainFormats(d, (XrSession)(uintptr_t)1, &n), n);
        if (which == 2) { XrSwapchain sc; SDL_GPUTexture **t; XrSwapchainCreateInfo c = { XR_TYPE_SWAPCHAIN_CREATE_INFO }; printf("no-xr CreateGPUXRSwapchain %d\n", (int)SDL_CreateGPUXRSwapchain(d, (XrSession)(uintptr_t)1, &c, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, &sc, &t)); }
        if (which == 3) { SDL_GPUTexture *t[1] = { NULL }; printf("no-xr DestroyGPUXRSwapchain %d\n", (int)SDL_DestroyGPUXRSwapchain(d, (XrSwapchain)(uintptr_t)1, t)); }
        fflush(stdout);
        _exit(0);
    }
    int st = 0;
    waitpid(pid, &st, 0);
    printf("no-xr %d child: %s %d\n", which, WIFSIGNALED(st) ? "signal" : "exit", WIFSIGNALED(st) ? WTERMSIG(st) : WEXITSTATUS(st));
}

int main(int argc, char **argv)
{
    (void)argc; (void)argv;
    setvbuf(stdout, NULL, _IOLBF, 0);
    SDL_SetLogOutputFunction(log_out, NULL);
    SDL_SetLogPriority(SDL_LOG_CATEGORY_GPU, SDL_LOG_PRIORITY_VERBOSE);
    if (!SDL_Init(SDL_INIT_VIDEO)) { printf("init: %s\n", SDL_GetError()); return 1; }
    printf("video driver %s\n", SDL_GetCurrentVideoDriver());

    for (int i = 0; i < 4; i++) no_xr_child(i);

    XrInstance inst = XR_NULL_HANDLE;
    XrSystemId sys = XR_NULL_SYSTEM_ID;
    SDL_GPUDevice *hh = make_device(1, 0, &inst, &sys);
    printf("HANDHELD device %p: %s (instance %d system %d)\n", (void *)hh, hh ? "" : SDL_GetError(), inst != 0, sys != 0);
    if (hh) SDL_DestroyGPUDevice(hh);

    inst = XR_NULL_HANDLE; sys = XR_NULL_SYSTEM_ID;
    SDL_GPUDevice *dev = make_device(0, 1, &inst, &sys);
    if (!dev) { printf("device: %s\n", SDL_GetError()); return 1; }
    printf("device %s instance %d system %d\n", SDL_GetGPUDeviceDriver(dev), inst != 0, sys != 0);

    gipa = (PFN_xrGetInstanceProcAddr)SDL_OpenXR_GetXrGetInstanceProcAddr();
    LOADI(inst, xrGetSystem); LOADI(inst, xrPollEvent); LOADI(inst, xrBeginSession); LOADI(inst, xrWaitFrame);
    LOADI(inst, xrBeginFrame); LOADI(inst, xrEndFrame); LOADI(inst, xrAcquireSwapchainImage);
    LOADI(inst, xrWaitSwapchainImage); LOADI(inst, xrReleaseSwapchainImage); LOADI(inst, xrCreateReferenceSpace);
    LOADI(inst, xrLocateViews); LOADI(inst, xrRequestExitSession); LOADI(inst, xrEndSession);
    LOADI(inst, xrDestroySession); LOADI(inst, xrDestroyInstance); LOADI(inst, xrDestroySpace);
    LOADI(inst, xrEnumerateSwapchainImages);

    XrSystemGetInfo sgi = { XR_TYPE_SYSTEM_GET_INFO };
    sgi.formFactor = XR_FORM_FACTOR_HANDHELD_DISPLAY;
    XrSystemId hsys = 0;
    printf("xrGetSystem(HANDHELD) %d\n", (int)xrGetSystem(inst, &sgi, &hsys));

    XrSessionCreateInfo sci = { XR_TYPE_SESSION_CREATE_INFO };
    XrSession session = XR_NULL_HANDLE;
    CHK(SDL_CreateGPUXRSession(dev, &sci, &session), "SDL_CreateGPUXRSession");

    int nf = -1;
    SDL_GPUTextureFormat *fmts = SDL_GetGPUXRSwapchainFormats(dev, session, &nf);
    printf("formats %d:", nf);
    for (int i = 0; fmts && i <= nf; i++) printf(" %d", (int)fmts[i]);
    printf("\n");
    SDL_free(fmts);

    /* swapchain 0: COLOR_ATTACHMENT only; 1: + TRANSFER_SRC */
    XrSwapchain sc[2];
    SDL_GPUTexture **tex[2];
    uint32_t nimg[2];
    for (int k = 0; k < 2; k++) {
        XrSwapchainCreateInfo c = { XR_TYPE_SWAPCHAIN_CREATE_INFO };
        c.usageFlags = XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT | (k ? XR_SWAPCHAIN_USAGE_TRANSFER_SRC_BIT : 0);
        c.sampleCount = 1; c.width = 64; c.height = 64; c.faceCount = 1; c.arraySize = 2; c.mipCount = 1;
        CHK(SDL_CreateGPUXRSwapchain(dev, session, &c, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, &sc[k], &tex[k]), k ? "SDL_CreateGPUXRSwapchain (+TRANSFER_SRC)" : "SDL_CreateGPUXRSwapchain (COLOR only)");
        xrEnumerateSwapchainImages(sc[k], 0, &nimg[k], NULL);
        printf("  images %u\n", nimg[k]);
    }

    XrReferenceSpaceCreateInfo rsci = { XR_TYPE_REFERENCE_SPACE_CREATE_INFO };
    rsci.referenceSpaceType = XR_REFERENCE_SPACE_TYPE_LOCAL;
    rsci.poseInReferenceSpace.orientation.w = 1;
    XrSpace space;
    CHK(xrCreateReferenceSpace(session, &rsci, &space), "xrCreateReferenceSpace");

    SDL_GPUTransferBufferCreateInfo tbi = { SDL_GPU_TRANSFERBUFFERUSAGE_DOWNLOAD, 64 * 64 * 4 * 4, 0 };
    SDL_GPUTransferBuffer *tb = SDL_CreateGPUTransferBuffer(dev, &tbi);

    int running = 0, frames = 0, done = 0;
    for (int iter = 0; iter < 4000 && !done; iter++) {
        XrEventDataBuffer ev = { XR_TYPE_EVENT_DATA_BUFFER };
        while (xrPollEvent(inst, &ev) == XR_SUCCESS) {
            if (ev.type == XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED) {
                XrSessionState st = ((XrEventDataSessionStateChanged *)&ev)->state;
                printf("state %d\n", (int)st);
                if (st == XR_SESSION_STATE_READY) {
                    XrSessionBeginInfo sbi = { XR_TYPE_SESSION_BEGIN_INFO };
                    sbi.primaryViewConfigurationType = XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO;
                    CHK(xrBeginSession(session, &sbi), "xrBeginSession");
                    running = 1;
                } else if (st == XR_SESSION_STATE_STOPPING) {
                    CHK(xrEndSession(session), "xrEndSession");
                    running = 0;
                } else if (st == XR_SESSION_STATE_EXITING || st == XR_SESSION_STATE_LOSS_PENDING) {
                    done = 1;
                }
            }
            ev.type = XR_TYPE_EVENT_DATA_BUFFER;
        }
        if (!running) continue;
        XrFrameWaitInfo fwi = { XR_TYPE_FRAME_WAIT_INFO };
        XrFrameState fs = { XR_TYPE_FRAME_STATE };
        if (XR_FAILED(xrWaitFrame(session, &fwi, &fs))) { printf("xrWaitFrame failed\n"); return 1; }
        XrFrameBeginInfo fbi = { XR_TYPE_FRAME_BEGIN_INFO };
        if (XR_FAILED(xrBeginFrame(session, &fbi))) { printf("xrBeginFrame failed\n"); return 1; }
        XrViewLocateInfo vli = { XR_TYPE_VIEW_LOCATE_INFO };
        vli.viewConfigurationType = XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO; vli.displayTime = fs.predictedDisplayTime; vli.space = space;
        XrViewState vs = { XR_TYPE_VIEW_STATE };
        XrView views[2] = { { XR_TYPE_VIEW }, { XR_TYPE_VIEW } };
        uint32_t nv = 0;
        xrLocateViews(session, &vli, &vs, 2, &nv, views);
        uint32_t idx[2];
        for (int k = 0; k < 2; k++) {
            XrSwapchainImageAcquireInfo sai = { XR_TYPE_SWAPCHAIN_IMAGE_ACQUIRE_INFO };
            xrAcquireSwapchainImage(sc[k], &sai, &idx[k]);
            XrSwapchainImageWaitInfo swi = { XR_TYPE_SWAPCHAIN_IMAGE_WAIT_INFO };
            swi.timeout = XR_INFINITE_DURATION;
            xrWaitSwapchainImage(sc[k], &swi);
        }
        SDL_GPUCommandBuffer *cb = SDL_AcquireGPUCommandBuffer(dev);
        for (int k = 0; k < 2; k++) {
            for (int e = 0; e < 2; e++) {
                SDL_GPUColorTargetInfo ct;
                SDL_zero(ct);
                ct.texture = tex[k][idx[k]];
                ct.layer_or_depth_plane = (Uint32)e;
                ct.clear_color.r = e ? 0.0f : 1.0f; ct.clear_color.g = e ? 1.0f : 0.0f;
                ct.clear_color.b = (float)(frames % 4) / 4.0f; ct.clear_color.a = 1.0f;
                ct.load_op = SDL_GPU_LOADOP_CLEAR; ct.store_op = SDL_GPU_STOREOP_STORE;
                SDL_GPURenderPass *rp = SDL_BeginGPURenderPass(cb, &ct, 1, NULL);
                if (!rp) printf("render pass %d/%d: %s\n", k, e, SDL_GetError());
                else SDL_EndGPURenderPass(rp);
            }
        }
        SDL_GPUCopyPass *cp = SDL_BeginGPUCopyPass(cb);
        for (int k = 0; k < 2; k++) {
            for (int e = 0; e < 2; e++) {
                SDL_GPUTextureRegion r;
                SDL_zero(r);
                r.texture = tex[k][idx[k]]; r.layer = (Uint32)e; r.w = 64; r.h = 64; r.d = 1;
                SDL_GPUTextureTransferInfo ti;
                SDL_zero(ti);
                ti.transfer_buffer = tb; ti.offset = (Uint32)((k * 2 + e) * 64 * 64 * 4);
                SDL_DownloadFromGPUTexture(cp, &r, &ti);
            }
        }
        SDL_EndGPUCopyPass(cp);
        SDL_GPUFence *fence = SDL_SubmitGPUCommandBufferAndAcquireFence(cb);
        SDL_WaitForGPUFences(dev, true, &fence, 1);
        SDL_ReleaseGPUFence(dev, fence);
        Uint8 *p = SDL_MapGPUTransferBuffer(dev, tb, false);
        if (frames < 3 || frames == 9) {
            for (int q = 0; q < 4; q++) {
                Uint8 *px = p + q * 64 * 64 * 4;
                printf("frame %d swapchain %d (%s) image %u layer %d: %u %u %u %u / last %u %u %u %u\n", frames, q / 2, q / 2 ? "+TRANSFER_SRC" : "COLOR only", idx[q / 2], q % 2,
                       px[0], px[1], px[2], px[3], px[64 * 64 * 4 - 4], px[64 * 64 * 4 - 3], px[64 * 64 * 4 - 2], px[64 * 64 * 4 - 1]);
            }
        }
        SDL_UnmapGPUTransferBuffer(dev, tb);
        for (int k = 0; k < 2; k++) {
            XrSwapchainImageReleaseInfo sri = { XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO };
            xrReleaseSwapchainImage(sc[k], &sri);
        }
        XrCompositionLayerProjectionView pv[2];
        for (int e = 0; e < 2; e++) {
            memset(&pv[e], 0, sizeof(pv[e]));
            pv[e].type = XR_TYPE_COMPOSITION_LAYER_PROJECTION_VIEW;
            pv[e].pose = views[e].pose; pv[e].fov = views[e].fov;
            pv[e].subImage.swapchain = sc[1]; pv[e].subImage.imageRect.extent.width = 64; pv[e].subImage.imageRect.extent.height = 64;
            pv[e].subImage.imageArrayIndex = (uint32_t)e;
        }
        XrCompositionLayerProjection layer = { XR_TYPE_COMPOSITION_LAYER_PROJECTION };
        layer.space = space; layer.viewCount = 2; layer.views = pv;
        const XrCompositionLayerBaseHeader *lp = (XrCompositionLayerBaseHeader *)&layer;
        XrFrameEndInfo fei = { XR_TYPE_FRAME_END_INFO };
        fei.displayTime = fs.predictedDisplayTime; fei.environmentBlendMode = XR_ENVIRONMENT_BLEND_MODE_OPAQUE;
        fei.layerCount = fs.shouldRender ? 1 : 0; fei.layers = &lp;
        XrResult er = xrEndFrame(session, &fei);
        if (XR_FAILED(er)) { printf("xrEndFrame %d\n", er); return 1; }
        if (++frames == 10) CHK(xrRequestExitSession(session), "xrRequestExitSession");
    }
    printf("frames %d done %d\n", frames, done);
    SDL_ReleaseGPUTransferBuffer(dev, tb);
    CHK(xrDestroySpace(space), "xrDestroySpace");
    {
        /* a texture that isn't an XR swapchain's: upstream's "Invalid GPU Texture handle." (stops half-way, B10) */
        SDL_GPUTextureCreateInfo ti = { SDL_GPU_TEXTURETYPE_2D, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET, 4, 4, 1, 1, SDL_GPU_SAMPLECOUNT_1, 0 };
        SDL_GPUTexture *plain = SDL_CreateGPUTexture(dev, &ti);
        SDL_GPUTexture *list[16];
        for (int q = 0; q < 16; q++) list[q] = plain;
        SDL_ClearError();
        printf("DestroyGPUXRSwapchain(null swapchain) %d: %s\n", (int)SDL_DestroyGPUXRSwapchain(dev, XR_NULL_HANDLE, list), SDL_GetError());
        SDL_ClearError();
        printf("DestroyGPUXRSwapchain(non-XR textures) %d: %s\n", (int)SDL_DestroyGPUXRSwapchain(dev, sc[0], list), SDL_GetError());
        SDL_ReleaseGPUTexture(dev, plain);
    }
    for (int k = 0; k < 2; k++) CHK(SDL_DestroyGPUXRSwapchain(dev, sc[k], tex[k]), "SDL_DestroyGPUXRSwapchain");
    CHK(xrDestroySession(session), "xrDestroySession");
    SDL_DestroyGPUDevice(dev);
    CHK(xrDestroyInstance(inst), "xrDestroyInstance");
    SDL_Quit();
    printf("ok\n");
    return 0;
}
