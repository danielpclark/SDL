/* diag only: the C twin of sdl3/tests/kmsdrm_input.rs. */
#include "kms_common.h"

#define KEYBOARD "SDL3 kmsdrm keyboard"
#define MOUSE "SDL3 kmsdrm mouse"

typedef struct { Uint32 id; char name[128]; } Named;
static Named keyboards[256], mice[256];
static int nkeyboards, nmice;
static SDL_WindowID window_id;

static const char *lookup(Named *list, int n, Uint32 id)
{
    for (int i = n - 1; i >= 0; i--) {
        if (list[i].id == id) {
            return list[i].name;
        }
    }
    return NULL;
}

static bool ours(const char *name)
{
    return name && strncmp(name, "SDL3 kmsdrm", 11) == 0;
}

static const char *win(SDL_WindowID id)
{
    return id == window_id ? "window" : id == 0 ? "none" : "another";
}

/* The event's line, if it's one of the traced ones. */
static bool describe(const SDL_Event *e, char *out, size_t size)
{
    const char *name;
    switch (e->type) {
    case SDL_EVENT_KEYBOARD_ADDED:
    case SDL_EVENT_KEYBOARD_REMOVED:
        if (e->type == SDL_EVENT_KEYBOARD_ADDED && nkeyboards < 256) {
            const char *n = SDL_GetKeyboardNameForID(e->kdevice.which);
            keyboards[nkeyboards].id = e->kdevice.which;
            snprintf(keyboards[nkeyboards++].name, 128, "%s", n ? n : "");
        }
        name = lookup(keyboards, nkeyboards, e->kdevice.which);
        if (!ours(name)) return false;
        snprintf(out, size, "keyboard %s: %s", e->type == SDL_EVENT_KEYBOARD_ADDED ? "added" : "removed", name);
        return true;
    case SDL_EVENT_MOUSE_ADDED:
    case SDL_EVENT_MOUSE_REMOVED:
        if (e->type == SDL_EVENT_MOUSE_ADDED && nmice < 256) {
            const char *n = SDL_GetMouseNameForID(e->mdevice.which);
            mice[nmice].id = e->mdevice.which;
            snprintf(mice[nmice++].name, 128, "%s", n ? n : "");
        }
        name = lookup(mice, nmice, e->mdevice.which);
        if (!ours(name)) return false;
        snprintf(out, size, "mouse %s: %s", e->type == SDL_EVENT_MOUSE_ADDED ? "added" : "removed", name);
        return true;
    case SDL_EVENT_KEY_DOWN:
    case SDL_EVENT_KEY_UP:
        name = lookup(keyboards, nkeyboards, e->key.which);
        if (!ours(name)) return false;
        snprintf(out, size, "key %s %s key %s mod 0x%04x %s", e->key.down ? "down" : "up",
                 SDL_GetScancodeName(e->key.scancode), SDL_GetKeyName(e->key.key), e->key.mod,
                 win(e->key.windowID));
        return true;
    case SDL_EVENT_TEXT_INPUT:
        snprintf(out, size, "text \"%s\" %s", e->text.text, win(e->text.windowID));
        return true;
    case SDL_EVENT_MOUSE_MOTION:
        name = e->motion.which == 0 ? "global" : lookup(mice, nmice, e->motion.which);
        if (e->motion.which != 0 && !ours(name)) return false;
        snprintf(out, size, "motion %s at (%.1f, %.1f) rel (%.1f, %.1f) %s", name, e->motion.x, e->motion.y,
                 e->motion.xrel, e->motion.yrel, win(e->motion.windowID));
        return true;
    case SDL_EVENT_MOUSE_BUTTON_DOWN:
    case SDL_EVENT_MOUSE_BUTTON_UP:
        name = e->button.which == 0 ? "global" : lookup(mice, nmice, e->button.which);
        if (e->button.which != 0 && !ours(name)) return false;
        snprintf(out, size, "button %s %d %s at (%.1f, %.1f) %s", e->button.down ? "down" : "up", e->button.button,
                 name, e->button.x, e->button.y, win(e->button.windowID));
        return true;
    default:
        if (e->type >= SDL_EVENT_WINDOW_FIRST && e->type <= SDL_EVENT_WINDOW_LAST) {
            snprintf(out, size, "window event 0x%x %d %d %s", e->type, e->window.data1, e->window.data2,
                     win(e->window.windowID));
            return true;
        }
        return false;
    }
}

static void drain(void)
{
    SDL_Event e;
    char buf[512];
    while (SDL_PollEvent(&e)) {
        if (describe(&e, buf, sizeof(buf))) {
            line("%s", buf);
        }
    }
}

static void pump(void)
{
    SDL_PumpEvents();
    drain();
}

typedef struct { const char *lines[2]; bool seen[2]; int n; } Wait;

static bool wait_done(void *data)
{
    Wait *w = data;
    SDL_Event e;
    char buf[512];
    while (SDL_PollEvent(&e)) {
        if (describe(&e, buf, sizeof(buf))) {
            for (int i = 0; i < w->n; i++) {
                if (strcmp(buf, w->lines[i]) == 0) {
                    w->seen[i] = true;
                }
            }
            line("%s", buf);
        }
    }
    for (int i = 0; i < w->n; i++) {
        if (!w->seen[i]) return false;
    }
    return true;
}

