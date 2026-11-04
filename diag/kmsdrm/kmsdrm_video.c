/* diag only: the C twin of sdl3/tests/kmsdrm_video.rs. */
#include "kms_common.h"

static SDL_Window *the_window;

static void window_events(void)
{
    SDL_Event e;
    SDL_PumpEvents();
    while (SDL_PollEvent(&e)) {
        if (e.type >= SDL_EVENT_WINDOW_FIRST && e.type <= SDL_EVENT_WINDOW_LAST) {
            line("window event 0x%x %d %d%s", e.type, e.window.data1, e.window.data2,
                 e.window.windowID == SDL_GetWindowID(the_window) ? "" : " (another window)");
        } else if (e.type >= SDL_EVENT_DISPLAY_FIRST && e.type <= SDL_EVENT_DISPLAY_LAST) {
            line("display event 0x%x %d %d", e.type, e.display.data1, e.display.data2);
        }
    }
}

static void scanout(void)
{
    ask("drm state");
    ask("drm crc");
    ask("drm pixels 0 0 320 240 799 599 1023 767");
}

static void frame(float r, float g, float b)
{
    gl_clear(r, g, b);
    line("swap (%.1f, %.1f, %.1f): %s", r, g, b, result(SDL_GL_SwapWindow(the_window)));
    window_events();
    scanout();
}

static void window_state(void)
{
    int w = 0, h = 0, pw = 0, ph = 0;
    const SDL_DisplayMode *m;
    SDL_GetWindowSize(the_window, &w, &h);
    SDL_GetWindowSizeInPixels(the_window, &pw, &ph);
    m = SDL_GetWindowFullscreenMode(the_window);
    line("window: flags 0x%llx size %dx%d pixels %dx%d fullscreen mode %s",
         (unsigned long long)SDL_GetWindowFlags(the_window), w, h, pw, ph, m ? mode_line(m) : "none");
}

static void displays(void)
{
    int count = 0;
    SDL_DisplayID *ids = SDL_GetDisplays(&count);
    line("displays: %d", count);
    for (int i = 0; i < count; i++) {
        SDL_DisplayID id = ids[i];
        SDL_Rect bounds = { 0 }, usable = { 0 };
        const SDL_DisplayMode *m;
        SDL_PropertiesID props;
        char orientation[64];
        int nmodes = 0;
        SDL_DisplayMode **modes;
        static const int sizes[][2] = { { 640, 480 }, { 800, 600 }, { 1000, 700 }, { 1024, 768 }, { 5000, 5000 } };

        SDL_GetDisplayBounds(id, &bounds);
        SDL_GetDisplayUsableBounds(id, &usable);
        line("display %d: \"%s\" bounds %d,%d %dx%d usable %d,%d %dx%d%s", i,
             SDL_GetDisplayName(id) ? SDL_GetDisplayName(id) : "", bounds.x, bounds.y, bounds.w,
             bounds.h, usable.x, usable.y, usable.w, usable.h,
             id == SDL_GetPrimaryDisplay() ? " (primary)" : "");
        m = SDL_GetDesktopDisplayMode(id);
        if (m) {
            line("desktop mode: %s", mode_line(m));
        }
        m = SDL_GetCurrentDisplayMode(id);
        if (m) {
            line("current mode: %s", mode_line(m));
        }
        props = SDL_GetDisplayProperties(id);
        if (props && SDL_GetPropertyType(props, SDL_PROP_DISPLAY_KMSDRM_PANEL_ORIENTATION_NUMBER) ==
                         SDL_PROPERTY_TYPE_NUMBER) {
            snprintf(orientation, sizeof(orientation), "Some(%lld)",
                     (long long)SDL_GetNumberProperty(props, SDL_PROP_DISPLAY_KMSDRM_PANEL_ORIENTATION_NUMBER, 0));
        } else {
            snprintf(orientation, sizeof(orientation), "None");
        }
        line("content scale %.2f, orientation natural %d current %d, panel orientation %s",
             SDL_GetDisplayContentScale(id), (int)SDL_GetNaturalDisplayOrientation(id),
             (int)SDL_GetCurrentDisplayOrientation(id), orientation);
        modes = SDL_GetFullscreenDisplayModes(id, &nmodes);
        line("fullscreen modes: %d", nmodes);
        for (int j = 0; j < nmodes; j++) {
            line("mode: %s", mode_line(modes[j]));
        }
        SDL_free(modes);
        for (int j = 0; j < 5; j++) {
            SDL_DisplayMode closest;
            if (SDL_GetClosestFullscreenDisplayMode(id, sizes[j][0], sizes[j][1], 0.0f, false, &closest)) {
                line("closest to %dx%d: %s", sizes[j][0], sizes[j][1], mode_line(&closest));
            } else {
                line("closest to %dx%d: error: %s", sizes[j][0], sizes[j][1], SDL_GetError());
            }
        }
    }
    SDL_free(ids);
}

