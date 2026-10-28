/* Upstream reference for sdl3/tests/uikit_tvos.rs (diag branch only). */
#define SDL_MAIN_HANDLED
#include "common.h"
#include <SDL3/SDL_main.h>
#import <GameController/GameController.h>

static const char *yes_no(bool b) { return b ? "yes" : "no"; }

static SDL_WindowID ids[8];
static const char *labels[8];
static int nids;

static void name_window(SDL_Window *w, const char *label)
{
    if (nids < 8) {
        ids[nids] = SDL_GetWindowID(w);
        labels[nids] = label;
        nids++;
    }
}

static const char *window_label(SDL_WindowID id)
{
    for (int i = 0; i < nids; i++) {
        if (ids[i] == id) {
            return labels[i];
        }
    }
    return "?";
}

static const char *wname(Uint32 type)
{
    switch (type) {
#define N(x) case SDL_EVENT_##x: return #x;
    N(WINDOW_SHOWN) N(WINDOW_HIDDEN) N(WINDOW_EXPOSED) N(WINDOW_MOVED) N(WINDOW_RESIZED)
    N(WINDOW_PIXEL_SIZE_CHANGED) N(WINDOW_METAL_VIEW_RESIZED) N(WINDOW_MINIMIZED) N(WINDOW_MAXIMIZED)
    N(WINDOW_RESTORED) N(WINDOW_MOUSE_ENTER) N(WINDOW_MOUSE_LEAVE) N(WINDOW_FOCUS_GAINED)
    N(WINDOW_FOCUS_LOST) N(WINDOW_CLOSE_REQUESTED) N(WINDOW_HIT_TEST) N(WINDOW_ICCPROF_CHANGED)
    N(WINDOW_DISPLAY_CHANGED) N(WINDOW_DISPLAY_SCALE_CHANGED) N(WINDOW_SAFE_AREA_CHANGED)
    N(WINDOW_OCCLUDED) N(WINDOW_ENTER_FULLSCREEN) N(WINDOW_LEAVE_FULLSCREEN) N(WINDOW_DESTROYED)
    N(WINDOW_HDR_STATE_CHANGED) N(AUDIO_DEVICE_ADDED) N(AUDIO_DEVICE_REMOVED)
#undef N
    default: return event_name(type);
    }
}

/* An event as the trace names it; NULL for the mice's device events. */
static const char *describe_w(const SDL_Event *e)
{
    static char buf[256];
    switch (e->type) {
    case SDL_EVENT_MOUSE_ADDED: case SDL_EVENT_MOUSE_REMOVED:
        return NULL;
    default: break;
    }
    if (e->type >= SDL_EVENT_DISPLAY_FIRST && e->type <= SDL_EVENT_DISPLAY_LAST) {
        SDL_snprintf(buf, sizeof(buf), "%s(%u,%d)", event_name(e->type), (unsigned)e->display.displayID, (int)e->display.data1);
        return buf;
    }
    if (e->type >= SDL_EVENT_WINDOW_FIRST && e->type <= SDL_EVENT_WINDOW_LAST) {
        SDL_snprintf(buf, sizeof(buf), "%s:%s(%d,%d)", window_label(e->window.windowID), wname(e->type), (int)e->window.data1, (int)e->window.data2);
        return buf;
    }
    return wname(e->type);
}

static void take_queued(List *l)
{
    SDL_Event e;
    SDL_PumpEvents();
    while (SDL_PollEvent(&e)) {
        const char *d = describe_w(&e);
        if (d) {
            list_add(l, d);
        }
    }
}

static void settle(List *l, int ms)
{
    take_queued(l);
    for (int i = 0; i < ms / 10; i++) {
        SDL_Delay(10);
        take_queued(l);
    }
}

static void events(const char *label, int ms)
{
    List l;
    memset(&l, 0, sizeof(l));
    settle(&l, ms);
    t("%s: events: %s", label, list_str(&l));
}

static const char *ok(bool b)
{
    static char buf[512];
    if (b) {
        return "ok";
    }
    SDL_snprintf(buf, sizeof(buf), "error: %s", SDL_GetError());
    return buf;
}

