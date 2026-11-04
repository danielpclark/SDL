/* diag only: the C twin of sdl3/tests/evdev_input.rs, on upstream SDL at
 * ea7a2da, run by tools/linux_console_run.py. Prints the same trace lines. */
#include <SDL3/SDL.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define PREFIX "sdl3-console-runner: "
#define KEYBOARD "SDL3 evdev keyboard"
#define MOUSE "SDL3 evdev mouse"
#define TABLET "SDL3 evdev tablet"
#define TOUCHSCREEN "SDL3 evdev touchscreen"
#define KEYBOARD2 "SDL3 evdev keyboard 2"

static char replies[16][512];
static int nreplies;

static void request(const char *line)
{
    char answer[1024];
    printf(PREFIX "request %s\n", line);
    fflush(stdout);
    nreplies = 0;
    while (fgets(answer, sizeof(answer), stdin)) {
        answer[strcspn(answer, "\n")] = 0;
        if (strcmp(answer, PREFIX "done") == 0) {
            return;
        }
        if (strncmp(answer, PREFIX "reply ", strlen(PREFIX "reply ")) == 0 && nreplies < 16) {
            snprintf(replies[nreplies++], sizeof(replies[0]), "%s", answer + strlen(PREFIX "reply "));
        }
    }
    fprintf(stderr, "the runner ended without answering\n");
    exit(2);
}

static int nlines;
static void line(const char *text)
{
    printf("trace: %s\n", text);
    nlines++;
}

typedef struct { Uint32 id; char name[128]; } Named;
static Named keyboards[64], mice[64];
static int nkeyboards, nmice;
static SDL_WindowID window_id;

static const char *lookup(Named *list, int n, Uint32 id)
{
    for (int i = n - 1; i >= 0; i--) {
        if (list[i].id == id) return list[i].name;
    }
    return NULL;
}

static void remember(Named *list, int *n, Uint32 id, const char *name)
{
    if (*n < 64) {
        list[*n].id = id;
        snprintf(list[*n].name, sizeof(list[0].name), "%s", name ? name : "");
        (*n)++;
    }
}

static int ours(const char *name)
{
    return name && strncmp(name, "SDL3 evdev", 10) == 0;
}

static const char *win(SDL_WindowID id)
{
    return id == window_id ? "window" : id == 0 ? "none" : "another";
}

static const char *mouse_who(SDL_MouseID which)
{
    const char *name;
    if (which == 0) return "global";
    if (which == SDL_TOUCH_MOUSEID) return "touch";
    if (which == SDL_PEN_MOUSEID) return "pen";
    name = lookup(mice, nmice, which);
    return name ? name : "other";
}

