/* Upstream SDL3 reference trace on the iOS simulator: a window, the
   displays, a renderer and the GPU API; ends with the runner sentinel. */
#include <SDL3/SDL.h>
#include <SDL3/SDL_main.h>
#include <stdio.h>
#include <stdlib.h>

static void P(const char *fmt, ...) SDL_PRINTF_VARARG_FUNC(1);
static void P(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    printf("[c] ");
    vprintf(fmt, ap);
    printf("\n");
    fflush(stdout);
    va_end(ap);
}

int main(int argc, char *argv[])
{
    int i, j, n = 0;
    P("main argc=%d argv0=%s platform=%s version=%d revision=%s", argc, argv[0], SDL_GetPlatform(), SDL_GetVersion(), SDL_GetRevision());
    for (i = 1; i < argc; i++) P("arg %d: %s", i, argv[i]);
    P("getenv SPIKE_VAR=%s", getenv("SPIKE_VAR") ? getenv("SPIKE_VAR") : "(unset)");
    P("SDL_GetBasePath=%s SDL_GetPrefPath=%s", SDL_GetBasePath(), SDL_GetPrefPath("org", "app"));
    P("SDL_GetUserFolder(HOME)=%s", SDL_GetUserFolder(SDL_FOLDER_HOME));
    for (i = 0; i < SDL_GetNumVideoDrivers(); i++) P("video driver %d: %s", i, SDL_GetVideoDriver(i));
    if (!SDL_Init(SDL_INIT_VIDEO | SDL_INIT_GAMEPAD | SDL_INIT_AUDIO)) {
        P("SDL_Init failed: %s", SDL_GetError());
    }
    P("current video driver: %s; audio driver: %s", SDL_GetCurrentVideoDriver(), SDL_GetCurrentAudioDriver());
    {
        SDL_PowerState ps; int secs, pct;
        ps = SDL_GetPowerInfo(&secs, &pct);
        P("power: state=%d secs=%d pct=%d", (int)ps, secs, pct);
    }
    SDL_DisplayID *displays = SDL_GetDisplays(&n);
    P("displays: %d (primary %u)", n, SDL_GetPrimaryDisplay());
    for (i = 0; i < n; i++) {
        SDL_Rect b, u;
        const SDL_DisplayMode *cur = SDL_GetCurrentDisplayMode(displays[i]);
        const SDL_DisplayMode *desk = SDL_GetDesktopDisplayMode(displays[i]);
        SDL_DisplayMode **modes;
        int nm = 0;
        SDL_GetDisplayBounds(displays[i], &b);
        SDL_GetDisplayUsableBounds(displays[i], &u);
        P("display %u name=%s bounds=%d,%d %dx%d usable=%d,%d %dx%d content_scale=%g natural_orientation=%d current_orientation=%d",
          displays[i], SDL_GetDisplayName(displays[i]), b.x, b.y, b.w, b.h, u.x, u.y, u.w, u.h,
          SDL_GetDisplayContentScale(displays[i]), (int)SDL_GetNaturalDisplayOrientation(displays[i]),
          (int)SDL_GetCurrentDisplayOrientation(displays[i]));
        if (cur) P("  current mode: %dx%d fmt=%s density=%g refresh=%g (%d/%d)", cur->w, cur->h, SDL_GetPixelFormatName(cur->format), cur->pixel_density, cur->refresh_rate, cur->refresh_rate_numerator, cur->refresh_rate_denominator);
        if (desk) P("  desktop mode: %dx%d density=%g refresh=%g", desk->w, desk->h, desk->pixel_density, desk->refresh_rate);
        modes = SDL_GetFullscreenDisplayModes(displays[i], &nm);
        for (j = 0; j < nm; j++) P("  fullscreen mode %d: %dx%d density=%g refresh=%g", j, modes[j]->w, modes[j]->h, modes[j]->pixel_density, modes[j]->refresh_rate);
        SDL_free(modes);
    }
    SDL_free(displays);

    SDL_Window *w = SDL_CreateWindow("spike", 0, 0, SDL_WINDOW_HIGH_PIXEL_DENSITY);
    if (!w) {
        P("SDL_CreateWindow failed: %s", SDL_GetError());
    } else {
        int ww, wh, pw, ph;
        SDL_Rect sa;
        SDL_GetWindowSize(w, &ww, &wh);
        SDL_GetWindowSizeInPixels(w, &pw, &ph);
        SDL_GetWindowSafeArea(w, &sa);
        P("window: size=%dx%d pixels=%dx%d density=%g display_scale=%g flags=0x%llx safe_area=%d,%d %dx%d",
          ww, wh, pw, ph, SDL_GetWindowPixelDensity(w), SDL_GetWindowDisplayScale(w),
          (unsigned long long)SDL_GetWindowFlags(w), sa.x, sa.y, sa.w, sa.h);
        for (i = 0; i < SDL_GetNumRenderDrivers(); i++) P("render driver %d: %s", i, SDL_GetRenderDriver(i));
        SDL_Renderer *r = SDL_CreateRenderer(w, NULL);
        if (r) {
            int ow, oh;
            SDL_GetRenderOutputSize(r, &ow, &oh);
            P("renderer: %s output=%dx%d", SDL_GetRendererName(r), ow, oh);
            SDL_SetRenderDrawColor(r, 30, 60, 200, 255);
            SDL_RenderClear(r);
            SDL_RenderPresent(r);
            SDL_DestroyRenderer(r);
        } else {
            P("SDL_CreateRenderer failed: %s", SDL_GetError());
        }
        {
            SDL_Renderer *g = SDL_CreateRenderer(w, "opengles2");
            P("opengles2 renderer: %s", g ? SDL_GetRendererName(g) : SDL_GetError());
            if (g) SDL_DestroyRenderer(g);
        }
    }
    for (i = 0; i < SDL_GetNumGPUDrivers(); i++) P("gpu driver %d: %s", i, SDL_GetGPUDriver(i));
    {
        SDL_GPUDevice *d = SDL_CreateGPUDevice(SDL_GPU_SHADERFORMAT_MSL | SDL_GPU_SHADERFORMAT_METALLIB, false, NULL);
        P("SDL_CreateGPUDevice: %s", d ? SDL_GetGPUDeviceDriver(d) : SDL_GetError());
        if (d) SDL_DestroyGPUDevice(d);
    }
    {
        int nj = 0;
        SDL_JoystickID *js = SDL_GetJoysticks(&nj);
        P("joysticks: %d", nj);
        SDL_free(js);
    }
    {
        Uint64 start = SDL_GetTicks();
        int frames = 0;
        while (SDL_GetTicks() - start < 1500) {
            SDL_Event e;
            while (SDL_PollEvent(&e)) {
                char buf[256];
                SDL_GetEventDescription(&e, buf, sizeof(buf));
                P("event: %s", buf);
                if (e.type == SDL_EVENT_JOYSTICK_ADDED) {
                    P("  joystick %u: name=%s gamepad=%d", e.jdevice.which, SDL_GetJoystickNameForID(e.jdevice.which), SDL_IsGamepad(e.jdevice.which));
                }
            }
            SDL_Delay(16);
            frames++;
        }
        P("pumped %d frames", frames);
    }
    if (w) SDL_DestroyWindow(w);
    SDL_Quit();
    P("done");
    for (i = 1; i < argc; i++) {
        if (SDL_strcmp(argv[i], "--return") == 0) {
            P("returning 3 from main without exit()");
            return 3;
        }
    }
    /* upstream doesn't exit after main returns (postFinishLaunch); the
       runner protocol does */
    printf("sdl3-ios-runner: exit 0\n");
    fflush(stdout);
    exit(0);
    return 0;
}