/* A path with the app's home directory (its container) as HOME. */
static const char *home_path(const char *path)
{
    static char buf[2][1024];
    static int n;
    char *b = buf[n++ % 2];
    const char *home = NSHomeDirectory().UTF8String;
    if (!path) {
        SDL_snprintf(b, 1024, "error: %s", SDL_GetError());
    } else if (strncmp(path, home, strlen(home)) == 0) {
        SDL_snprintf(b, 1024, "HOME%s", path + strlen(home));
    } else {
        SDL_snprintf(b, 1024, "%s", path);
    }
    return b;
}

static char *names[64];
static int nnames;
static void SDLCALL enum_cb(void *userdata, SDL_PropertiesID props, const char *name)
{
    (void)userdata; (void)props;
    if (nnames < 64) {
        names[nnames++] = SDL_strdup(name);
    }
}
static int cmp_str(const void *a, const void *b) { return strcmp(*(char *const *)a, *(char *const *)b); }

static void props_state(const char *label, SDL_PropertiesID props)
{
    List l;
    memset(&l, 0, sizeof(l));
    nnames = 0;
    if (props) {
        SDL_EnumerateProperties(props, enum_cb, NULL);
    }
    qsort(names, (size_t)nnames, sizeof(names[0]), cmp_str);
    for (int i = 0; i < nnames; i++) {
        char item[512];
        switch (SDL_GetPropertyType(props, names[i])) {
        case SDL_PROPERTY_TYPE_BOOLEAN: SDL_snprintf(item, sizeof(item), "%s=%s", names[i], SDL_GetBooleanProperty(props, names[i], false) ? "true" : "false"); break;
        case SDL_PROPERTY_TYPE_NUMBER: SDL_snprintf(item, sizeof(item), "%s=%lld", names[i], (long long)SDL_GetNumberProperty(props, names[i], 0)); break;
        case SDL_PROPERTY_TYPE_FLOAT: SDL_snprintf(item, sizeof(item), "%s=%g", names[i], (double)SDL_GetFloatProperty(props, names[i], 0.0f)); break;
        case SDL_PROPERTY_TYPE_STRING: SDL_snprintf(item, sizeof(item), "%s=%s", names[i], SDL_GetStringProperty(props, names[i], "")); break;
        case SDL_PROPERTY_TYPE_POINTER: SDL_snprintf(item, sizeof(item), "%s=(pointer)", names[i]); break;
        default: SDL_snprintf(item, sizeof(item), "%s=(none)", names[i]); break;
        }
        list_add(&l, item);
        SDL_free(names[i]);
    }
    t("%s: properties: %s", label, list_str(&l));
}

static const char *rect_str(const SDL_Rect *r)
{
    static char buf[4][64];
    static int n;
    char *b = buf[n++ % 4];
    SDL_snprintf(b, 64, "%d,%d %dx%d", r->x, r->y, r->w, r->h);
    return b;
}

static const char *cgrect_str(CGRect r)
{
    static char buf[4][96];
    static int n;
    char *b = buf[n++ % 4];
    SDL_snprintf(b, 96, "%g,%g %gx%g", r.origin.x, r.origin.y, r.size.width, r.size.height);
    return b;
}

static const char *mode_str(const SDL_DisplayMode *m)
{
    static char buf[256];
    SDL_snprintf(buf, sizeof(buf), "%dx%d fmt=%s density=%g refresh=%g (%d/%d) display=%u",
                 m->w, m->h, SDL_GetPixelFormatName(m->format), (double)m->pixel_density,
                 (double)m->refresh_rate, m->refresh_rate_numerator, m->refresh_rate_denominator,
                 (unsigned)m->displayID);
    return buf;
}

