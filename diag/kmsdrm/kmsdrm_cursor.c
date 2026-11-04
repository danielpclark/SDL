/* diag only: the C twin of sdl3/tests/kmsdrm_cursor.rs. */
#include "kms_common.h"

static SDL_Window *the_window;

static const char *which_window(SDL_WindowID id)
{
    return id == SDL_GetWindowID(the_window) ? "window" : "another";
}

static void mouse_events(void)
{
    SDL_Event e;
    float x, y;
    SDL_MouseButtonFlags buttons;
    SDL_Window *focus;
    SDL_PumpEvents();
    while (SDL_PollEvent(&e)) {
        if (e.type == SDL_EVENT_MOUSE_MOTION) {
            line("motion %u at (%.1f, %.1f) rel (%.1f, %.1f) %s", e.motion.which, e.motion.x, e.motion.y,
                 e.motion.xrel, e.motion.yrel, which_window(e.motion.windowID));
        } else if (e.type >= SDL_EVENT_WINDOW_FIRST && e.type <= SDL_EVENT_WINDOW_LAST) {
            line("window event 0x%x %d %d %s", e.type, e.window.data1, e.window.data2,
                 which_window(e.window.windowID));
        }
    }
    buttons = SDL_GetMouseState(&x, &y);
    focus = SDL_GetMouseFocus();
    line("mouse: at (%.1f, %.1f) buttons 0x%x focus %s cursor visible %s", x, y, buttons,
         focus == the_window ? "window" : focus ? "another" : "none", SDL_CursorVisible() ? "true" : "false");
}

static void cursor(void)
{
    ask("drm state");
    ask("drm crc");
}

static SDL_Cursor *color_cursor(void)
{
    SDL_Surface *s = SDL_CreateSurface(32, 32, SDL_PIXELFORMAT_ARGB8888);
    const SDL_PixelFormatDetails *d;
    SDL_Rect r1 = { 2, 2, 28, 28 }, r2 = { 8, 8, 16, 16 };
    SDL_Cursor *c;
    if (!s) {
        return NULL;
    }
    d = SDL_GetPixelFormatDetails(s->format);
    SDL_FillSurfaceRect(s, NULL, SDL_MapRGBA(d, NULL, 255, 255, 255, 255));
    SDL_FillSurfaceRect(s, &r1, SDL_MapRGBA(d, NULL, 255, 0, 0, 255));
    SDL_FillSurfaceRect(s, &r2, SDL_MapRGBA(d, NULL, 0, 0, 255, 128));
    c = SDL_CreateColorCursor(s, 5, 7);
    SDL_DestroySurface(s);
    return c;
}

int main(void)
{
    SDL_GLContext context;
    SDL_Cursor *custom, *c;

    begin();

    line("-- init");
    if (!init_video()) {
        return finish("kmsdrm_cursor");
    }
    line("warp global before a window: %s", result(SDL_WarpMouseGlobal(10.0f, 10.0f)));
    c = color_cursor();
    line("color cursor before a window: %s", result(c != NULL));
    line("default cursor before a window: %s", SDL_GetDefaultCursor() ? "true" : "false");

    line("-- window");
    the_window = SDL_CreateWindow("kmsdrm_cursor", 1024, 768, 0);
    line("window: %s", result(the_window != NULL));
    if (!the_window) {
        SDL_Quit();
        return finish("kmsdrm_cursor");
    }
    line("default cursor: %s, current is default: %s", SDL_GetDefaultCursor() ? "true" : "false",
         SDL_GetCursor() == SDL_GetDefaultCursor() ? "true" : "false");
    mouse_events();
    ask("drm state");
    context = SDL_GL_CreateContext(the_window);
    line("context: %s", result(context != NULL));
    gl_clear(0.0f, 0.0f, 0.5f);
    line("swap: %s", result(SDL_GL_SwapWindow(the_window)));
    mouse_events();
    cursor();

    line("-- warps");
    SDL_WarpMouseInWindow(the_window, 100.0f, 200.0f);
    mouse_events();
    cursor();
    line("warp global: %s", result(SDL_WarpMouseGlobal(300.5f, 400.5f)));
    mouse_events();
    cursor();
    SDL_WarpMouseInWindow(the_window, 1023.0f, 767.0f);
    mouse_events();
    ask("drm state");

    line("-- color cursor");
    SDL_WarpMouseInWindow(the_window, 400.0f, 300.0f);
    mouse_events();
    custom = color_cursor();
    line("color cursor: %s", result(custom != NULL));
    if (custom) {
        line("set cursor: %s", result(SDL_SetCursor(custom)));
        cursor();
        SDL_WarpMouseInWindow(the_window, 50.0f, 60.0f);
        mouse_events();
        cursor();
    }

    line("-- hidden and shown");
    SDL_HideCursor();
    mouse_events();
    cursor();
    SDL_ShowCursor();
    mouse_events();
    cursor();

    line("-- relative mode");
    line("relative mode on: %s", result(SDL_SetWindowRelativeMouseMode(the_window, true)));
    mouse_events();
    ask("drm state");
    line("relative mode off: %s", result(SDL_SetWindowRelativeMouseMode(the_window, false)));
    mouse_events();
    ask("drm state");

    line("-- system cursor");
    c = SDL_CreateSystemCursor(SDL_SYSTEM_CURSOR_POINTER);
    line("system cursor: %s", result(c != NULL));

    line("-- destroyed");
    if (custom) {
        SDL_DestroyCursor(custom);
        line("cursor destroyed, current is default: %s", SDL_GetCursor() == SDL_GetDefaultCursor() ? "true" : "false");
    }
    mouse_events();
    cursor();
    SDL_GL_DestroyContext(context);
    SDL_DestroyWindow(the_window);
    line("window destroyed");
    ask("drm state");
    SDL_Quit();
    line("quit");
    ask("drm state");

    return finish("kmsdrm_cursor");
}
