/* diag only: the C twin of sdl3/tests/kmsdrm_gles.rs (its trace; the
 * Rust-only scenes aren't here). */
#include "kms_common.h"

#define PIXELS "drm pixels 0 0 100 100 300 200 512 384 700 500 1023 767"

static void scanout(void)
{
    SDL_Event e;
    SDL_PumpEvents();
    while (SDL_PollEvent(&e)) {
    }
    ask("drm crc");
    ask(PIXELS);
}

typedef struct { const char *name, *value; } Setting;

static void swap_path(const char *label, const Setting *settings, int n)
{
    line("-- %s", label);
    for (int i = 0; i < n; i++) {
        SDL_SetHint(settings[i].name, settings[i].value);
    }
    if (init_video()) {
        SDL_Window *window = SDL_CreateWindow("kmsdrm_gles", 1024, 768, 0);
        line("window: %s", result(window != NULL));
        if (window) {
            SDL_GLContext context = SDL_GL_CreateContext(window);
            line("context: %s", result(context != NULL));
            if (context) {
                static const float colors[3][3] = { { 1, 0, 0 }, { 0, 1, 0 }, { 0, 0, 1 } };
                for (int interval = 0; interval <= 1; interval++) {
                    line("swap interval %d: %s", interval, result(SDL_GL_SetSwapInterval(interval)));
                    for (int i = 0; i < 3; i++) {
                        gl_clear(colors[i][0], colors[i][1], colors[i][2]);
                        line("swap %d: %s", i, result(SDL_GL_SwapWindow(window)));
                        if (i == 0 && interval == 0) {
                            ask("drm state");
                        }
                        scanout();
                    }
                }
                gl_clear(1, 1, 1);
                SDL_GL_SwapWindow(window);
                gl_clear(0.5f, 0.5f, 0.5f);
                line("swap twice: %s", result(SDL_GL_SwapWindow(window)));
                scanout();
                SDL_GL_DestroyContext(context);
            }
            SDL_DestroyWindow(window);
        }
        quit_video();
    }
    for (int i = 0; i < n; i++) {
        SDL_ResetHint(settings[i].name);
    }
    ask("drm state");
}

static SDL_Surface *pattern(void)
{
    SDL_Surface *s = SDL_CreateSurface(16, 16, SDL_PIXELFORMAT_ARGB8888);
    const SDL_PixelFormatDetails *d = SDL_GetPixelFormatDetails(s->format);
    for (int y = 0; y < 16; y++) {
        for (int x = 0; x < 16; x++) {
            SDL_Rect r = { x, y, 1, 1 };
            SDL_FillSurfaceRect(s, &r, SDL_MapRGB(d, NULL, (Uint8)(x * 16), (Uint8)(y * 16), (x + y) % 2 == 0 ? 255 : 0));
        }
    }
    return s;
}

