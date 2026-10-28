/* Upstream reference for sdl3/tests/uikit_callbacks.rs (diag branch only). */
#define SDL_MAIN_HANDLED
#include "common.h"
#include <SDL3/SDL_main.h>

static Uint64 iterations[64];
static int niterations;
static bool quit_called;
static bool warmup = true;
static int active_iterations;
static Uint64 warmup_start;

static SDL_AppResult SDLCALL app_init(void **appstate, int argc, char *argv[])
{
    (void)appstate;
    (void)argv;
    t("init: argc=%d", argc);
    bool ok = SDL_Init(SDL_INIT_VIDEO);
    t("init video: %s", ok ? "ok" : SDL_GetError());
    warmup_start = SDL_GetTicks();
    return SDL_APP_CONTINUE;
}

static SDL_AppResult SDLCALL app_iterate(void *appstate)
{
    (void)appstate;
    if (warmup) {
        if ([UIApplication sharedApplication].applicationState == 0) {
            active_iterations++;
        }
        if (active_iterations >= 10 || SDL_GetTicks() - warmup_start > 30000) {
            warmup = false;
            printf("uikit_callbacks: %.3f s: application state %d\n", elapsed(), (int)[UIApplication sharedApplication].applicationState);
            t("warmup: application state %d", (int)[UIApplication sharedApplication].applicationState);
        }
        return SDL_APP_CONTINUE;
    }
    if (niterations < 64) {
        iterations[niterations] = SDL_GetTicksNS();
    }
    int n = ++niterations;
    switch (n) {
    case 2:
        t("iterate 2: posting a memory warning");
        [[NSNotificationCenter defaultCenter] postNotificationName:UIApplicationDidReceiveMemoryWarningNotification object:[UIApplication sharedApplication]];
        return SDL_APP_CONTINUE;
    case 3: {
        t("iterate 3: pushing a user event");
        SDL_Event e;
        SDL_zero(e);
        e.type = SDL_EVENT_USER;
        SDL_PushEvent(&e);
        return SDL_APP_CONTINUE;
    }
    case 5:
        t("iterate 5: success");
        return SDL_APP_SUCCESS;
    default:
        t("iterate %d", n);
        return SDL_APP_CONTINUE;
    }
}

static SDL_AppResult SDLCALL app_event(void *appstate, SDL_Event *event)
{
    (void)appstate;
    const char *d = describe(event, "");
    bool lifecycle = event->type >= SDL_EVENT_TERMINATING && event->type <= SDL_EVENT_DID_ENTER_FOREGROUND;
    if (lifecycle) {
        printf("uikit_callbacks: %.3f s: %s\n", elapsed(), d);
    }
    if (d && !(lifecycle && warmup)) {
        t("event %s", d);
    }
    return SDL_APP_CONTINUE;
}

static void SDLCALL app_quit(void *appstate, SDL_AppResult result)
{
    (void)appstate;
    t("quit: %s", result == SDL_APP_SUCCESS ? "SUCCESS" : result == SDL_APP_FAILURE ? "FAILURE" : "CONTINUE");
    quit_called = true;
}

static void at_exit(void)
{
    bool paced = niterations == 5;
    for (int i = 1; i < niterations && i < 64; i++) {
        Uint64 d = iterations[i] - iterations[i - 1];
        printf("interval %d: %.2f ms\n", i, d / 1e6);
        if (d < 4 * SDL_NS_PER_MS || d > 100 * SDL_NS_PER_MS) {
            paced = false;
        }
    }
    t("iterations paced by the display: %s", paced ? "yes" : "no");
    t("%s", quit_called ? "exit: after quit" : "exit: without quit");
    exit_line(0);
}

static int app_main(int argc, char *argv[])
{
    t("main: argc=%d", argc);
    int rc = SDL_EnterAppMainCallbacks(argc, argv, app_init, app_iterate, app_event, app_quit);
    t("main: enter_app_main_callbacks returned %d", rc);
    t("main returns");
    return rc;
}

int main(int argc, char *argv[])
{
    atexit(at_exit);
    elapsed();
    int status = SDL_RunApp(argc, argv, app_main, NULL);
    printf("uikit_callbacks_ref: SDL_RunApp returned %d\n", status);
    return 1;
}
