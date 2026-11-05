/* diag only (never committed to a PR branch): shared by the C twins of
   sdl3/tests/android_openxr.rs and android_openxr_loader.rs, on upstream's
   SDL 3.5.0 (ea7a2da), built as upstream's android-project's app with
   Gradle and CMake, run by tools/android_run.py --run-apk. Trace lines go
   to logcat's SDL/APP through SDL_Log(); the GPU category's log lines are
   traced too (as the Rust's `log N: MESSAGE`). */
#include <SDL3/SDL.h>
#include <SDL3/SDL_main.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../SDL/src/video/khronos/openxr/openxr.h"
#include <SDL3/SDL_openxr.h>

static void t(const char *fmt, ...) SDL_PRINTF_VARARG_FUNC(1);
static void t(const char *fmt, ...)
{
    char buf[4000];
    va_list ap;
    va_start(ap, fmt);
    SDL_vsnprintf(buf, sizeof(buf), fmt, ap);
    va_end(ap);
    SDL_Log("trace: %s", buf);
}

static const char *yes(bool b)
{
    return b ? "yes" : "no";
}

static const char *result(bool ok)
{
    static char buf[4][1024];
    static int i = 0;
    i = (i + 1) % 4;
    if (ok) {
        return "ok";
    }
    SDL_snprintf(buf[i], sizeof(buf[i]), "error: %s", SDL_GetError());
    return buf[i];
}

/* an entry point's error: the result, and SDL's error when there's one */
static const char *xr_err(XrResult r)
{
    static char buf[4][512];
    static int i = 0;
    i = (i + 1) % 4;
    if (*SDL_GetError()) SDL_snprintf(buf[i], sizeof(buf[i]), "error %d: %s", (int)r, SDL_GetError());
    else SDL_snprintf(buf[i], sizeof(buf[i]), "error %d", (int)r);
    return buf[i];
}

static SDL_LogOutputFunction default_output;

/* the GPU category's lines traced, the others as they are */
static void SDLCALL log_output(void *userdata, int category, SDL_LogPriority priority, const char *message)
{
    (void)userdata;
    if (category == SDL_LOG_CATEGORY_GPU) {
        char buf[4096];
        SDL_snprintf(buf, sizeof(buf), "trace: log %d: %s", (int)priority, message);
        default_output(NULL, SDL_LOG_CATEGORY_APPLICATION, SDL_LOG_PRIORITY_INFO, buf);
    } else {
        default_output(NULL, category, priority, message);
    }
}

static void start_logs(void)
{
    default_output = SDL_GetDefaultLogOutputFunction();
    SDL_SetLogOutputFunction(log_output, NULL);
    SDL_SetLogPriority(SDL_LOG_CATEGORY_GPU, SDL_LOG_PRIORITY_VERBOSE);
}

static XrInstance xr_instance = XR_NULL_HANDLE;
static XrSystemId xr_system = XR_NULL_SYSTEM_ID;

/* an OpenXR device's properties: SPIR-V, no debug mode, no verbose logs */
static SDL_PropertiesID xr_props(void)
{
    SDL_PropertiesID props = SDL_CreateProperties();
    SDL_SetBooleanProperty(props, SDL_PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN, true);
    SDL_SetBooleanProperty(props, SDL_PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN, false);
    SDL_SetBooleanProperty(props, SDL_PROP_GPU_DEVICE_CREATE_VERBOSE_BOOLEAN, false);
    SDL_SetBooleanProperty(props, SDL_PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN, true);
    SDL_SetPointerProperty(props, SDL_PROP_GPU_DEVICE_CREATE_XR_INSTANCE_POINTER, &xr_instance);
    SDL_SetPointerProperty(props, SDL_PROP_GPU_DEVICE_CREATE_XR_SYSTEM_ID_POINTER, &xr_system);
    return props;
}

static bool supports_xr(void)
{
    SDL_PropertiesID p = xr_props();
    bool r = SDL_GPUSupportsProperties(p);
    SDL_DestroyProperties(p);
    return r;
}
