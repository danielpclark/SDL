/* diag only: the C twin of sdl3/tests/kmsdrm_displays.rs. */
#include "kms_common.h"

#define PIXELS "drm pixels 0 0 320 240 639 479 1023 767"

static SDL_Window *wins[2];
static int nwins;

static void window_events(void)
{
    SDL_Event e;
    SDL_PumpEvents();
    while (SDL_PollEvent(&e)) {
        if (e.type >= SDL_EVENT_WINDOW_FIRST && e.type <= SDL_EVENT_WINDOW_LAST) {
            char which[16] = "?";
            for (int i = 0; i < nwins; i++) {
                if (wins[i] && SDL_GetWindowID(wins[i]) == e.window.windowID) {
                    snprintf(which, sizeof(which), "%d", i);
                }
            }
            line("window %s event 0x%x %d %d", which, e.type, e.window.data1, e.window.data2);
        } else if (e.type >= SDL_EVENT_DISPLAY_FIRST && e.type <= SDL_EVENT_DISPLAY_LAST) {
            line("display event 0x%x %d %d", e.type, e.display.data1, e.display.data2);
        }
    }
}

static void displays(int the_card)
{
    int count = 0;
    SDL_DisplayID *ids = SDL_GetDisplays(&count);
    line("displays: %d", count);
    for (int i = 0; i < count; i++) {
        SDL_Rect b = { 0 };
        int nmodes = 0;
        SDL_DisplayMode **modes = SDL_GetFullscreenDisplayModes(ids[i], &nmodes);
        const SDL_DisplayMode *m;
        SDL_free(modes);
        SDL_GetDisplayBounds(ids[i], &b);
        line("display %d: \"%s\" bounds %d,%d %dx%d%s, %d fullscreen modes", i,
             connector_name(the_card, SDL_GetDisplayName(ids[i])), b.x, b.y, b.w, b.h,
             ids[i] == SDL_GetPrimaryDisplay() ? " (primary)" : "", nmodes);
        m = SDL_GetDesktopDisplayMode(ids[i]);
        if (m) {
            line("display %d: desktop mode %s", i, mode_line(m));
        }
    }
    SDL_free(ids);
}

/* The runner's `drm card CARD CRTC`, traced without the card's index. */
static void drm_card(int the_card, int crtc)
{
    char req[64], expect[64];
    snprintf(req, sizeof(req), "drm card %d %d", the_card, crtc);
    snprintf(expect, sizeof(expect), "drm: card %d crtc %d", the_card, crtc);
    request(req);
    for (int i = 0; i < nreplies; i++) {
        if (strcmp(replies[i], expect) == 0) {
            line("drm: the device's card, crtc %d", crtc);
        } else {
            line("%s", replies[i]);
        }
    }
}

static void scanouts(int the_card, bool crc)
{
    for (int c = 0; c < 2; c++) {
        drm_card(the_card, c);
        ask("drm state");
        if (crc) {
            ask("drm crc");
            ask(PIXELS);
        }
    }
}

static void windows(int the_card, const int *on, int count)
{
    int nd = 0;
    SDL_DisplayID *ids = SDL_GetDisplays(&nd);
    SDL_GLContext ctx[2] = { 0 };
    static const float colors[2][3] = { { 1, 0, 0 }, { 0, 0, 1 } };
    nwins = 0;
    for (int k = 0, tried = 0; k < count; k++) {
        int i;
        if (on[k] >= nd) {
            continue;
        }
        i = tried++;
        SDL_PropertiesID p = SDL_CreateProperties();
        char title[64];
        SDL_Window *w;
        snprintf(title, sizeof(title), "kmsdrm_displays %d", i);
        SDL_SetStringProperty(p, SDL_PROP_WINDOW_CREATE_TITLE_STRING, title);
        SDL_SetNumberProperty(p, SDL_PROP_WINDOW_CREATE_X_NUMBER, SDL_WINDOWPOS_UNDEFINED_DISPLAY(ids[on[k]]));
        SDL_SetNumberProperty(p, SDL_PROP_WINDOW_CREATE_Y_NUMBER, SDL_WINDOWPOS_UNDEFINED_DISPLAY(ids[on[k]]));
        SDL_SetNumberProperty(p, SDL_PROP_WINDOW_CREATE_WIDTH_NUMBER, 640);
        SDL_SetNumberProperty(p, SDL_PROP_WINDOW_CREATE_HEIGHT_NUMBER, 480);
        w = SDL_CreateWindowWithProperties(p);
        SDL_DestroyProperties(p);
        line("window %d: %s", i, result(w != NULL));
        if (!w) {
            continue;
        }
        {
            int ww = 0, hh = 0;
            char disp[32] = "None";
            SDL_DisplayID d = SDL_GetDisplayForWindow(w);
            for (int j = 0; j < nd; j++) {
                if (ids[j] == d) {
                    snprintf(disp, sizeof(disp), "Some(%d)", j);
                }
            }
            SDL_GetWindowSize(w, &ww, &hh);
            line("window %d: display %s, flags 0x%llx size %dx%d", i, disp,
                 (unsigned long long)SDL_GetWindowFlags(w), ww, hh);
        }
        wins[nwins++] = w;
    }
    window_events();
    for (int i = 0; i < nwins; i++) {
        ctx[i] = SDL_GL_CreateContext(wins[i]);
        line("context %d: %s", i, result(ctx[i] != NULL));
    }
    for (int frame = 0; frame < 2; frame++) {
        for (int i = 0; i < nwins; i++) {
            const float *c;
            if (!ctx[i]) {
                continue;
            }
            line("window %d: make current: %s", i, result(SDL_GL_MakeCurrent(wins[i], ctx[i])));
            c = colors[(i + frame) % 2];
            gl_clear(c[0], c[1], c[2]);
            line("window %d: swap %d: %s", i, frame, result(SDL_GL_SwapWindow(wins[i])));
        }
        window_events();
        scanouts(the_card, true);
    }
    for (int i = 0; i < nwins; i++) {
        if (ctx[i]) {
            SDL_GL_DestroyContext(ctx[i]);
        }
    }
    for (int i = 0; i < nwins; i++) {
        SDL_DestroyWindow(wins[i]);
    }
    line("windows destroyed");
    nwins = 0;
    window_events();
    SDL_free(ids);
}

