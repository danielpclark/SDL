/* diag only: what the C twins of sdl3/tests/kmsdrm_*.rs share (the
 * runner's requests, the trace), on upstream SDL at ea7a2da, run by
 * tools/linux_console_run.py --vt 3. */
#define _GNU_SOURCE
#include <SDL3/SDL.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dirent.h>

#define PREFIX "sdl3-console-runner: "
#define VT "3"
#define OTHER_VT "1"

static FILE *reply_file;
static char replies[64][512];
static int nreplies;
static int nlines;

static void request(const char *line)
{
    char answer[1024];
    if (!reply_file) {
        const char *fd = getenv("SDL3_CONSOLE_REPLY_FD");
        if (!fd) {
            fprintf(stderr, "no SDL3_CONSOLE_REPLY_FD\n");
            exit(2);
        }
        reply_file = fdopen(atoi(fd), "r");
        setvbuf(reply_file, NULL, _IONBF, 0);
    }
    printf(PREFIX "request %s\n", line);
    fflush(stdout);
    nreplies = 0;
    while (fgets(answer, sizeof(answer), reply_file)) {
        answer[strcspn(answer, "\n")] = 0;
        if (strcmp(answer, PREFIX "done") == 0) {
            return;
        }
        if (strncmp(answer, PREFIX "reply ", strlen(PREFIX "reply ")) == 0 && nreplies < 64) {
            snprintf(replies[nreplies++], sizeof(replies[0]), "%s", answer + strlen(PREFIX "reply "));
        }
    }
    fprintf(stderr, "the runner ended without answering\n");
    exit(2);
}

static void line(const char *fmt, ...)
{
    char buf[2048];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(buf, sizeof(buf), fmt, ap);
    va_end(ap);
    printf("trace: %s\n", buf);
    fflush(stdout);
    nlines++;
}

static void ask(const char *what)
{
    request(what);
    for (int i = 0; i < nreplies; i++) {
        line("%s", replies[i]);
    }
}

static const char *result(bool ok)
{
    static char buf[1024];
    if (ok) {
        return "ok";
    }
    snprintf(buf, sizeof(buf), "error: %s", SDL_GetError());
    return buf;
}

static const char *mode_line(const SDL_DisplayMode *m)
{
    static char buf[4][256];
    static int n;
    char *b = buf[n++ % 4];
    snprintf(b, 256, "%dx%d@%.2f (%d/%d) density %.2f %s", m->w, m->h, m->refresh_rate,
             m->refresh_rate_numerator, m->refresh_rate_denominator, m->pixel_density,
             SDL_GetPixelFormatName(m->format));
    return b;
}

/* The card the programs run on (SDL_KMSDRM_DEVICE_INDEX's, which the
 * runner runs them with), as card() in kmsdrm_support. */
static int card(void)
{
    const char *index = getenv("SDL_KMSDRM_DEVICE_INDEX");
    return index && *index ? atoi(index) : 0;
}

/* A display's name (its connector's, TYPE-N) as the traces give it,
 * TYPE-#K: the card's Kth connector of that type, by number (TYPE-#?
 * when the card has no connector of that name), as connector_name() in
 * kmsdrm_support: the kernel numbers a type's connectors across all the
 * machine's cards, so N depends on the cards before the vkms card. */
static const char *connector_name(int the_card, const char *name)
{
    static char buf[8][256];
    static int next;
    char *out = buf[next++ % 8];
    const char *dash = name ? strrchr(name, '-') : NULL;
    char prefix[256];
    size_t plen;
    char *end;
    long number;
    int rank = 1, found = 0;
    DIR *dir;
    struct dirent *e;

    if (!dash || !dash[1] || dash[1] < '0' || dash[1] > '9') {
        snprintf(out, 256, "%s", name ? name : "");
        return out;
    }
    number = strtol(dash + 1, &end, 10);
    if (*end) {
        snprintf(out, 256, "%s", name);
        return out;
    }
    snprintf(prefix, sizeof(prefix), "card%d-%.*s-", the_card, (int)(dash - name), name);
    plen = strlen(prefix);
    dir = opendir("/sys/class/drm");
    while (dir && (e = readdir(dir))) {
        if (strncmp(e->d_name, prefix, plen) == 0 && e->d_name[plen] >= '0' && e->d_name[plen] <= '9') {
            long n = strtol(e->d_name + plen, &end, 10);
            if (!*end) {
                if (n < number) {
                    rank++;
                } else if (n == number) {
                    found = 1;
                }
            }
        }
    }
    if (dir) {
        closedir(dir);
    }
    if (found) {
        snprintf(out, 256, "%.*s-#%d", (int)(dash - name), name, rank);
    } else {
        snprintf(out, 256, "%.*s-#?", (int)(dash - name), name);
    }
    return out;
}

typedef void (*ClearColorFn)(float, float, float, float);
typedef void (*ClearFn)(unsigned int);

static void gl_clear(float r, float g, float b)
{
    ClearColorFn clear_color = (ClearColorFn)SDL_GL_GetProcAddress("glClearColor");
    ClearFn clear = (ClearFn)SDL_GL_GetProcAddress("glClear");
    clear_color(r, g, b, 1.0f);
    clear(0x4000);
}

/* Pump until done() says so (or 10 s passed: false). */
static bool pump_until(bool (*done)(void *), void *data)
{
    Uint64 end = SDL_GetTicks() + 10000;
    for (;;) {
        SDL_PumpEvents();
        if (done(data)) {
            return true;
        }
        if (SDL_GetTicks() >= end) {
            return false;
        }
        SDL_Delay(10);
    }
}

static void begin(void)
{
    /* (the runner's `drm versions`: the reference's preamble) */
    ask("drm versions");
}

static int finish(const char *program)
{
    printf("%s.c: %d lines\n", program, nlines);
    return 0;
}

static bool init_video(void)
{
    if (!SDL_Init(SDL_INIT_VIDEO)) {
        line("init: %s", SDL_GetError());
        return false;
    }
    line("init: ok (%s)", SDL_GetCurrentVideoDriver() ? SDL_GetCurrentVideoDriver() : "(none)");
    return true;
}

static void quit_video(void)
{
    SDL_Quit();
    line("quit");
}
