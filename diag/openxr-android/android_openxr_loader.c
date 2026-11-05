/* diag only: the C twin of sdl3/tests/android_openxr_loader.rs: the
   Khronos OpenXR loader for Android (libopenxr_loader.so in the app's
   jniLibs) and no runtime, on upstream's SDL. */
#include "common.h"

int main(int argc, char **argv)
{
    (void)argc;
    (void)argv;
    start_logs();
    if (!SDL_Init(SDL_INIT_VIDEO)) {
        t("init: error: %s", SDL_GetError());
        SDL_Log("sdl3-android-runner: exit 1");
        return 1;
    }
    t("init: ok (%s)", SDL_GetCurrentVideoDriver());

    t("xrGetInstanceProcAddr: %s", result(SDL_OpenXR_GetXrGetInstanceProcAddr() != NULL));
    t("supports xr: %s", yes(supports_xr()));
    {
        SDL_PropertiesID p = xr_props();
        SDL_ClearError();
        SDL_GPUDevice *dev = SDL_CreateGPUDeviceWithProperties(p);
        SDL_DestroyProperties(p);
        if (dev) {
            t("device: ok (%s)", SDL_GetGPUDeviceDriver(dev));
            SDL_DestroyGPUDevice(dev);
        } else {
            /* (no error set there: not traced) */
            t("device: failed");
            SDL_Log("c: device: '%s'", SDL_GetError());
        }
    }

    t("load library: %s", result(SDL_OpenXR_LoadLibrary()));
    PFN_xrGetInstanceProcAddr gipa = (PFN_xrGetInstanceProcAddr)SDL_OpenXR_GetXrGetInstanceProcAddr();
    if (gipa) {
        t("xrGetInstanceProcAddr: ok");
        PFN_xrEnumerateApiLayerProperties layers = NULL;
        XrResult r = gipa(XR_NULL_HANDLE, "xrEnumerateApiLayerProperties", (PFN_xrVoidFunction *)&layers);
        if (layers) {
            Uint32 count = 0;
            r = layers(0, &count, NULL);
            t("xrEnumerateApiLayerProperties: %d, %u layers", (int)r, count);
        } else {
            t("xrEnumerateApiLayerProperties: lookup %d", (int)r);
        }
        PFN_xrEnumerateInstanceExtensionProperties exts = NULL;
        r = gipa(XR_NULL_HANDLE, "xrEnumerateInstanceExtensionProperties", (PFN_xrVoidFunction *)&exts);
        if (exts) {
            Uint32 count = 0;
            r = exts(NULL, 0, &count, NULL);
            t("xrEnumerateInstanceExtensionProperties: %d", (int)r);
        } else {
            t("xrEnumerateInstanceExtensionProperties: lookup %d", (int)r);
        }
    } else {
        t("xrGetInstanceProcAddr: error: %s", SDL_GetError());
    }
    SDL_OpenXR_UnloadLibrary();
    t("library unloaded");
    t("xrGetInstanceProcAddr after the unload: %s", result(SDL_OpenXR_GetXrGetInstanceProcAddr() != NULL));

    SDL_Quit();
    t("quit");
    SDL_Log("sdl3-android-runner: exit 0");
    return 0;
}