/* A comma list of names, each the device's connector's masked */
static const char *masked(int the_card, char conns[2][64], const char *names)
{
    static char out[512];
    char copy[512];
    char *save = NULL;
    out[0] = 0;
    snprintf(copy, sizeof(copy), "%s", names);
    for (char *n = strtok_r(copy, ",", &save); n; n = strtok_r(NULL, ",", &save)) {
        const char *m = (strcmp(n, conns[0]) == 0 || strcmp(n, conns[1]) == 0) ? connector_name(the_card, n) : n;
        if (out[0]) {
            strncat(out, ",", sizeof(out) - strlen(out) - 1);
        }
        strncat(out, m, sizeof(out) - strlen(out) - 1);
    }
    return out;
}

static void card_hint(int the_card)
{
    /* (again before each init: SDL_Quit() resets every hint) */
    char index[16];
    snprintf(index, sizeof(index), "%d", the_card);
    SDL_SetHintWithPriority(SDL_HINT_KMSDRM_DEVICE_INDEX, index, SDL_HINT_OVERRIDE);
}

int main(void)
{
    int the_card = -1;
    char name[64] = "", conns[2][64] = { "", "" };
    char both[160], priorities[3][256];
    static const int first[] = { 0 }, second[] = { 1 }, each[] = { 0, 1 };

    begin();
    line("-- a vkms device of two pipelines");
    request("vkms add multi 2");
    if (nreplies < 1 || sscanf(replies[0], "vkms: added %63s card%d %63s %63s", name, &the_card, conns[0], conns[1]) != 4) {
        for (int i = 0; i < nreplies; i++) {
            line("%s", replies[i]);
        }
        return finish("kmsdrm_displays");
    }
    snprintf(both, sizeof(both), "%s,%s", conns[0], conns[1]);
    line("vkms: added %s, connectors %s", name, masked(the_card, conns, both));
    scanouts(the_card, false);

    line("-- displays");
    card_hint(the_card);
    if (init_video()) {
        displays(the_card);
        quit_video();
    }
    snprintf(priorities[0], sizeof(priorities[0]), "%s", conns[1]);
    snprintf(priorities[1], sizeof(priorities[1]), "nosuch,%s,%s", conns[1], conns[0]);
    snprintf(priorities[2], sizeof(priorities[2]), "%s,%s", conns[0], conns[1]);
    for (int i = 0; i < 3; i++) {
        line("-- SDL_VIDEO_DISPLAY_PRIORITY=%s", masked(the_card, conns, priorities[i]));
        card_hint(the_card);
        SDL_SetHint(SDL_HINT_VIDEO_DISPLAY_PRIORITY, priorities[i]);
        if (init_video()) {
            displays(the_card);
            quit_video();
        }
        SDL_ResetHint(SDL_HINT_VIDEO_DISPLAY_PRIORITY);
    }

    line("-- a window on each display (legacy, SDL_KMSDRM_ATOMIC=0)");
    card_hint(the_card);
    SDL_SetHint(SDL_HINT_KMSDRM_ATOMIC, "0");
    if (init_video()) {
        windows(the_card, each, 2);
        quit_video();
    }
    SDL_ResetHint(SDL_HINT_KMSDRM_ATOMIC);
    scanouts(the_card, false);

    line("-- a window on the first display (atomic)");
    card_hint(the_card);
    if (init_video()) {
        windows(the_card, first, 1);
        quit_video();
    }
    scanouts(the_card, false);

    line("-- a window on the second display alone (atomic)");
    card_hint(the_card);
    if (init_video()) {
        windows(the_card, second, 1);
        quit_video();
    }
    scanouts(the_card, false);

    SDL_ResetHint(SDL_HINT_KMSDRM_DEVICE_INDEX);
    ask("drm card");
    ask("vkms remove multi");
    return finish("kmsdrm_displays");
}
