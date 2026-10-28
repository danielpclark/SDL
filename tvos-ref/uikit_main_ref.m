/* Upstream reference for sdl3/tests/uikit_main.rs (diag branch only). */
#define SDL_MAIN_HANDLED
#include "common.h"
#include <SDL3/SDL_main.h>

static int lifecycle[256];
static int nlifecycle;
static bool before_run_app;
static bool main_returned;

static bool SDLCALL watch(void *userdata, SDL_Event *e)
{
    (void)userdata;
    if (e->type >= SDL_EVENT_TERMINATING && e->type <= SDL_EVENT_DID_ENTER_FOREGROUND && nlifecycle < 256) {
        printf("uikit_main: %.3f s: %s\n", elapsed(), event_name(e->type));
        lifecycle[nlifecycle++] = (int)e->type;
    }
    return true;
}

static void take_lifecycle(List *l)
{
    for (int i = 0; i < nlifecycle; i++) {
        list_add(l, event_name((Uint32)lifecycle[i]));
    }
    nlifecycle = 0;
}

static void take_queued(List *l, const char *scheme)
{
    SDL_Event e;
    SDL_PumpEvents();
    while (SDL_PollEvent(&e)) {
        const char *d = describe(&e, scheme);
        if (d) {
            list_add(l, d);
        }
    }
}

static const char *post(NSString *name)
{
    static List l;
    memset(&l, 0, sizeof(l));
    [[NSNotificationCenter defaultCenter] postNotificationName:name object:[UIApplication sharedApplication]];
    take_lifecycle(&l);
    take_queued(&l, "");
    return list_str(&l);
}

static const char *yes_no(bool b) { return b ? "yes" : "no"; }

static bool contains(List *l, const char *what)
{
    return strstr(l->s, what) != NULL;
}

static void after_main(void *ctx)
{
    (void)ctx;
    if (!main_returned) {
        printf("main didn't return\n");
        exit_line(101);
        exit(101);
    }
    t("after main returned: application state: %d", (int)[UIApplication sharedApplication].applicationState);
    [[NSNotificationCenter defaultCenter] postNotificationName:UIApplicationDidReceiveMemoryWarningNotification object:[UIApplication sharedApplication]];
    {
        List l = { { 0 }, 0 };
        take_lifecycle(&l);
        t("after main returned: posted memory warning: %s", list_str(&l));
    }
    SDL_RemoveEventWatch(watch, NULL);
    SDL_Quit();
    t("quit");
    exit_line(0);
    exit(0);
}