static void renderer(const char *name)
{
    SDL_Window *window;
    line("-- renderer %s", name);
    if (!init_video()) {
        return;
    }
    window = SDL_CreateWindow("kmsdrm_gles", 1024, 768, 0);
    line("window: %s", result(window != NULL));
    if (window) {
        SDL_Renderer *r = SDL_CreateRenderer(window, name);
        line("renderer: %s", result(r != NULL));
        if (r) {
            int w = 0, h = 0;
            SDL_Surface *surface = pattern();
            SDL_Texture *texture;
            line("name: %s", SDL_GetRendererName(r));
            SDL_GetRenderOutputSize(r, &w, &h);
            line("output size: (%d, %d)", w, h);
            texture = SDL_CreateTextureFromSurface(r, surface);
            line("texture: %s", result(texture != NULL));
            for (int frame = 0; frame < 2; frame++) {
                SDL_FRect a = { 50, 50, 100, 100 }, b = { 250, 150, 100, 100 },
                         c = { 450.0f + frame * 50.0f, 334, 124, 100 }, d = { 100, 100, 200, 100 },
                         t = { 692, 492, 16, 16 };
                SDL_Surface *read;
                SDL_SetRenderDrawColor(r, 32, (Uint8)(64 + frame * 64), 96, 255);
                SDL_RenderClear(r);
                SDL_SetRenderDrawColor(r, 255, 0, 0, 255);
                SDL_RenderFillRect(r, &a);
                SDL_SetRenderDrawColor(r, 0, 255, 0, 255);
                SDL_RenderFillRect(r, &b);
                SDL_SetRenderDrawColor(r, 0, 0, 255, 255);
                SDL_RenderFillRect(r, &c);
                SDL_SetRenderDrawBlendMode(r, SDL_BLENDMODE_BLEND);
                SDL_SetRenderDrawColor(r, 255, 255, 255, 128);
                SDL_RenderFillRect(r, &d);
                SDL_SetRenderDrawBlendMode(r, SDL_BLENDMODE_NONE);
                if (texture) {
                    SDL_RenderTexture(r, texture, NULL, &t);
                }
                read = SDL_RenderReadPixels(r, NULL);
                if (read) {
                    SDL_Surface *rgba = SDL_ConvertSurface(read, SDL_PIXELFORMAT_RGBA32);
                    Uint32 h = 2166136261u;
                    for (int y = 0; y < rgba->h; y++) {
                        const Uint8 *row = (const Uint8 *)rgba->pixels + y * rgba->pitch;
                        for (int i = 0; i < rgba->w * 4; i++) {
                            h ^= row[i];
                            h *= 16777619u;
                        }
                    }
                    line("read back %d: %dx%d %08x", frame, rgba->w, rgba->h, h);
                    SDL_DestroySurface(rgba);
                    SDL_DestroySurface(read);
                } else {
                    line("read back %d: error: %s", frame, SDL_GetError());
                }
                line("present %d: %s", frame, result(SDL_RenderPresent(r)));
                scanout();
            }
            ask("drm state");
            SDL_DestroySurface(surface);
            SDL_DestroyRenderer(r);
        }
        SDL_DestroyWindow(window);
    }
    quit_video();
}

static void window_surface(void)
{
    SDL_Window *window;
    line("-- window surface");
    if (!init_video()) {
        return;
    }
    window = SDL_CreateWindow("kmsdrm_gles", 1024, 768, 0);
    line("window: %s", result(window != NULL));
    if (window) {
        SDL_Surface *s = SDL_GetWindowSurface(window);
        line("surface: %s", result(s != NULL));
        if (s) {
            const SDL_PixelFormatDetails *d = SDL_GetPixelFormatDetails(s->format);
            line("surface: %dx%d %s", s->w, s->h, SDL_GetPixelFormatName(s->format));
            for (int frame = 0; frame < 2; frame++) {
                SDL_Rect a = { 50, 50, 100, 100 }, b = { 250 + frame * 25, 150, 100, 100 };
                SDL_FillSurfaceRect(s, NULL, SDL_MapRGB(d, NULL, 16, (Uint8)(32 + frame * 100), 48));
                SDL_FillSurfaceRect(s, &a, SDL_MapRGB(d, NULL, 255, 0, 0));
                SDL_FillSurfaceRect(s, &b, SDL_MapRGB(d, NULL, 0, 255, 0));
                line("update %d: %s", frame, result(SDL_UpdateWindowSurface(window)));
                scanout();
            }
            ask("drm state");
        }
        SDL_DestroyWindow(window);
    }
    quit_video();
}

int main(void)
{
    static const Setting db[] = { { SDL_HINT_VIDEO_DOUBLE_BUFFER, "1" } };
    static const Setting legacy[] = { { SDL_HINT_KMSDRM_ATOMIC, "0" } };
    static const Setting legacy_db[] = { { SDL_HINT_KMSDRM_ATOMIC, "0" }, { SDL_HINT_VIDEO_DOUBLE_BUFFER, "1" } };

    begin();
    ask("drm state");

    swap_path("fenced (atomic)", NULL, 0);
    swap_path("double buffered (atomic, SDL_VIDEO_DOUBLE_BUFFER=1)", db, 1);
    swap_path("legacy (SDL_KMSDRM_ATOMIC=0)", legacy, 1);
    swap_path("legacy, double buffered", legacy_db, 2);
    renderer("opengles2");
    renderer("opengl");
    window_surface();

    ask("drm state");
    return finish("kmsdrm_gles");
}