static void display_state(const char *label, SDL_DisplayID display)
{
    SDL_Rect r, u;
    char b1[128], b2[128];
    const char *name = SDL_GetDisplayName(display);
    t("%s: display %u name=%s primary=%s", label, (unsigned)display, name ? name : "", yes_no(SDL_GetPrimaryDisplay() == display));
    if (SDL_GetDisplayBounds(display, &r)) SDL_snprintf(b1, sizeof(b1), "%s", rect_str(&r)); else SDL_snprintf(b1, sizeof(b1), "error: %s", SDL_GetError());
    if (SDL_GetDisplayUsableBounds(display, &u)) SDL_snprintf(b2, sizeof(b2), "%s", rect_str(&u)); else SDL_snprintf(b2, sizeof(b2), "error: %s", SDL_GetError());
    t("%s: bounds=%s usable=%s", label, b1, b2);
    t("%s: content_scale=%g natural=%d current=%d", label, (double)SDL_GetDisplayContentScale(display),
      (int)SDL_GetNaturalDisplayOrientation(display), (int)SDL_GetCurrentDisplayOrientation(display));
    const SDL_DisplayMode *dm = SDL_GetDesktopDisplayMode(display);
    if (dm) t("%s: desktop mode %s", label, mode_str(dm)); else t("%s: desktop mode error: %s", label, SDL_GetError());
    const SDL_DisplayMode *cm = SDL_GetCurrentDisplayMode(display);
    if (cm) t("%s: current mode %s", label, mode_str(cm)); else t("%s: current mode error: %s", label, SDL_GetError());
    int count = 0;
    SDL_DisplayMode **modes = SDL_GetFullscreenDisplayModes(display, &count);
    t("%s: fullscreen modes: %d", label, count);
    for (int i = 0; modes && i < count; i++) {
        t("%s: fullscreen mode %d: %s", label, i, mode_str(modes[i]));
    }
    SDL_free(modes);
    struct { int w, h; float rate; bool hd; } q[] = {
        { 1920, 1080, 0.0f, true }, { 1920, 1080, 0.0f, false }, { 1080, 1920, 0.0f, true },
        { 1280, 720, 0.0f, true }, { 3840, 2160, 0.0f, true }, { 1920, 1080, 30.0f, true }, { 0, 0, 0.0f, true },
    };
    for (size_t i = 0; i < SDL_arraysize(q); i++) {
        SDL_DisplayMode closest;
        if (SDL_GetClosestFullscreenDisplayMode(display, q[i].w, q[i].h, q[i].rate, q[i].hd, &closest)) {
            t("%s: closest to %dx%d at %g high density %s: %s", label, q[i].w, q[i].h, (double)q[i].rate, yes_no(q[i].hd), mode_str(&closest));
        } else {
            t("%s: closest to %dx%d at %g high density %s: error: %s", label, q[i].w, q[i].h, (double)q[i].rate, yes_no(q[i].hd), SDL_GetError());
        }
    }
    props_state(label, SDL_GetDisplayProperties(display));
}

static void displays_state(const char *label)
{
    int count = 0;
    SDL_DisplayID *displays = SDL_GetDisplays(&count);
    List l;
    memset(&l, 0, sizeof(l));
    for (int i = 0; displays && i < count; i++) {
        char b[32];
        SDL_snprintf(b, sizeof(b), "%u", (unsigned)displays[i]);
        list_add(&l, b);
    }
    t("%s: displays: %s", label, list_str(&l));
    for (int i = 0; displays && i < count; i++) {
        display_state(label, displays[i]);
    }
    SDL_free(displays);
    int pts[3][2] = { { 10, 10 }, { 1900, 1000 }, { -50, -50 } };
    for (int i = 0; i < 3; i++) {
        SDL_Point p = { pts[i][0], pts[i][1] };
        SDL_DisplayID d = SDL_GetDisplayForPoint(&p);
        if (d) t("%s: display for point %d,%d: %u", label, p.x, p.y, (unsigned)d);
        else t("%s: display for point %d,%d: error: %s", label, p.x, p.y, SDL_GetError());
    }
}

static UIWindow *uiwindow_of(SDL_Window *w)
{
    return (__bridge UIWindow *)SDL_GetPointerProperty(SDL_GetWindowProperties(w), SDL_PROP_WINDOW_UIKIT_WINDOW_POINTER, NULL);
}

static UIViewController *vc_of(SDL_Window *w)
{
    return uiwindow_of(w).rootViewController;
}