static int app_main(int argc, char *argv[])
{
    (void)argv;
    printf("uikit_main: %.3f s: main\n", elapsed());
    t("main: argc=%d", argc);
    t("main thread: %s", yes_no([NSThread isMainThread]));
    t("before run_app: %s", yes_no(before_run_app));
    UIApplication *app = [UIApplication sharedApplication];
    printf("uikit_main: %.3f s: application state %d\n", elapsed(), (int)app.applicationState);
    id app_delegate = app.delegate;
    t("app delegate: %s", object_getClassName(app_delegate));
    UIScene *scene = app.connectedScenes.allObjects.firstObject;
    id scene_delegate = scene.delegate;
    t("scenes: %d %s, delegate %s, the app delegate: %s", (int)app.connectedScenes.count, object_getClassName(scene),
      object_getClassName(scene_delegate), yes_no(scene_delegate == app_delegate));

    {
        char cwd[4096], a[4096], b[4096];
        getcwd(cwd, sizeof(cwd));
        realpath(cwd, a);
        realpath([NSBundle mainBundle].resourcePath.UTF8String, b);
        t("cwd is the bundle's resource path: %s", yes_no(strcmp(a, b) == 0));
    }

    t("form factor: %s", SDL_GetDeviceFormFactorName(SDL_GetDeviceFormFactor()));
    {
        int secs, pct;
        SDL_PowerState ps = SDL_GetPowerInfo(&secs, &pct);
        t("power: state=%d secs=%d pct=%d", (int)ps, secs, pct);
    }
#if !TARGET_OS_TV
    t("battery monitoring: %s", yes_no([UIDevice currentDevice].isBatteryMonitoringEnabled));
#endif
    t("platform: %s", SDL_GetPlatform());

    SDL_AddEventWatch(watch, NULL);
    bool ok = SDL_Init(SDL_INIT_VIDEO);
    t("init video: %s", ok ? "ok" : SDL_GetError());
    t("video driver: %s", SDL_GetCurrentVideoDriver());
    t("system theme: %d", (int)SDL_GetSystemTheme());
    int n = 0;
    SDL_DisplayID *displays = SDL_GetDisplays(&n);
    t("displays: %d", n);
    SDL_DisplayID display = displays[0];
    SDL_Rect r;
    SDL_GetDisplayBounds(display, &r);
    t("display: bounds=%d,%d %dx%d natural=%d current=%d", r.x, r.y, r.w, r.h,
      (int)SDL_GetNaturalDisplayOrientation(display), (int)SDL_GetCurrentDisplayOrientation(display));
    const SDL_DisplayMode *m = SDL_GetDesktopDisplayMode(display);
    t("desktop mode: %dx%d fmt=%s density=%g refresh=%g", m->w, m->h, SDL_GetPixelFormatName(m->format),
      m->pixel_density, m->refresh_rate);
    {
        List l = { { 0 }, 0 };
        take_queued(&l, "");
        t("events: %s", list_str(&l));
    }
    {
        List l = { { 0 }, 0 };
        List queued = { { 0 }, 0 };
        Uint64 end = SDL_GetTicks() + 30000;
        while ([UIApplication sharedApplication].applicationState != 0 && SDL_GetTicks() < end) {
            take_queued(&queued, "");
            SDL_Delay(10);
        }
        printf("uikit_main: %.3f s: application state %d\n", elapsed(), (int)[UIApplication sharedApplication].applicationState);
        for (int i = 0; i < 10; i++) {
            take_queued(&queued, "");
            SDL_Delay(10);
        }
        take_lifecycle(&l);
        t("activation: application state %d, %s", (int)[UIApplication sharedApplication].applicationState, list_str(&queued));
    }

    struct { const char *label; NSString *name; } notifications[] = {
        { "will resign active", UIApplicationWillResignActiveNotification },
        { "did enter background", UIApplicationDidEnterBackgroundNotification },
        { "will enter foreground", UIApplicationWillEnterForegroundNotification },
        { "did become active", UIApplicationDidBecomeActiveNotification },
        { "memory warning", UIApplicationDidReceiveMemoryWarningNotification },
        { "will terminate", UIApplicationWillTerminateNotification },
#if !TARGET_OS_TV
        { "status bar orientation", UIApplicationDidChangeStatusBarOrientationNotification },
#endif
    };
    for (int i = 0; i < (int)SDL_arraysize(notifications); i++) {
        t("posted %s: %s", notifications[i].label, post(notifications[i].name));
    }

    __block bool ran = false;
    SDL_SetiOSEventPump(false);
    dispatch_async(dispatch_get_main_queue(), ^{ ran = true; });
    SDL_PumpEvents();
    t("pump off: main queue ran: %s", yes_no(ran));
    t("pump off: posted memory warning: %s", post(UIApplicationDidReceiveMemoryWarningNotification));
    SDL_SetiOSEventPump(true);
    SDL_PumpEvents();
    t("pump on: main queue ran: %s", yes_no(ran));
    t("pump on: posted memory warning: %s", post(UIApplicationDidReceiveMemoryWarningNotification));

    {
        List l = { { 0 }, 0 };
        [scene_delegate performSelector:@selector(handleURL:) withObject:[NSURL fileURLWithPath:@"/tmp/sdl3 drop.txt"]];
        take_queued(&l, "");
        t("handleURL file: %s", list_str(&l));
    }
    {
        List l = { { 0 }, 0 };
        [scene_delegate performSelector:@selector(handleURL:) withObject:[NSURL URLWithString:@"https://libsdl.org/?q=1"]];
        take_queued(&l, "");
        t("handleURL https: %s", list_str(&l));
    }

    t("open_url unknown scheme: %s", SDL_OpenURL("sdl-no-such-scheme-for-tests://x") ? "ok" : SDL_GetError());
    const char *scheme = [NSBundle mainBundle].bundleIdentifier.UTF8String;
    char url[512];
    SDL_snprintf(url, sizeof(url), "%s://open-url-test", scheme);
    t("open_url own scheme: %s", SDL_OpenURL(url) ? "ok" : SDL_GetError());
    printf("uikit_main: %.3f s: opened\n", elapsed());
    {
        List l = { { 0 }, 0 };
        Uint64 end = SDL_GetTicks() + 10000;
        while (SDL_GetTicks() < end) {
            take_queued(&l, scheme);
            if (contains(&l, "DROP_COMPLETE")) {
                break;
            }
            SDL_Delay(10);
        }
        printf("uikit_main: %.3f s: drop events\n", elapsed());
        t("own scheme opened: %s", list_str(&l));
        nlifecycle = 0;
    }

    {
        List l = { { 0 }, 0 };
        List ignored = { { 0 }, 0 };
        bool backgrounded = false, foregrounded = false;
        printf("sdl3-ios-runner: request background\n");
        fflush(stdout);
        Uint64 end = SDL_GetTicks() + 60000;
        while (SDL_GetTicks() < end) {
            take_queued(&ignored, "");
            take_lifecycle(&l);
            if (contains(&l, "DID_ENTER_BACKGROUND")) {
                backgrounded = true;
                break;
            }
            SDL_Delay(10);
        }
        printf("sdl3-ios-runner: request foreground\n");
        fflush(stdout);
        end = SDL_GetTicks() + 60000;
        while (backgrounded && SDL_GetTicks() < end) {
            take_queued(&ignored, "");
            take_lifecycle(&l);
            if (contains(&l, "DID_ENTER_FOREGROUND")) {
                foregrounded = true;
                break;
            }
            SDL_Delay(10);
        }
        end = SDL_GetTicks() + 1000;
        while (foregrounded && SDL_GetTicks() < end) {
            take_queued(&ignored, "");
            take_lifecycle(&l);
            SDL_Delay(10);
        }
        t("background and back: %s", list_str(&l));
    }
    t("application state: %d", (int)[UIApplication sharedApplication].applicationState);

    dispatch_after_f(dispatch_time(DISPATCH_TIME_NOW, 500 * NSEC_PER_MSEC), dispatch_get_main_queue(), NULL, after_main);
    t("main returns");
    main_returned = true;
    return 0;
}

int main(int argc, char *argv[])
{
    elapsed();
    before_run_app = true;
    int status = SDL_RunApp(argc, argv, app_main, NULL);
    printf("uikit_main_ref: SDL_RunApp returned %d\n", status);
    exit_line(1);
    return 1;
}
