/* Shared by the upstream C reference programs (diag branch only). */
#include <SDL3/SDL.h>
#import <UIKit/UIKit.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <objc/runtime.h>

__attribute__((used, section("__DATA,__sdl3_runner"))) static const char runner_words[] = "app";

static void t(const char *fmt, ...) SDL_PRINTF_VARARG_FUNC(1);
static void t(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    printf("trace: ");
    vprintf(fmt, ap);
    printf("\n");
    fflush(stdout);
    va_end(ap);
}

static double elapsed(void)
{
    static Uint64 start;
    if (!start) {
        start = SDL_GetTicksNS();
    }
    return (SDL_GetTicksNS() - start) / 1e9;
}

static void exit_line(int status)
{
    printf("sdl3-ios-runner: exit %d\n", status);
    fflush(stdout);
    fflush(stderr);
}

static const char *event_name(Uint32 type)
{
    switch (type) {
#define N(x) case SDL_EVENT_##x: return #x;
    N(QUIT) N(TERMINATING) N(LOW_MEMORY) N(WILL_ENTER_BACKGROUND) N(DID_ENTER_BACKGROUND)
    N(WILL_ENTER_FOREGROUND) N(DID_ENTER_FOREGROUND) N(LOCALE_CHANGED) N(SYSTEM_THEME_CHANGED)
    N(DISPLAY_ORIENTATION) N(DISPLAY_ADDED) N(DISPLAY_REMOVED) N(DISPLAY_MOVED)
    N(DISPLAY_DESKTOP_MODE_CHANGED) N(DISPLAY_CURRENT_MODE_CHANGED) N(DISPLAY_CONTENT_SCALE_CHANGED)
    N(DISPLAY_USABLE_BOUNDS_CHANGED) N(DROP_FILE) N(DROP_TEXT) N(DROP_BEGIN) N(DROP_COMPLETE)
    N(DROP_POSITION) N(USER) N(KEYBOARD_ADDED) N(KEYBOARD_REMOVED) N(MOUSE_ADDED) N(MOUSE_REMOVED)
    N(KEYMAP_CHANGED) N(CLIPBOARD_UPDATE)
#undef N
    default: {
        static char buf[32];
        SDL_snprintf(buf, sizeof(buf), "EventType(0x%x)", (unsigned)type);
        return buf;
    }
    }
}

/* "A, B, C" or "(none)" */
typedef struct { char s[4096]; int n; } List;
static void list_add(List *l, const char *item)
{
    size_t len = strlen(l->s);
    SDL_snprintf(l->s + len, sizeof(l->s) - len, "%s%s", l->n ? ", " : "", item);
    l->n++;
}
static const char *list_str(List *l) { return l->n ? l->s : "(none)"; }

/* An event as the trace names it; NULL for the mice's device events. */
static const char *describe(const SDL_Event *e, const char *scheme)
{
    static char buf[1024];
    switch (e->type) {
    case SDL_EVENT_MOUSE_ADDED: case SDL_EVENT_MOUSE_REMOVED:
        return NULL;
    default: break;
    }
    if (e->type >= SDL_EVENT_DISPLAY_FIRST && e->type <= SDL_EVENT_DISPLAY_LAST) {
        SDL_snprintf(buf, sizeof(buf), "%s(%d)", event_name(e->type), (int)e->display.data1);
        return buf;
    }
    if (e->type == SDL_EVENT_DROP_FILE) {
        char data[900];
        const char *d = e->drop.data ? e->drop.data : "";
        if (scheme && *scheme && strncmp(d, scheme, strlen(scheme)) == 0) {
            SDL_snprintf(data, sizeof(data), "SCHEME%s", d + strlen(scheme));
        } else {
            SDL_snprintf(data, sizeof(data), "%s", d);
        }
        SDL_snprintf(buf, sizeof(buf), "%s(%s)", event_name(e->type), data);
        return buf;
    }
    return event_name(e->type);
}
