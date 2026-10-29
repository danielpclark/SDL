/* Upstream SDL3 reference trace on the Android emulator (built with
   upstream's android-project and Gradle on the diag branch only): the
   platform, paths, drivers, displays, a window, renderers, the GPU API,
   audio devices, cameras, sensors, joysticks; SDL_Log goes to logcat. */
#include <SDL3/SDL.h>
#include <SDL3/SDL_main.h>
#include <stdlib.h>

#define P(...) SDL_Log("[c] " __VA_ARGS__)

int main(int argc, char *argv[])
{
    int i, n = 0;
    Uint64 t0 = SDL_GetTicksNS();
    P("main argc=%d argv0=%s platform=%s version=%d revision=%s", argc, argv[0], SDL_GetPlatform(), SDL_GetVersion(), SDL_GetRevision());
    P("SDK=%d chromebook=%d dex=%d tablet=%d tv=%d", SDL_GetAndroidSDKVersion(), SDL_IsChromebook(), SDL_IsDeXMode(), SDL_IsTablet(), SDL_IsTV());
    P("internal=%s external=%s (state 0x%x) cache=%s", SDL_GetAndroidInternalStoragePath(), SDL_GetAndroidExternalStoragePath(),
      SDL_GetAndroidExternalStorageState(), SDL_GetAndroidCachePath());
    P("base=%s pref=%s", SDL_GetBasePath(), SDL_GetPrefPath("org", "app"));
    {
        SDL_Locale **loc = SDL_GetPreferredLocales(&n);
        for (i = 0; i < n; i++) P("locale %d: %s_%s", i, loc[i]->language, loc[i]->country ? loc[i]->country : "");
        SDL_free(loc);
    }
    for (i = 0; i < SDL_GetNumVideoDrivers(); i++) P("video driver %d: %s", i, SDL_GetVideoDriver(i));
    for (i = 0; i < SDL_GetNumAudioDrivers(); i++) P("audio driver %d: %s", i, SDL_GetAudioDriver(i));
    if (!SDL_Init(SDL_INIT_VIDEO | SDL_INIT_AUDIO | SDL_INIT_GAMEPAD | SDL_INIT_SENSOR | SDL_INIT_CAMERA | SDL_INIT_HAPTIC)) {
        P("SDL_Init failed: %s", SDL_GetError());
    }
    P("SDL_Init done after %.1f ms", (SDL_GetTicksNS() - t0) / 1e6);
    P("current video driver: %s; audio driver: %s; camera driver: %s", SDL_GetCurrentVideoDriver(), SDL_GetCurrentAudioDriver(), SDL_GetCurrentCameraDriver());
    {
        int secs, pct;
        SDL_PowerState ps = SDL_GetPowerInfo(&secs, &pct);
        P("power: state=%d secs=%d pct=%d", (int)ps, secs, pct);
    }
    {
        SDL_DisplayID *d = SDL_GetDisplays(&n);
        for (i = 0; i < n; i++) {
            SDL_Rect b, u;
            const SDL_DisplayMode *m = SDL_GetCurrentDisplayMode(d[i]);
            SDL_GetDisplayBounds(d[i], &b);
            SDL_GetDisplayUsableBounds(d[i], &u);
            P("display %u name=%s bounds=%d,%d %dx%d usable=%d,%d %dx%d content_scale=%g natural=%d current=%d", d[i], SDL_GetDisplayName(d[i]),
              b.x, b.y, b.w, b.h, u.x, u.y, u.w, u.h, SDL_GetDisplayContentScale(d[i]), (int)SDL_GetNaturalDisplayOrientation(d[i]),
              (int)SDL_GetCurrentDisplayOrientation(d[i]));
            if (m) P("  mode %dx%d fmt=%s density=%g refresh=%g", m->w, m->h, SDL_GetPixelFormatName(m->format), m->pixel_density, m->refresh_rate);
        }
        SDL_free(d);
    }
    SDL_Window *w = SDL_CreateWindow("spike", 0, 0, SDL_WINDOW_HIGH_PIXEL_DENSITY);
    if (!w) {
        P("SDL_CreateWindow failed: %s", SDL_GetError());
    } else {
        int ww, wh, pw, ph;
        SDL_Rect sa;
        SDL_GetWindowSize(w, &ww, &wh);
        SDL_GetWindowSizeInPixels(w, &pw, &ph);
        SDL_GetWindowSafeArea(w, &sa);
        P("window: size=%dx%d pixels=%dx%d density=%g display_scale=%g flags=0x%llx safe_area=%d,%d %dx%d", ww, wh, pw, ph,
          SDL_GetWindowPixelDensity(w), SDL_GetWindowDisplayScale(w), (unsigned long long)SDL_GetWindowFlags(w), sa.x, sa.y, sa.w, sa.h);
        for (i = 0; i < SDL_GetNumRenderDrivers(); i++) P("render driver %d: %s", i, SDL_GetRenderDriver(i));
        {
            const char *names[] = { NULL, "opengles2", "opengles", "vulkan", "gpu", "software" };
            int k;
            for (k = 0; k < 6; k++) {
                SDL_Renderer *r = SDL_CreateRenderer(w, names[k]);
                if (r) {
                    int ow, oh;
                    SDL_GetRenderOutputSize(r, &ow, &oh);
                    P("renderer %s: %s output=%dx%d", names[k] ? names[k] : "(default)", SDL_GetRendererName(r), ow, oh);
                    SDL_SetRenderDrawColor(r, 30, 60, 200, 255);
                    SDL_RenderClear(r);
                    SDL_RenderPresent(r);
                    SDL_DestroyRenderer(r);
                } else {
                    P("renderer %s: failed: %s", names[k] ? names[k] : "(default)", SDL_GetError());
                }
            }
        }
    }
    for (i = 0; i < SDL_GetNumGPUDrivers(); i++) P("gpu driver %d: %s", i, SDL_GetGPUDriver(i));
    {
        SDL_GPUDevice *g = SDL_CreateGPUDevice(SDL_GPU_SHADERFORMAT_SPIRV, false, NULL);
        P("SDL_CreateGPUDevice(SPIRV): %s", g ? SDL_GetGPUDeviceDriver(g) : SDL_GetError());
        if (g) SDL_DestroyGPUDevice(g);
    }
    {
        SDL_AudioDeviceID *a = SDL_GetAudioPlaybackDevices(&n);
        for (i = 0; i < n; i++) P("playback device %u: %s", a[i], SDL_GetAudioDeviceName(a[i]));
        SDL_free(a);
        a = SDL_GetAudioRecordingDevices(&n);
        for (i = 0; i < n; i++) P("recording device %u: %s", a[i], SDL_GetAudioDeviceName(a[i]));
        SDL_free(a);
        SDL_AudioSpec spec = { SDL_AUDIO_F32, 2, 48000 };
        SDL_AudioStream *s = SDL_OpenAudioDeviceStream(SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK, &spec, NULL, NULL);
        if (s) {
            SDL_AudioSpec src, dst;
            int frames = 0;
            SDL_GetAudioDeviceFormat(SDL_GetAudioStreamDevice(s), &dst, &frames);
            SDL_GetAudioStreamFormat(s, &src, NULL);
            P("playback stream opened: device format %d ch=%d rate=%d frames=%d", dst.format, dst.channels, dst.freq, frames);
            SDL_DestroyAudioStream(s);
        } else {
            P("SDL_OpenAudioDeviceStream(playback) failed: %s", SDL_GetError());
        }
    }
    {
        SDL_CameraID *c = SDL_GetCameras(&n);
        P("cameras: %d", n);
        for (i = 0; i < n; i++) P("camera %u: %s position=%d", c[i], SDL_GetCameraName(c[i]), (int)SDL_GetCameraPosition(c[i]));
        SDL_free(c);
    }
    {
        SDL_SensorID *s = SDL_GetSensors(&n);
        P("sensors: %d", n);
        for (i = 0; i < n; i++) P("sensor %u: %s type=%d", s[i], SDL_GetSensorNameForID(s[i]), (int)SDL_GetSensorTypeForID(s[i]));
        SDL_free(s);
    }
    {
        SDL_JoystickID *j = SDL_GetJoysticks(&n);
        P("joysticks: %d", n);
        for (i = 0; i < n; i++) P("joystick %u: %s gamepad=%d", j[i], SDL_GetJoystickNameForID(j[i]), SDL_IsGamepad(j[i]));
        SDL_free(j);
        SDL_HapticID *h = SDL_GetHaptics(&n);
        P("haptics: %d", n);
        SDL_free(h);
    }
    {
        Uint64 start = SDL_GetTicks();
        int frames = 0;
        while (SDL_GetTicks() - start < 2000) {
            SDL_Event e;
            while (SDL_PollEvent(&e)) {
                char buf[256];
                SDL_GetEventDescription(&e, buf, sizeof(buf));
                P("event: %s", buf);
            }
            SDL_Delay(16);
            frames++;
        }
        P("pumped %d frames", frames);
    }
    if (w) SDL_DestroyWindow(w);
    SDL_Quit();
    P("done after %.1f ms", (SDL_GetTicksNS() - t0) / 1e6);
    SDL_Log("sdl3-android-runner: exit 0");
    return 0;
}
