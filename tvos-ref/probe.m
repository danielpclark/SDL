/* A probe of upstream SDL on the tvOS Simulator (diag branch only). */
#define SDL_MAIN_HANDLED
#include "common.h"
#include <SDL3/SDL_main.h>
#import <GameController/GameController.h>
#import <Metal/Metal.h>

static void pump(int ms)
{
    Uint64 end = SDL_GetTicks() + ms;
    SDL_Event e;
    while (SDL_GetTicks() < end) {
        SDL_PumpEvents();
        while (SDL_PollEvent(&e)) {
            const char *d = describe(&e, NULL);
            if (d) {
                t("event %s", d);
            }
        }
        SDL_Delay(10);
    }
}

static int app_main(int argc, char *argv[])
{
    (void)argc; (void)argv;
    Uint64 end = SDL_GetTicks() + 30000;
    while ([UIApplication sharedApplication].applicationState != UIApplicationStateActive && SDL_GetTicks() < end) {
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, false);
    }
    t("app state %ld", (long)[UIApplication sharedApplication].applicationState);
    UIScreen *s = [UIScreen mainScreen];
    t("screen bounds %gx%g native %gx%g scale %g native %g fps %ld screens %lu current %gx%g",
      s.bounds.size.width, s.bounds.size.height, s.nativeBounds.size.width, s.nativeBounds.size.height,
      s.scale, s.nativeScale, (long)s.maximumFramesPerSecond, (unsigned long)[UIScreen screens].count,
      s.currentMode.size.width, s.currentMode.size.height);
    t("idiom %ld system %s %s", (long)[UIDevice currentDevice].userInterfaceIdiom,
      [UIDevice currentDevice].systemName.UTF8String, [UIDevice currentDevice].systemVersion.UTF8String);
    for (GCController *c in [GCController controllers]) {
        t("GCController %s category %s extended %d micro %d", c.vendorName.UTF8String, c.productCategory.UTF8String,
          c.extendedGamepad != nil, c.microGamepad != nil);
    }
    t("GCKeyboard %p", (__bridge void *)[GCKeyboard coalescedKeyboard]);
    id<MTLDevice> dev = MTLCreateSystemDefaultDevice();
    t("metal %s apple1 %d apple2 %d apple3 %d common1 %d", dev.name.UTF8String,
      [dev supportsFamily:MTLGPUFamilyApple1], [dev supportsFamily:MTLGPUFamilyApple2],
      [dev supportsFamily:MTLGPUFamilyApple3], [dev supportsFamily:MTLGPUFamilyCommon1]);

    bool ok = SDL_Init(SDL_INIT_VIDEO | SDL_INIT_AUDIO | SDL_INIT_GAMEPAD | SDL_INIT_SENSOR | SDL_INIT_CAMERA);
    t("init %s; platform %s video %s audio %s camera %s", ok ? "ok" : SDL_GetError(), SDL_GetPlatform(),
      SDL_GetCurrentVideoDriver(), SDL_GetCurrentAudioDriver(), SDL_GetCurrentCameraDriver());
    for (int i = 0; i < SDL_GetNumVideoDrivers(); i++) t("video driver %d %s", i, SDL_GetVideoDriver(i));
    for (int i = 0; i < SDL_GetNumRenderDrivers(); i++) t("render driver %d %s", i, SDL_GetRenderDriver(i));
    for (int i = 0; i < SDL_GetNumCameraDrivers(); i++) t("camera driver %d %s", i, SDL_GetCameraDriver(i));
    int secs, pct;
    SDL_PowerState ps = SDL_GetPowerInfo(&secs, &pct);
    t("power %d %d %d", (int)ps, secs, pct);
    char *pref = SDL_GetPrefPath("org", "app");
    t("pref %s", pref ? pref : SDL_GetError());
    t("base %s", SDL_GetBasePath());
    t("documents %s", SDL_GetUserFolder(SDL_FOLDER_DOCUMENTS) ? SDL_GetUserFolder(SDL_FOLDER_DOCUMENTS) : SDL_GetError());
    t("home %s", SDL_GetUserFolder(SDL_FOLDER_HOME) ? SDL_GetUserFolder(SDL_FOLDER_HOME) : SDL_GetError());
    SDL_DisplayID d = SDL_GetPrimaryDisplay();
    SDL_Rect r, u;
    SDL_GetDisplayBounds(d, &r);
    SDL_GetDisplayUsableBounds(d, &u);
    t("display %u %s bounds %d,%d %dx%d usable %d,%d %dx%d scale %g nat %d cur %d", (unsigned)d, SDL_GetDisplayName(d),
      r.x, r.y, r.w, r.h, u.x, u.y, u.w, u.h, (double)SDL_GetDisplayContentScale(d),
      (int)SDL_GetNaturalDisplayOrientation(d), (int)SDL_GetCurrentDisplayOrientation(d));
    const SDL_DisplayMode *m = SDL_GetDesktopDisplayMode(d);
    t("desktop %dx%d %g %g", m->w, m->h, (double)m->pixel_density, (double)m->refresh_rate);
    int count;
    SDL_DisplayMode **modes = SDL_GetFullscreenDisplayModes(d, &count);
    for (int i = 0; i < count; i++) t("fs mode %dx%d %g %g", modes[i]->w, modes[i]->h, (double)modes[i]->pixel_density, (double)modes[i]->refresh_rate);
    SDL_free(modes);
    SDL_Window *w = SDL_CreateWindow("probe", 640, 480, SDL_WINDOW_HIGH_PIXEL_DENSITY);
    pump(500);
    int ww, wh, pw, ph;
    SDL_GetWindowSize(w, &ww, &wh);
    SDL_GetWindowSizeInPixels(w, &pw, &ph);
    SDL_GetWindowSafeArea(w, &r);
    t("window %dx%d pixels %dx%d flags 0x%llx safe %d,%d %dx%d", ww, wh, pw, ph, (unsigned long long)SDL_GetWindowFlags(w), r.x, r.y, r.w, r.h);
    UIWindow *uiw = (__bridge UIWindow *)SDL_GetPointerProperty(SDL_GetWindowProperties(w), SDL_PROP_WINDOW_UIKIT_WINDOW_POINTER, NULL);
    UIViewController *vc = uiw.rootViewController;
    t("vc %s super %s view %s", class_getName([vc class]), class_getName([vc superclass]), class_getName([vc.view class]));
    t("safe insets %g %g %g %g", uiw.safeAreaInsets.top, uiw.safeAreaInsets.left, uiw.safeAreaInsets.bottom, uiw.safeAreaInsets.right);
    SDL_Renderer *ren = SDL_CreateRenderer(w, NULL);
    t("renderer %s", ren ? SDL_GetRendererName(ren) : SDL_GetError());
    SDL_GPUDevice *gpu = SDL_CreateGPUDevice(SDL_GPU_SHADERFORMAT_MSL | SDL_GPU_SHADERFORMAT_METALLIB, false, NULL);
    t("gpu %s", gpu ? SDL_GetGPUDeviceDriver(gpu) : SDL_GetError());
    int n;
    SDL_JoystickID *js = SDL_GetJoysticks(&n);
    for (int i = 0; i < n; i++) t("joystick %u %s gamepad %d", (unsigned)js[i], SDL_GetJoystickNameForID(js[i]), SDL_IsGamepad(js[i]));
    SDL_KeyboardID *kb = SDL_GetKeyboards(&n);
    for (int i = 0; i < n; i++) t("keyboard %u %s", (unsigned)kb[i], SDL_GetKeyboardNameForID(kb[i]));
    SDL_MouseID *mice = SDL_GetMice(&n);
    t("mice %d", n);
    SDL_TouchID *touches = SDL_GetTouchDevices(&n);
    t("touch devices %d", n);
    SDL_SetHint(SDL_HINT_TV_REMOTE_AS_JOYSTICK, "1");
    pump(1000);
    SDL_StartTextInput(w);
    pump(1500);
    t("text input active %d keyboard shown %d", SDL_TextInputActive(w), SDL_ScreenKeyboardShown(w));
    SDL_StopTextInput(w);
    pump(1500);
    SDL_free(js); SDL_free(kb); SDL_free(mice); SDL_free(touches);
    SDL_Quit();
    t("done");
    exit_line(0);
    exit(0);
}

int main(int argc, char *argv[])
{
    elapsed();
    int status = SDL_RunApp(argc, argv, app_main, NULL);
    printf("probe: SDL_RunApp returned %d\n", status);
    exit_line(1);
    return 1;
}