/* The event's line in buf, if it's a traced one. */
static int describe(const SDL_Event *e, char *buf, size_t size)
{
    const char *name;
    switch (e->type) {
    case SDL_EVENT_KEYBOARD_ADDED:
    case SDL_EVENT_KEYBOARD_REMOVED:
        if (e->type == SDL_EVENT_KEYBOARD_ADDED) {
            remember(keyboards, &nkeyboards, e->kdevice.which, SDL_GetKeyboardNameForID(e->kdevice.which));
        }
        name = lookup(keyboards, nkeyboards, e->kdevice.which);
        if (!ours(name)) return 0;
        snprintf(buf, size, "keyboard %s: %s", e->type == SDL_EVENT_KEYBOARD_ADDED ? "added" : "removed", name);
        return 1;
    case SDL_EVENT_MOUSE_ADDED:
    case SDL_EVENT_MOUSE_REMOVED:
        if (e->type == SDL_EVENT_MOUSE_ADDED) {
            remember(mice, &nmice, e->mdevice.which, SDL_GetMouseNameForID(e->mdevice.which));
        }
        name = lookup(mice, nmice, e->mdevice.which);
        if (!ours(name)) return 0;
        snprintf(buf, size, "mouse %s: %s", e->type == SDL_EVENT_MOUSE_ADDED ? "added" : "removed", name);
        return 1;
    case SDL_EVENT_KEY_DOWN:
    case SDL_EVENT_KEY_UP:
        name = lookup(keyboards, nkeyboards, e->key.which);
        snprintf(buf, size, "key %s %s key %s raw %u mod 0x%04x repeat %d %s from %s", e->key.down ? "down" : "up",
                 SDL_GetScancodeName(e->key.scancode), SDL_GetKeyName(e->key.key), (unsigned)e->key.raw,
                 (unsigned)e->key.mod, e->key.repeat ? 1 : 0, win(e->key.windowID), name ? name : "other");
        return 1;
    case SDL_EVENT_TEXT_INPUT:
        snprintf(buf, size, "text \"%s\" %s", e->text.text, win(e->text.windowID));
        return 1;
    case SDL_EVENT_MOUSE_MOTION:
        snprintf(buf, size, "motion %s at (%.4f, %.4f) rel (%.4f, %.4f) state 0x%x %s", mouse_who(e->motion.which),
                 e->motion.x, e->motion.y, e->motion.xrel, e->motion.yrel, (unsigned)e->motion.state,
                 win(e->motion.windowID));
        return 1;
    case SDL_EVENT_MOUSE_BUTTON_DOWN:
    case SDL_EVENT_MOUSE_BUTTON_UP:
        snprintf(buf, size, "button %s %u %s at (%.4f, %.4f) %s", e->button.down ? "down" : "up",
                 (unsigned)e->button.button, mouse_who(e->button.which), e->button.x, e->button.y,
                 win(e->button.windowID));
        return 1;
    case SDL_EVENT_MOUSE_WHEEL:
        snprintf(buf, size, "wheel %s (%.4f, %.4f) integer (%d, %d) direction %d at (%.4f, %.4f) %s",
                 mouse_who(e->wheel.which), e->wheel.x, e->wheel.y, (int)e->wheel.integer_x, (int)e->wheel.integer_y,
                 (int)e->wheel.direction, e->wheel.mouse_x, e->wheel.mouse_y, win(e->wheel.windowID));
        return 1;
    case SDL_EVENT_FINGER_DOWN:
    case SDL_EVENT_FINGER_UP:
    case SDL_EVENT_FINGER_MOTION:
    case SDL_EVENT_FINGER_CANCELED:
        name = SDL_GetTouchDeviceName(e->tfinger.touchID);
        snprintf(buf, size, "finger %s %" SDL_PRIu64 " on %s at (%.4f, %.4f) delta (%.4f, %.4f) pressure %.4f %s",
                 e->type == SDL_EVENT_FINGER_DOWN ? "down" : e->type == SDL_EVENT_FINGER_UP ? "up"
                 : e->type == SDL_EVENT_FINGER_MOTION ? "motion" : "canceled",
                 e->tfinger.fingerID, name ? name : "", e->tfinger.x, e->tfinger.y, e->tfinger.dx, e->tfinger.dy,
                 e->tfinger.pressure, win(e->tfinger.windowID));
        return 1;
    default:
        return 0;
    }
}

static void pump(void)
{
    SDL_Event e;
    char buf[512];
    SDL_PumpEvents();
    while (SDL_PollEvent(&e)) {
        if (describe(&e, buf, sizeof(buf))) line(buf);
    }
}

static void wait_for(const char *want)
{
    SDL_Event e;
    char buf[512];
    Uint64 end = SDL_GetTicks() + 10000;
    for (;;) {
        SDL_PumpEvents();
        while (SDL_PollEvent(&e)) {
            if (describe(&e, buf, sizeof(buf))) {
                int done = strcmp(buf, want) == 0;
                line(buf);
                if (done) return;
            }
        }
        if (SDL_GetTicks() >= end) {
            snprintf(buf, sizeof(buf), "timeout: no \"%s\" in 10 s", want);
            line(buf);
            return;
        }
        SDL_Delay(10);
    }
}

static void events(const char *device, const char *evs)
{
    static char req[65536];
    snprintf(req, sizeof(req), "uinput events '%s' %s", device, evs);
    request(req);
    for (int i = 0; i < nreplies; i++) {
        if (strncmp(replies[i], "uinput: sent ", 13) != 0) line(replies[i]);
    }
    pump();
}

static const char *tap(const char *key)
{
    static char bufs[4][256];
    static int n;
    char *b = bufs[n++ % 4];
    snprintf(b, 256, "EV_KEY %s 1 EV_SYN SYN_REPORT 0 EV_KEY %s 0 EV_SYN SYN_REPORT 0", key, key);
    return b;
}

static int cmp(const void *a, const void *b)
{
    return strcmp((const char *)a, (const char *)b);
}