static void wait_for(const char *a, const char *b)
{
    Wait w = { { a, b }, { false, false }, 2 };
    if (!pump_until(wait_done, &w)) {
        char missing[512] = "[";
        for (int i = 0; i < w.n; i++) {
            if (!w.seen[i]) {
                if (strlen(missing) > 1) strcat(missing, ", ");
                strcat(missing, "\"");
                strcat(missing, w.lines[i]);
                strcat(missing, "\"");
            }
        }
        strcat(missing, "]");
        line("timeout: no %s in 10 s", missing);
    }
}

static void send(const char *device, const char *events)
{
    char req[2048];
    snprintf(req, sizeof(req), "uinput events '%s' %s", device, events);
    request(req);
    for (int i = 0; i < nreplies; i++) {
        if (strncmp(replies[i], "uinput: sent ", 13) != 0) {
            line("%s", replies[i]);
        }
    }
    pump();
}

static void mouse_state(void)
{
    float x, y;
    SDL_MouseButtonFlags b = SDL_GetMouseState(&x, &y);
    line("mouse: at (%.1f, %.1f) buttons 0x%x", x, y, b);
}

static char *tap(char *buf, const char *key)
{
    sprintf(buf, "EV_KEY %s 1 EV_SYN SYN_REPORT 0 EV_KEY %s 0 EV_SYN SYN_REPORT 0", key, key);
    return buf;
}

static bool away_done(void *data)
{
    (void)data;
    request("vt active");
    return nreplies > 0 && strcmp(replies[0], "vt: active " OTHER_VT) == 0;
}

int main(void)
{
    SDL_Window *window;
    SDL_GLContext context;
    char a[256], b[256], buf[1024];

    begin();
    ask("vt active");
    ask("vt modes");

    line("-- init");
    if (!init_video()) {
        return finish("kmsdrm_input");
    }
    window = SDL_CreateWindow("kmsdrm_input", 1024, 768, 0);
    line("window: %s", result(window != NULL));
    if (!window) {
        SDL_Quit();
        return finish("kmsdrm_input");
    }
    window_id = SDL_GetWindowID(window);
    SDL_StartTextInput(window);
    drain();
    context = SDL_GL_CreateContext(window);
    line("context: %s", result(context != NULL));
    gl_clear(0.0f, 0.0f, 1.0f);
    line("swap: %s", result(SDL_GL_SwapWindow(window)));
    pump();
    ask("vt modes");
    ask("drm master");

    line("-- devices");
    ask("uinput add '" KEYBOARD "' keyboard");
    ask("uinput add '" MOUSE "' mouse");
    wait_for("keyboard added: " KEYBOARD, "mouse added: " MOUSE);

    line("-- keys and mouse");
    snprintf(buf, sizeof(buf), "%s %s", tap(a, "KEY_A"), tap(b, "KEY_1"));
    send(KEYBOARD, buf);
    snprintf(buf, sizeof(buf), "EV_KEY KEY_LEFTSHIFT 1 EV_SYN SYN_REPORT 0 %s EV_KEY KEY_LEFTSHIFT 0 EV_SYN SYN_REPORT 0",
             tap(a, "KEY_B"));
    send(KEYBOARD, buf);
    send(MOUSE, "EV_REL REL_X 40 EV_REL REL_Y 30 EV_SYN SYN_REPORT 0");
    mouse_state();
    ask("drm state");
    send(MOUSE, "EV_KEY BTN_LEFT 1 EV_SYN SYN_REPORT 0 EV_KEY BTN_LEFT 0 EV_SYN SYN_REPORT 0");
    mouse_state();

    line("-- vt switch away");
    ask("vt activate " OTHER_VT);
    pump_until(away_done, NULL);
    for (int i = 0; i < nreplies; i++) {
        line("%s", replies[i]);
    }
    drain();
    ask("vt modes");
    ask("drm master");
    ask("drm state");

    line("-- vt switch back");
    ask("vt switch " VT);
    pump();
    ask("vt modes");
    ask("drm master");
    gl_clear(0.0f, 1.0f, 0.0f);
    line("swap: %s", result(SDL_GL_SwapWindow(window)));
    pump();
    ask("drm state");
    ask("drm crc");
    send(KEYBOARD, tap(a, "KEY_Z"));
    send(MOUSE, "EV_REL REL_X -20 EV_SYN SYN_REPORT 0");
    mouse_state();
    ask("drm state");

    SDL_WarpMouseInWindow(window, 0.0f, 0.0f);
    pump();
    ask("drm state");

    line("-- removed");
    ask("uinput remove '" KEYBOARD "'");
    ask("uinput remove '" MOUSE "'");
    wait_for("keyboard removed: " KEYBOARD, "mouse removed: " MOUSE);
    SDL_GL_DestroyContext(context);
    SDL_DestroyWindow(window);
    line("window destroyed");
    SDL_Quit();
    line("quit");
    ask("vt modes");
    ask("drm state");

    return finish("kmsdrm_input");
}