static void window_state(const char *label, SDL_Window *w)
{
    int x = 0, y = 0, ww = 0, wh = 0, pw = 0, ph = 0;
    SDL_Rect safe = { 0, 0, 0, 0 };
    SDL_GetWindowPosition(w, &x, &y);
    SDL_GetWindowSize(w, &ww, &wh);
    SDL_GetWindowSizeInPixels(w, &pw, &ph);
    SDL_GetWindowSafeArea(w, &safe);
    t("%s: pos=%d,%d size=%dx%d pixels=%dx%d density=%g display_scale=%g", label, x, y, ww, wh, pw, ph,
      (double)SDL_GetWindowPixelDensity(w), (double)SDL_GetWindowDisplayScale(w));
    t("%s: flags=0x%llx safe_area=%s display=%u", label,
      (unsigned long long)(SDL_GetWindowFlags(w) & ~SDL_WINDOW_OPENGL), rect_str(&safe),
      (unsigned)SDL_GetDisplayForWindow(w));
    const SDL_DisplayMode *fm = SDL_GetWindowFullscreenMode(w);
    if (fm) {
        t("%s: fullscreen mode %dx%d density=%g", label, fm->w, fm->h, (double)fm->pixel_density);
    } else {
        t("%s: fullscreen mode (none)", label);
    }
    UIWindow *uw = uiwindow_of(w);
    UIViewController *vc = uw.rootViewController;
    t("%s: uiwindow: %s frame=%s hidden=%s key=%s scene=%s safe insets=%g,%g,%g,%g", label,
      uw ? object_getClassName(uw) : "nil", cgrect_str(uw.frame), yes_no(uw.hidden), yes_no(uw.keyWindow),
      yes_no(uw.windowScene != nil), uw.safeAreaInsets.top, uw.safeAreaInsets.left, uw.safeAreaInsets.bottom,
      uw.safeAreaInsets.right);
    bool ui_events = [vc isKindOfClass:[GCEventViewController class]] && ((GCEventViewController *)vc).controllerUserInteractionEnabled;
    t("%s: root: %s (%s) view=%s frame=%s controller user interaction=%s", label, vc ? object_getClassName(vc) : "nil",
      vc ? class_getName([vc superclass]) : "nil", vc.view ? object_getClassName(vc.view) : "nil",
      cgrect_str(vc.view.frame), yes_no(ui_events));
    List l;
    memset(&l, 0, sizeof(l));
    for (UIGestureRecognizer *g in vc.view.gestureRecognizers) {
        char item[128];
        if ([g isKindOfClass:[UISwipeGestureRecognizer class]]) {
            SDL_snprintf(item, sizeof(item), "%s(%lu)", object_getClassName(g), (unsigned long)((UISwipeGestureRecognizer *)g).direction);
        } else {
            SDL_snprintf(item, sizeof(item), "%s", object_getClassName(g));
        }
        list_add(&l, item);
    }
    t("%s: view gesture recognizers: %s", label, list_str(&l));
    props_state(label, SDL_GetWindowProperties(w));
}

static void controller_ui_events(const char *label, SDL_Window *w, const char *hint)
{
    SDL_SetHint(SDL_HINT_APPLE_TV_CONTROLLER_UI_EVENTS, hint);
    UIViewController *vc = vc_of(w);
    bool enabled = [vc isKindOfClass:[GCEventViewController class]] && ((GCEventViewController *)vc).controllerUserInteractionEnabled;
    t("%s: controller UI events hint %s: controller user interaction=%s", label, hint ? hint : "(unset)", yes_no(enabled));
}

static int frames[3];
static SDL_Window *animated;
static void SDLCALL count_and_stop(void *userdata)
{
    int *count = (int *)userdata;
    (*count)++;
    if (*count == 30 && count == &frames[0]) {
        SDL_SetiOSAnimationCallback(animated, 1, NULL, NULL);
    }
    if (*count == 10 && count == &frames[1]) {
        SDL_SetiOSAnimationCallback(animated, 1, NULL, NULL);
    }
}