int main(void)
{
    SDL_GLContext context;
    SDL_PropertiesID props;
    SDL_DisplayID display;
    SDL_DisplayMode mode;
    const SDL_DisplayMode *cm;
    char dev_index[64];

    begin();
    ask("drm state");

    line("-- init");
    if (!init_video()) {
        return finish("kmsdrm_video");
    }
    displays();

    line("-- window");
    the_window = SDL_CreateWindow("kmsdrm_video", 640, 480, 0);
    line("window: %s", result(the_window != NULL));
    if (!the_window) {
        quit_video();
        return finish("kmsdrm_video");
    }
    window_state();
    props = SDL_GetWindowProperties(the_window);
    if (SDL_GetPropertyType(props, SDL_PROP_WINDOW_KMSDRM_DEVICE_INDEX_NUMBER) == SDL_PROPERTY_TYPE_NUMBER) {
        snprintf(dev_index, sizeof(dev_index), "Some(%lld)",
                 (long long)SDL_GetNumberProperty(props, SDL_PROP_WINDOW_KMSDRM_DEVICE_INDEX_NUMBER, 0));
    } else {
        snprintf(dev_index, sizeof(dev_index), "None");
    }
    line("properties: device index %s, drm fd set %s, gbm device set %s", dev_index,
         SDL_GetPropertyType(props, SDL_PROP_WINDOW_KMSDRM_DRM_FD_NUMBER) == SDL_PROPERTY_TYPE_NUMBER &&
                 SDL_GetNumberProperty(props, SDL_PROP_WINDOW_KMSDRM_DRM_FD_NUMBER, -1) >= 0
             ? "true"
             : "false",
         SDL_GetPointerProperty(props, SDL_PROP_WINDOW_KMSDRM_GBM_DEVICE_POINTER, NULL) ? "true" : "false");
    window_events();
    ask("drm state");

    context = SDL_GL_CreateContext(the_window);
    line("context: %s", result(context != NULL));
    if (!context) {
        SDL_DestroyWindow(the_window);
        quit_video();
        return finish("kmsdrm_video");
    }
    {
        int interval = 0;
        if (SDL_GL_GetSwapInterval(&interval)) {
            line("swap interval: %d", interval);
        } else {
            line("swap interval: error: %s", SDL_GetError());
        }
    }
    frame(1.0f, 0.0f, 0.0f);
    frame(0.0f, 1.0f, 0.0f);
    window_state();

    line("-- fullscreen 800x600");
    display = SDL_GetDisplayForWindow(the_window);
    if (SDL_GetClosestFullscreenDisplayMode(display, 800, 600, 0.0f, false, &mode)) {
        line("set fullscreen mode: %s", result(SDL_SetWindowFullscreenMode(the_window, &mode)));
    } else {
        line("set fullscreen mode: error: %s", SDL_GetError());
    }
    line("set fullscreen: %s", result(SDL_SetWindowFullscreen(the_window, true)));
    window_events();
    window_state();
    cm = SDL_GetCurrentDisplayMode(display);
    if (cm) {
        line("current mode: %s", mode_line(cm));
    }
    frame(0.0f, 0.0f, 1.0f);

    line("-- leave fullscreen");
    line("set fullscreen false: %s", result(SDL_SetWindowFullscreen(the_window, false)));
    window_events();
    window_state();
    cm = SDL_GetCurrentDisplayMode(display);
    if (cm) {
        line("current mode: %s", mode_line(cm));
    }
    frame(1.0f, 1.0f, 0.0f);

    line("-- size 1024x768");
    line("set size: %s", result(SDL_SetWindowSize(the_window, 1024, 768)));
    window_events();
    window_state();
    frame(0.0f, 1.0f, 1.0f);

    line("-- destroyed");
    SDL_GL_DestroyContext(context);
    SDL_DestroyWindow(the_window);
    line("window destroyed");
    ask("drm state");
    quit_video();
    ask("drm state");
    ask("drm master");

    line("-- without DRM master");
    ask("drm master hold");
    SDL_SetHintWithPriority(SDL_HINT_KMSDRM_DEVICE_INDEX, "", SDL_HINT_OVERRIDE);
    if (init_video()) {
        quit_video();
    }
    SDL_ResetHint(SDL_HINT_KMSDRM_DEVICE_INDEX);
    SDL_SetHint(SDL_HINT_KMSDRM_REQUIRE_DRM_MASTER, "0");
    if (init_video()) {
        displays();
        the_window = SDL_CreateWindow("kmsdrm_video", 1024, 768, 0);
        line("window: %s", result(the_window != NULL));
        if (the_window) {
            context = SDL_GL_CreateContext(the_window);
            line("context: %s", result(context != NULL));
            if (context) {
                gl_clear(1.0f, 0.0f, 1.0f);
                line("swap: %s", result(SDL_GL_SwapWindow(the_window)));
                SDL_GL_DestroyContext(context);
            }
            SDL_DestroyWindow(the_window);
            line("window destroyed");
        }
        quit_video();
    }
    SDL_ResetHint(SDL_HINT_KMSDRM_REQUIRE_DRM_MASTER);
    ask("drm state");
    ask("drm master release");
    ask("drm state");

    return finish("kmsdrm_video");
}