int main(int argc, char **argv)
{
    static const char *setup[][2] = { { KEYBOARD, "keyboard" }, { MOUSE, "mouse" }, { TABLET, "tablet" }, { TOUCHSCREEN, "touchscreen" } };
    char buf[1024], found[32][512];
    int nfound = 0;
    SDL_Window *window;
    SDL_Event e;
    (void)argc; (void)argv;

    char saved[8][512];
    int nsaved = 0;
    for (int i = 0; i < 4; i++) {
        snprintf(buf, sizeof(buf), "uinput add '%s' %s", setup[i][0], setup[i][1]);
        request(buf);
        for (int r = 0; r < nreplies; r++) snprintf(saved[nsaved++], 512, "%s", replies[r]);
    }
    for (int i = 0; i < nsaved; i++) line(saved[i]);

    SDL_SetHint(SDL_HINT_VIDEO_DRIVER, "evdev");
    if (!SDL_Init(SDL_INIT_VIDEO)) {
        snprintf(buf, sizeof(buf), "init: %s", SDL_GetError());
        line(buf);
        return 1;
    }
    snprintf(buf, sizeof(buf), "init: ok (%s)", SDL_GetCurrentVideoDriver());
    line(buf);
    window = SDL_CreateWindow("evdev_input", 1024, 768, 0);
    window_id = SDL_GetWindowID(window);
    SDL_StartTextInput(window);

    while (SDL_PollEvent(&e)) {
        if (describe(&e, found[nfound], sizeof(found[0])) && nfound < 31) nfound++;
    }
    qsort(found, nfound, sizeof(found[0]), cmp);
    for (int i = 0; i < nfound; i++) line(found[i]);
    {
        int n = 0;
        SDL_TouchID *ids = SDL_GetTouchDevices(&n);
        char touches[8][256];
        int nt = 0;
        for (int i = 0; i < n && nt < 8; i++) {
            SDL_TouchDeviceType type = SDL_GetTouchDeviceType(ids[i]);
            const char *name = SDL_GetTouchDeviceName(ids[i]);
            snprintf(touches[nt], 256, "touch device: %s (%s)", name ? name : "",
                     type == SDL_TOUCH_DEVICE_DIRECT ? "direct" : type == SDL_TOUCH_DEVICE_INDIRECT_ABSOLUTE ? "indirect absolute"
                     : type == SDL_TOUCH_DEVICE_INDIRECT_RELATIVE ? "indirect relative" : "invalid");
            if (strstr(touches[nt], "SDL3 evdev")) nt++;
        }
        SDL_free(ids);
        qsort(touches, nt, sizeof(touches[0]), cmp);
        for (int i = 0; i < nt; i++) line(touches[i]);
    }

    line("-- keyboard");
    events(KEYBOARD, tap("KEY_A"));
    snprintf(buf, sizeof(buf), "EV_KEY KEY_LEFTSHIFT 1 EV_SYN SYN_REPORT 0 %s %s EV_KEY KEY_LEFTSHIFT 0 EV_SYN SYN_REPORT 0", tap("KEY_A"), tap("KEY_1"));
    events(KEYBOARD, buf);
    events(KEYBOARD, "EV_KEY KEY_B 1 EV_SYN SYN_REPORT 0 EV_KEY KEY_B 2 EV_SYN SYN_REPORT 0 "
                     "EV_KEY KEY_B 2 EV_SYN SYN_REPORT 0 EV_KEY KEY_B 0 EV_SYN SYN_REPORT 0");
    snprintf(buf, sizeof(buf), "%s %s", tap("KEY_KP7"), tap("KEY_NUMLOCK"));
    events(KEYBOARD, buf);
    snprintf(buf, sizeof(buf), "%s %s %s", tap("KEY_KP7"), tap("KEY_KPENTER"), tap("KEY_NUMLOCK"));
    events(KEYBOARD, buf);
    snprintf(buf, sizeof(buf), "%s %s %s", tap("KEY_CAPSLOCK"), tap("KEY_A"), tap("KEY_CAPSLOCK"));
    events(KEYBOARD, buf);
    snprintf(buf, sizeof(buf), "EV_KEY KEY_LEFTCTRL 1 EV_SYN SYN_REPORT 0 %s EV_KEY KEY_LEFTCTRL 0 EV_SYN SYN_REPORT 0", tap("KEY_C"));
    events(KEYBOARD, buf);
    snprintf(buf, sizeof(buf), "%s %s", tap("KEY_SPACE"), tap("0x2fe"));
    events(KEYBOARD, buf);

    line("-- mouse");
    events(MOUSE, "EV_REL REL_X 10 EV_REL REL_Y 5 EV_SYN SYN_REPORT 0");
    events(MOUSE, "EV_REL REL_X -4 EV_SYN SYN_REPORT 0 EV_REL REL_Y 2 EV_SYN SYN_REPORT 0");
    {
        static const char *buttons[] = { "BTN_LEFT", "BTN_RIGHT", "BTN_MIDDLE", "BTN_SIDE", "BTN_EXTRA", "BTN_FORWARD", "BTN_BACK", "BTN_TASK" };
        for (int i = 0; i < 8; i++) events(MOUSE, tap(buttons[i]));
    }
    events(MOUSE, "EV_REL REL_WHEEL 1 EV_REL REL_WHEEL_HI_RES 120 EV_SYN SYN_REPORT 0 "
                  "EV_REL REL_HWHEEL_HI_RES -60 EV_SYN SYN_REPORT 0 EV_REL REL_WHEEL_HI_RES 30 EV_SYN SYN_REPORT 0");
    snprintf(buf, sizeof(buf), "relative mode: %s", SDL_SetWindowRelativeMouseMode(window, true) ? "ok" : "failed");
    line(buf);
    events(MOUSE, "EV_REL REL_X 3 EV_REL REL_Y -2 EV_SYN SYN_REPORT 0");
    events(MOUSE, tap("BTN_LEFT"));
    snprintf(buf, sizeof(buf), "relative mode off: %s", SDL_SetWindowRelativeMouseMode(window, false) ? "ok" : "failed");
    line(buf);
    pump();

    line("-- tablet");
    events(TABLET, "EV_ABS ABS_X 16384 EV_ABS ABS_Y 8192 EV_SYN SYN_REPORT 0");
    events(TABLET, "EV_ABS ABS_X 32000 EV_SYN SYN_REPORT 0");
    events(TABLET, tap("BTN_LEFT"));

    line("-- touchscreen");
    events(TOUCHSCREEN, "EV_ABS ABS_MT_SLOT 0 EV_ABS ABS_MT_TRACKING_ID 100 EV_ABS ABS_MT_POSITION_X 100 "
                        "EV_ABS ABS_MT_POSITION_Y 200 EV_ABS ABS_MT_PRESSURE 128 EV_KEY BTN_TOUCH 1 "
                        "EV_ABS ABS_X 100 EV_ABS ABS_Y 200 EV_SYN SYN_REPORT 0");
    events(TOUCHSCREEN, "EV_ABS ABS_MT_SLOT 1 EV_ABS ABS_MT_TRACKING_ID 101 EV_ABS ABS_MT_POSITION_X 900 "
                        "EV_ABS ABS_MT_POSITION_Y 800 EV_ABS ABS_MT_PRESSURE 255 EV_SYN SYN_REPORT 0");
    events(TOUCHSCREEN, "EV_ABS ABS_MT_SLOT 0 EV_ABS ABS_MT_POSITION_X 150 EV_ABS ABS_X 150 EV_SYN SYN_REPORT 0");
    events(TOUCHSCREEN, "EV_ABS ABS_MT_TRACKING_ID -1 EV_ABS ABS_MT_SLOT 1 EV_ABS ABS_MT_TRACKING_ID -1 "
                        "EV_KEY BTN_TOUCH 0 EV_SYN SYN_REPORT 0");
    {
        static char overflow[60000];
        size_t len = (size_t)snprintf(overflow, sizeof(overflow),
                                      "EV_ABS ABS_MT_SLOT 0 EV_ABS ABS_MT_TRACKING_ID 200 EV_ABS ABS_MT_POSITION_X 500 "
                                      "EV_ABS ABS_MT_POSITION_Y 500 EV_ABS ABS_MT_PRESSURE 200 EV_SYN SYN_REPORT 0");
        for (int i = 1; i <= 300; i++) {
            len += (size_t)snprintf(overflow + len, sizeof(overflow) - len, " EV_ABS ABS_MT_POSITION_X %d EV_SYN SYN_REPORT 0", 500 + i % 200);
        }
        snprintf(overflow + len, sizeof(overflow) - len, " EV_ABS ABS_MT_POSITION_X 700 EV_ABS ABS_MT_POSITION_Y 600 EV_SYN SYN_REPORT 0");
        events(TOUCHSCREEN, overflow);
    }
    events(TOUCHSCREEN, "EV_ABS ABS_MT_TRACKING_ID -1 EV_SYN SYN_REPORT 0");

    line("-- hotplug");
    request("uinput add '" KEYBOARD2 "' keyboard");
    for (int i = 0; i < nreplies; i++) line(replies[i]);
    wait_for("keyboard added: " KEYBOARD2);
    events(KEYBOARD2, tap("KEY_Z"));
    request("uinput remove '" KEYBOARD2 "'");
    for (int i = 0; i < nreplies; i++) line(replies[i]);
    wait_for("keyboard removed: " KEYBOARD2);
    request("uinput remove '" TABLET "'");
    for (int i = 0; i < nreplies; i++) line(replies[i]);
    wait_for("mouse removed: " TABLET);

    SDL_DestroyWindow(window);
    SDL_Quit();
    line("quit");
    printf("evdev_input (C): %d lines\n", nlines);
    return 0;
}