static void wait_frames(int *count, int n)
{
    Uint64 end = SDL_GetTicks() + 10000;
    List ignored;
    while (*count < n && SDL_GetTicks() < end) {
        memset(&ignored, 0, sizeof(ignored));
        take_queued(&ignored);
        SDL_Delay(5);
    }
    for (int i = 0; i < 20; i++) {
        memset(&ignored, 0, sizeof(ignored));
        take_queued(&ignored);
        SDL_Delay(10);
    }
}

static void path_exists(const char *label, const char *path)
{
    SDL_PathInfo info;
    if (SDL_GetPathInfo(path, &info)) {
        t("%s: %s: type=%d size=%llu", label, home_path(path), (int)info.type, (unsigned long long)info.size);
    } else {
        t("%s: %s: error: %s", label, home_path(path), SDL_GetError());
    }
}

static int app_main(int argc, char *argv[])
{
    (void)argc; (void)argv;
    List l;
    printf("uikit_tvos_ref: %.3f s: main\n", elapsed());
    t("platform: %s", SDL_GetPlatform());
    bool b = SDL_Init(SDL_INIT_VIDEO | SDL_INIT_AUDIO);
    t("init video and audio: %s", ok(b));
    t("video driver: %s; audio driver: %s", SDL_GetCurrentVideoDriver(), SDL_GetCurrentAudioDriver());
    memset(&l, 0, sizeof(l));
    for (int i = 0; i < SDL_GetNumVideoDrivers(); i++) list_add(&l, SDL_GetVideoDriver(i));
    t("video drivers: %s", list_str(&l));
    memset(&l, 0, sizeof(l));
    /* (without the OpenGL ES 1 renderer, which the Rust translation hasn't) */
    for (int i = 0; i < SDL_GetNumRenderDrivers(); i++) {
        if (strcmp(SDL_GetRenderDriver(i), "opengles") != 0) list_add(&l, SDL_GetRenderDriver(i));
    }
    t("render drivers: %s", list_str(&l));
    memset(&l, 0, sizeof(l));
    for (int i = 0; i < SDL_GetNumCameraDrivers(); i++) list_add(&l, SDL_GetCameraDriver(i));
    t("camera drivers: %s", list_str(&l));
    memset(&l, 0, sizeof(l)); take_queued(&l);
    t("events: %s", list_str(&l));

    List ignored;
    Uint64 end = SDL_GetTicks() + 30000;
    while ([UIApplication sharedApplication].applicationState != UIApplicationStateActive && SDL_GetTicks() < end) {
        memset(&ignored, 0, sizeof(ignored)); take_queued(&ignored);
        SDL_Delay(10);
    }
    for (int i = 0; i < 10; i++) {
        memset(&ignored, 0, sizeof(ignored)); take_queued(&ignored);
        SDL_Delay(10);
    }
    t("application state: %d", (int)[UIApplication sharedApplication].applicationState);
    t("idiom: %ld", (long)[UIDevice currentDevice].userInterfaceIdiom);

    int secs = 0, pct = 0;
    SDL_PowerState ps = SDL_GetPowerInfo(&secs, &pct);
    t("power: state=%d secs=%d pct=%d", (int)ps, secs, pct);

    /* The file system: the pref path is in Caches, and there are no user folders */
    char *pref = SDL_GetPrefPath("org.libsdl", "tvos test");
    t("pref path: %s", home_path(pref));
    path_exists("pref path", pref);
    SDL_free(pref);
    pref = SDL_GetPrefPath(NULL, "tvos test");
    t("pref path without org: %s", home_path(pref));
    SDL_free(pref);
    pref = SDL_GetPrefPath("", "");
    t("pref path empty: %s", home_path(pref));
    SDL_free(pref);
    const char *base = SDL_GetBasePath();
    t("base path: ends with .app/: %s", yes_no(base && strlen(base) > 5 && strcmp(base + strlen(base) - 5, ".app/") == 0));
    SDL_Folder folders[] = { SDL_FOLDER_HOME, SDL_FOLDER_DOCUMENTS, SDL_FOLDER_SAVEDGAMES, SDL_FOLDER_DESKTOP, SDL_FOLDER_DOWNLOADS };
    const char *folder_names[] = { "home", "documents", "saved games", "desktop", "downloads" };
    for (size_t i = 0; i < SDL_arraysize(folders); i++) {
        t("user folder %s: %s", folder_names[i], home_path(SDL_GetUserFolder(folders[i])));
    }
    SDL_IOStream *io = SDL_IOFromFile("tvos-relative.txt", "w");
    t("relative file written: %s", ok(io && SDL_WriteIO(io, "tvOS\n", 5) == 5 && SDL_CloseIO(io)));
    pref = SDL_GetPrefPath("", "");
    char relative[1024];
    SDL_snprintf(relative, sizeof(relative), "%stvos-relative.txt", pref ? pref : "");
    path_exists("relative file", relative);
    t("relative file removed: %s", ok(SDL_RemovePath("tvos-relative.txt")));
    path_exists("relative file", relative);
    SDL_free(pref);

    /* Displays */
    displays_state("start");
    struct { const char *label; NSNotificationName name; } posts[] = {
        { "connected", UIScreenDidConnectNotification },
        { "disconnected", UIScreenDidDisconnectNotification },
        { "connected again", UIScreenDidConnectNotification },
    };
    for (size_t i = 0; i < SDL_arraysize(posts); i++) {
        [[NSNotificationCenter defaultCenter] postNotificationName:posts[i].name object:[UIScreen mainScreen]];
        memset(&l, 0, sizeof(l)); take_queued(&l);
        t("%s: events: %s", posts[i].label, list_str(&l));
        displays_state(posts[i].label);
    }

    /* A: a plain window */
    SDL_Window *a = SDL_CreateWindow("A", 320, 480, 0);
    t("A: create 320x480: %s", ok(a != NULL));
    name_window(a, "A");
    events("A: created", 500);
    window_state("A: created", a);
    SDL_Window *other = SDL_CreateWindow("other", 100, 100, 0);
    t("A: another window: %s", ok(other != NULL));
    events("A: another window", 200);
    t("A: hide: %s", ok(SDL_HideWindow(a)));
    events("A: hidden", 500);
    window_state("A: hidden", a);
    t("A: show: %s", ok(SDL_ShowWindow(a)));
    events("A: shown", 500);
    window_state("A: shown", a);
    t("A: title: %s", ok(SDL_SetWindowTitle(a, "Title A")));
    t("A: title: window=%s view controller=%s", SDL_GetWindowTitle(a), vc_of(a).title.UTF8String);
    t("A: borderless: %s", ok(SDL_SetWindowBordered(a, false)));
    events("A: borderless", 1000);
    window_state("A: borderless", a);
    t("A: bordered: %s", ok(SDL_SetWindowBordered(a, true)));
    events("A: bordered", 1000);
    t("A: fullscreen: %s", ok(SDL_SetWindowFullscreen(a, true)));
    t("A: sync: %s", ok(SDL_SyncWindow(a)));
    events("A: fullscreen", 1000);
    window_state("A: fullscreen", a);
    t("A: windowed: %s", ok(SDL_SetWindowFullscreen(a, false)));
    t("A: sync: %s", ok(SDL_SyncWindow(a)));
    events("A: windowed", 1000);
    window_state("A: windowed", a);
    int count = 0;
    SDL_DisplayMode **modes = SDL_GetFullscreenDisplayModes(SDL_GetPrimaryDisplay(), &count);
    for (int i = 0; modes && i < count; i++) {
        char label[64];
        SDL_snprintf(label, sizeof(label), "A: exclusive %dx%d", modes[i]->w, modes[i]->h);
        t("%s: set mode: %s", label, ok(SDL_SetWindowFullscreenMode(a, modes[i])));
        t("%s: fullscreen: %s", label, ok(SDL_SetWindowFullscreen(a, true)));
        t("%s: sync: %s", label, ok(SDL_SyncWindow(a)));
        events(label, 1000);
        window_state(label, a);
        t("%s: windowed: %s", label, ok(SDL_SetWindowFullscreen(a, false)));
        t("%s: sync: %s", label, ok(SDL_SyncWindow(a)));
        t("%s: unset mode: %s", label, ok(SDL_SetWindowFullscreenMode(a, NULL)));
        SDL_snprintf(label, sizeof(label), "A: exclusive %dx%d left", modes[i]->w, modes[i]->h);
        events(label, 1000);
    }
    SDL_free(modes);
    controller_ui_events("A", a, "1");
    controller_ui_events("A", a, "0");
    controller_ui_events("A", a, "1");
    controller_ui_events("A", a, NULL);
    SDL_SetHint(SDL_HINT_ORIENTATIONS, "LandscapeLeft Portrait");
    t("A: resize to 100x100: %s", ok(SDL_SetWindowSize(a, 100, 100)));
    events("A: resized", 500);
    window_state("A: resized", a);
    SDL_SetHint(SDL_HINT_ORIENTATIONS, NULL);
    SDL_DestroyWindow(a);
    t("A: destroyed");
    events("A: destroyed", 500);

    /* B: resizable, high density, made hidden, with the hint for the controllers' UI events */
    SDL_SetHint(SDL_HINT_APPLE_TV_CONTROLLER_UI_EVENTS, "1");
    SDL_Window *bw = SDL_CreateWindow("B", 640, 200, SDL_WINDOW_RESIZABLE | SDL_WINDOW_HIGH_PIXEL_DENSITY | SDL_WINDOW_HIDDEN);
    t("B: create 640x200 resizable high-density hidden: %s", ok(bw != NULL));
    name_window(bw, "B");
    events("B: created", 500);
    window_state("B: created", bw);
    t("B: show: %s", ok(SDL_ShowWindow(bw)));
    events("B: shown", 500);
    window_state("B: shown", bw);
    SDL_SetHint(SDL_HINT_APPLE_TV_CONTROLLER_UI_EVENTS, NULL);
    animated = bw;
    t("B: animation every frame: %s", ok(SDL_SetiOSAnimationCallback(bw, 1, count_and_stop, &frames[0])));
    wait_frames(&frames[0], 30);
    t("B: animation every frame: %d frames", frames[0]);
    t("B: animation every 3rd frame: %s", ok(SDL_SetiOSAnimationCallback(bw, 3, count_and_stop, &frames[1])));
    wait_frames(&frames[1], 10);
    t("B: animation every 3rd frame: %d frames", frames[1]);
    t("B: animation stopped: %s", ok(SDL_SetiOSAnimationCallback(bw, 1, NULL, NULL)));
    events("B: animation", 200);

    /* The clipboard (no pasteboard on tvOS) */
    t("clipboard: has text: %s", yes_no(SDL_HasClipboardText()));
    t("clipboard: set: %s", ok(SDL_SetClipboardText("tvOS text \xc3\xa9")));
    char *text = SDL_GetClipboardText();
    t("clipboard: has text: %s text=%s", yes_no(SDL_HasClipboardText()), text);
    SDL_free(text);
    t("clipboard: set empty: %s", ok(SDL_SetClipboardText("")));
    text = SDL_GetClipboardText();
    t("clipboard: has text: %s text=%s", yes_no(SDL_HasClipboardText()), text);
    SDL_free(text);
    events("clipboard", 200);

    SDL_DestroyWindow(bw);
    t("B: destroyed");
    events("B: destroyed", 500);

    /* C: fullscreen from the start */
    SDL_Window *c = SDL_CreateWindow("C", 200, 300, SDL_WINDOW_FULLSCREEN);
    t("C: create 200x300 fullscreen: %s", ok(c != NULL));
    name_window(c, "C");
    events("C: created", 1000);
    window_state("C: created", c);
    SDL_DestroyWindow(c);
    t("C: destroyed");
    events("C: destroyed", 500);

    SDL_Quit();
    t("quit");
    exit_line(0);
    exit(0);
}

int main(int argc, char *argv[])
{
    elapsed();
    int status = SDL_RunApp(argc, argv, app_main, NULL);
    printf("uikit_tvos_ref: SDL_RunApp returned %d\n", status);
    exit_line(1);
    return 1;
}
