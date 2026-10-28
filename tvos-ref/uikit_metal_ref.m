/* Upstream reference for sdl3/tests/uikit_metal.rs on iOS and tvOS (diag branch only). */
#define SDL_MAIN_HANDLED
#include "common.h"
#include <SDL3/SDL_main.h>
#import <Metal/Metal.h>
#import <QuartzCore/CAMetalLayer.h>

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
    N(WINDOW_HDR_STATE_CHANGED)
    N(RENDER_TARGETS_RESET) N(RENDER_DEVICE_RESET) N(RENDER_DEVICE_LOST)
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

static void pump_until(List *l, const char *until, Uint64 limit_ms, int after_ms)
{
    Uint64 end = SDL_GetTicks() + limit_ms;
    while (SDL_GetTicks() < end) {
        take_queued(l);
        if (strstr(l->s, until)) {
            settle(l, after_ms);
            break;
        }
        SDL_Delay(10);
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

static UIWindow *uiwindow_of(SDL_Window *w)
{
    return (__bridge UIWindow *)SDL_GetPointerProperty(SDL_GetWindowProperties(w), SDL_PROP_WINDOW_UIKIT_WINDOW_POINTER, NULL);
}

static UIViewController *vc_of(SDL_Window *w)
{
    return uiwindow_of(w).rootViewController;
}

static const char *rect_str(CGRect r)
{
    static char buf[4][96];
    static int n;
    char *b = buf[n++ % 4];
    SDL_snprintf(b, 96, "%g,%g %gx%g", r.origin.x, r.origin.y, r.size.width, r.size.height);
    return b;
}

/* The window's sizes and flags (without the OpenGL flag, which upstream
 * gives every window that asks for no graphics API on UIKit), its Metal
 * view tag and its view controller's view. */
static void window_state(const char *label, SDL_Window *w)
{
    int ww = 0, wh = 0, pw = 0, ph = 0;
    SDL_GetWindowSize(w, &ww, &wh);
    SDL_GetWindowSizeInPixels(w, &pw, &ph);
    t("%s: size=%dx%d pixels=%dx%d density=%g flags=0x%llx metal_view_tag=%lld", label, ww, wh, pw, ph,
      (double)SDL_GetWindowPixelDensity(w), (unsigned long long)(SDL_GetWindowFlags(w) & ~SDL_WINDOW_OPENGL),
      (long long)SDL_GetNumberProperty(SDL_GetWindowProperties(w), SDL_PROP_WINDOW_UIKIT_METAL_VIEW_TAG_NUMBER, -1));
    UIView *v = vc_of(w).view;
    t("%s: root view: %s tag=%ld frame=%s", label, v ? object_getClassName(v) : "nil", (long)v.tag, rect_str(v.frame));
}

/* A Metal view and its layer. */
static void view_state(const char *label, SDL_Window *w, SDL_MetalView view)
{
    UIView *v = (__bridge UIView *)view;
    t("%s: view: %s super=%s tag=%ld root=%s frame=%s", label, object_getClassName(v),
      class_getName(class_getSuperclass(object_getClass(v))), (long)v.tag, yes_no(vc_of(w).view == v),
      rect_str(v.frame));
    CAMetalLayer *layer = (__bridge CAMetalLayer *)SDL_Metal_GetLayer(view);
    int pw = 0, ph = 0;
    SDL_GetWindowSizeInPixels(w, &pw, &ph);
    t("%s: layer: %s same=%s scale=%g drawable=%gx%g pixels=%dx%d format=%lu framebuffer_only=%s device=%s", label,
      layer ? object_getClassName(layer) : "nil", yes_no((__bridge void *)layer == (__bridge void *)v.layer),
      layer.contentsScale, layer.drawableSize.width, layer.drawableSize.height, pw, ph,
      (unsigned long)layer.pixelFormat, yes_no(layer.framebufferOnly), yes_no(layer.device != nil));
}

static void turn(SDL_Window *w, const char *label, unsigned long mask)
{
#if TARGET_OS_TV
    (void)w; (void)mask;
    t("%s: not turned (no orientations on tvOS)", label);
    events(label, 500);
    return;
#else
    List l;
    memset(&l, 0, sizeof(l));
    printf("uikit_metal_ref: %.3f s: turning to %s\n", elapsed(), label);
    if (@available(iOS 16.0, *)) {
        UIWindowSceneGeometryPreferencesIOS *prefs = [[UIWindowSceneGeometryPreferencesIOS alloc] initWithInterfaceOrientations:mask];
        [vc_of(w) setNeedsUpdateOfSupportedInterfaceOrientations];
        [uiwindow_of(w).windowScene requestGeometryUpdateWithPreferences:prefs errorHandler:nil];
    }
    pump_until(&l, "DISPLAY_ORIENTATION", 5000, 1500);
    printf("uikit_metal_ref: %.3f s: turned\n", elapsed());
    t("%s: events: %s", label, list_str(&l));
#endif
}

/* ------------------------------------------------------------------ */
/* The scenes, as sdl3/tests/uikit_metal.rs draws them. */

#define RW 64
#define RH 48

static Uint32 fnv(const Uint8 *p, size_t n, Uint32 h)
{
    for (size_t i = 0; i < n; i++) {
        h ^= p[i];
        h *= 16777619u;
    }
    return h;
}

/* The hash of a surface's pixels as tightly packed RGBA32 (and each row's,
 * printed for comparing by hand). */
static void hash_surface(const char *label, SDL_Surface *s)
{
    if (!s) {
        t("%s: read: error: %s", label, SDL_GetError());
        return;
    }
    SDL_Surface *c = SDL_ConvertSurface(s, SDL_PIXELFORMAT_RGBA32);
    Uint32 h = 2166136261u;
    char rows[RH * 9 * 8 + 1];
    size_t used = 0;
    rows[0] = 0;
    for (int y = 0; y < c->h; y++) {
        const Uint8 *row = (const Uint8 *)c->pixels + y * c->pitch;
        h = fnv(row, (size_t)c->w * 4, h);
        if (used + 10 < sizeof(rows)) {
            used += (size_t)SDL_snprintf(rows + used, sizeof(rows) - used, " %08x", fnv(row, (size_t)c->w * 4, 2166136261u));
        }
    }
    printf("rows %s:%s\n", label, rows);
    t("%s: %dx%d %08x", label, c->w, c->h, h);
    SDL_DestroySurface(c);
    SDL_DestroySurface(s);
}

/* Draw a scene, read back the top left RWxRH pixels, present. */
static void scene(SDL_Renderer *r, const char *name, bool (*draw)(SDL_Renderer *))
{
    char label[96];
    SDL_snprintf(label, sizeof(label), "scene %s", name);
    if (!draw(r)) {
        t("%s: error: %s", label, SDL_GetError());
    }
    SDL_Rect rect = { 0, 0, RW, RH };
    hash_surface(label, SDL_RenderReadPixels(r, &rect));
    SDL_RenderPresent(r);
}

static SDL_Color pattern_color(int x, int y)
{
    SDL_Color c;
    c.r = (Uint8)(x * 16);
    c.g = (Uint8)(y * 16);
    c.b = (Uint8)((x + y) * 8);
    c.a = (x + y) % 3 == 0 ? 255 : (Uint8)(x * 8 + y * 7);
    return c;
}

static SDL_Surface *pattern_surface(SDL_PixelFormat format)
{
    SDL_Surface *s = SDL_CreateSurface(16, 16, format);
    for (int y = 0; y < 16; y++) {
        for (int x = 0; x < 16; x++) {
            SDL_Color c = pattern_color(x, y);
            SDL_WriteSurfacePixel(s, x, y, c.r, c.g, c.b, c.a);
        }
    }
    return s;
}

static Uint8 *yuv_pixels(SDL_PixelFormat format, int w, int h)
{
    SDL_Surface *rgb = SDL_CreateSurface(w, h, SDL_PIXELFORMAT_XRGB8888);
    for (int y = 0; y < h; y++) {
        for (int x = 0; x < w; x++) {
            SDL_Color c = pattern_color(x % 16, y % 16);
            SDL_WriteSurfacePixel(rgb, x, y, c.r, c.g, c.b, c.a);
        }
    }
    size_t size = (size_t)(w * h + 2 * ((w + 1) / 2) * ((h + 1) / 2));
    Uint8 *out = SDL_calloc(1, size);
    SDL_ConvertPixels(w, h, SDL_PIXELFORMAT_XRGB8888, rgb->pixels, rgb->pitch, format, out, w);
    SDL_DestroySurface(rgb);
    return out;
}

static SDL_FRect fr(float x, float y, float w, float h)
{
    SDL_FRect r = { x, y, w, h };
    return r;
}

static bool clear_and_fill(SDL_Renderer *r)
{
    SDL_SetRenderDrawColor(r, 10, 20, 30, 255);
    SDL_RenderClear(r);
    SDL_SetRenderDrawColor(r, 200, 100, 50, 255);
    SDL_FRect a = fr(8, 8, 32, 24);
    SDL_RenderFillRect(r, &a);
    SDL_SetRenderDrawColor(r, 0, 255, 0, 255);
    SDL_FRect b[2] = { { -5, -5, 10, 10 }, { 50, 40, 30, 30 } };
    SDL_RenderFillRects(r, b, 2);
    SDL_SetRenderDrawColor(r, 255, 255, 255, 255);
    SDL_FRect c = fr(2, 30, 12, 10);
    return SDL_RenderRect(r, &c);
}

static bool points_and_lines(SDL_Renderer *r)
{
    SDL_SetRenderDrawColor(r, 0, 0, 0, 255);
    SDL_RenderClear(r);
    SDL_SetRenderDrawColor(r, 255, 128, 0, 255);
    SDL_FPoint points[40];
    for (int i = 0; i < 40; i++) {
        points[i].x = (float)(i * 7 % 64);
        points[i].y = (float)(i * 5 % 48);
    }
    SDL_RenderPoints(r, points, 40);
    SDL_SetRenderDrawColor(r, 255, 255, 0, 255);
    SDL_RenderLine(r, 0, 0, 63, 0);
    SDL_RenderLine(r, 5, 3, 5, 40);
    SDL_RenderLine(r, 10, 10, 40, 30);
    SDL_FPoint lines[3] = { { 50, 5 }, { 60, 5 }, { 60, 15 } };
    return SDL_RenderLines(r, lines, 3);
}

static bool viewport_and_clip(SDL_Renderer *r)
{
    SDL_SetRenderDrawColor(r, 0, 0, 0, 255);
    SDL_RenderClear(r);
    SDL_Rect vp = { 10, 6, 40, 30 };
    SDL_SetRenderViewport(r, &vp);
    SDL_SetRenderDrawColor(r, 255, 0, 0, 255);
    SDL_RenderFillRect(r, NULL);
    SDL_Rect clip = { 5, 5, 12, 8 };
    SDL_SetRenderClipRect(r, &clip);
    SDL_SetRenderDrawColor(r, 0, 0, 255, 255);
    SDL_FRect a = fr(0, 0, 30, 30);
    SDL_RenderFillRect(r, &a);
    SDL_SetRenderClipRect(r, NULL);
    return SDL_SetRenderViewport(r, NULL);
}

static bool blend_modes(SDL_Renderer *r)
{
    SDL_BlendMode modes[4] = { SDL_BLENDMODE_BLEND, SDL_BLENDMODE_ADD, SDL_BLENDMODE_MOD, SDL_BLENDMODE_MUL };
    SDL_SetRenderDrawBlendMode(r, SDL_BLENDMODE_NONE);
    SDL_SetRenderDrawColor(r, 10, 20, 30, 255);
    SDL_RenderClear(r);
    for (int i = 0; i < 4; i++) {
        SDL_SetRenderDrawBlendMode(r, modes[i]);
        SDL_SetRenderDrawColor(r, 220, 90, 40, 128);
        SDL_FRect a = fr((float)i * 12.0f, 12, 10, 30);
        SDL_RenderFillRect(r, &a);
    }
    return true;
}

static SDL_PixelFormat texture_format;

static bool texture_scene(SDL_Renderer *r)
{
    SDL_SetRenderDrawBlendMode(r, SDL_BLENDMODE_NONE);
    SDL_SetRenderDrawColor(r, 40, 50, 60, 255);
    SDL_RenderClear(r);
    SDL_Surface *s = pattern_surface(texture_format);
    SDL_Texture *tex = SDL_CreateTextureFromSurface(r, s);
    SDL_DestroySurface(s);
    if (!tex) {
        return false;
    }
    SDL_SetTextureScaleMode(tex, SDL_SCALEMODE_NEAREST);
    SDL_FRect a = fr(4, 4, 16, 16);
    SDL_RenderTexture(r, tex, NULL, &a);
    SDL_SetTextureColorMod(tex, 255, 128, 64);
    SDL_SetTextureAlphaMod(tex, 160);
    SDL_FRect b = fr(24, 8, 32, 32);
    SDL_RenderTexture(r, tex, NULL, &b);
    SDL_DestroyTexture(tex);
    return true;
}

static bool streaming_texture(SDL_Renderer *r)
{
    SDL_SetRenderDrawColor(r, 0, 0, 0, 255);
    SDL_RenderClear(r);
    SDL_Texture *tex = SDL_CreateTexture(r, SDL_PIXELFORMAT_ARGB8888, SDL_TEXTUREACCESS_STREAMING, 16, 8);
    if (!tex) {
        return false;
    }
    SDL_SetTextureScaleMode(tex, SDL_SCALEMODE_NEAREST);
    Uint8 pixels[16 * 8 * 4];
    for (int i = 0; i < 16 * 8; i++) {
        pixels[i * 4 + 0] = (Uint8)(i * 3);
        pixels[i * 4 + 1] = (Uint8)(i * 5);
        pixels[i * 4 + 2] = 200;
        pixels[i * 4 + 3] = 255;
    }
    SDL_UpdateTexture(tex, NULL, pixels, 64);
    SDL_FRect a = fr(0, 0, 16, 8);
    SDL_RenderTexture(r, tex, NULL, &a);
    for (int i = 0; i < 16 * 8; i++) {
        for (int c = 0; c < 3; c++) {
            pixels[i * 4 + c] = (Uint8)(pixels[i * 4 + c] + 40);
        }
    }
    SDL_UpdateTexture(tex, NULL, pixels, 64);
    SDL_FRect b = fr(20, 20, 16, 8);
    SDL_RenderTexture(r, tex, NULL, &b);
    SDL_DestroyTexture(tex);
    return true;
}

static bool render_target(SDL_Renderer *r)
{
    SDL_Texture *target = SDL_CreateTexture(r, SDL_PIXELFORMAT_ARGB8888, SDL_TEXTUREACCESS_TARGET, 32, 24);
    if (!target) {
        return false;
    }
    SDL_SetRenderTarget(r, target);
    SDL_SetRenderDrawColor(r, 255, 0, 255, 255);
    SDL_RenderClear(r);
    SDL_SetRenderDrawColor(r, 0, 255, 0, 255);
    SDL_FRect a = fr(4, 4, 8, 8);
    SDL_RenderFillRect(r, &a);
    SDL_SetRenderTarget(r, NULL);
    SDL_SetRenderDrawColor(r, 0, 0, 0, 255);
    SDL_RenderClear(r);
    SDL_SetTextureScaleMode(target, SDL_SCALEMODE_NEAREST);
    SDL_FRect b = fr(8, 8, 32, 24);
    SDL_RenderTexture(r, target, NULL, &b);
    SDL_DestroyTexture(target);
    return true;
}

static bool packed_texture(SDL_Renderer *r)
{
    SDL_SetRenderDrawBlendMode(r, SDL_BLENDMODE_NONE);
    SDL_SetRenderDrawColor(r, 40, 50, 60, 255);
    SDL_RenderClear(r);
    SDL_Surface *p = pattern_surface(SDL_PIXELFORMAT_ARGB8888);
    SDL_Surface *s = SDL_ConvertSurface(p, texture_format);
    SDL_DestroySurface(p);
    if (!s) {
        return false;
    }
    SDL_Texture *tex = SDL_CreateTextureFromSurface(r, s);
    SDL_DestroySurface(s);
    if (!tex) {
        return false;
    }
    SDL_SetTextureScaleMode(tex, SDL_SCALEMODE_NEAREST);
    SDL_FRect a = fr(4, 4, 32, 32);
    SDL_RenderTexture(r, tex, NULL, &a);
    SDL_DestroyTexture(tex);
    return true;
}

static bool palette_texture(SDL_Renderer *r)
{
    SDL_SetRenderDrawColor(r, 0, 0, 0, 255);
    SDL_RenderClear(r);
    SDL_Surface *s = SDL_CreateSurface(16, 16, SDL_PIXELFORMAT_INDEX8);
    SDL_Palette *palette = SDL_CreateSurfacePalette(s);
    SDL_Color colors[256];
    for (int i = 0; i < 256; i++) {
        colors[i].r = (Uint8)i;
        colors[i].g = (Uint8)(255 - i);
        colors[i].b = (Uint8)(i * 7);
        colors[i].a = 255;
    }
    SDL_SetPaletteColors(palette, colors, 0, 256);
    for (int y = 0; y < 16; y++) {
        for (int x = 0; x < 16; x++) {
            ((Uint8 *)s->pixels)[y * s->pitch + x] = (Uint8)(x * 16 + y);
        }
    }
    SDL_Texture *tex = SDL_CreateTextureFromSurface(r, s);
    SDL_DestroySurface(s);
    if (!tex) {
        return false;
    }
    SDL_SetTextureScaleMode(tex, SDL_SCALEMODE_NEAREST);
    SDL_FRect a = fr(0, 0, 16, 16);
    SDL_RenderTexture(r, tex, NULL, &a);
    SDL_FRect b = fr(20, 4, 32, 32);
    SDL_RenderTexture(r, tex, NULL, &b);
    SDL_DestroyTexture(tex);
    return true;
}

static bool yuv_texture(SDL_Renderer *r)
{
    SDL_SetRenderDrawColor(r, 0, 0, 0, 255);
    SDL_RenderClear(r);
    SDL_Texture *tex = SDL_CreateTexture(r, texture_format, SDL_TEXTUREACCESS_STATIC, 32, 32);
    if (!tex) {
        return false;
    }
    SDL_SetTextureScaleMode(tex, SDL_SCALEMODE_NEAREST);
    Uint8 *pixels = yuv_pixels(texture_format, 32, 32);
    SDL_UpdateTexture(tex, NULL, pixels, 32);
    SDL_free(pixels);
    SDL_FRect a = fr(8, 8, 32, 32);
    SDL_RenderTexture(r, tex, NULL, &a);
    SDL_DestroyTexture(tex);
    return true;
}

static bool iyuv_planes(SDL_Renderer *r)
{
    SDL_SetRenderDrawColor(r, 0, 0, 0, 255);
    SDL_RenderClear(r);
    SDL_Texture *tex = SDL_CreateTexture(r, SDL_PIXELFORMAT_IYUV, SDL_TEXTUREACCESS_STATIC, 32, 32);
    if (!tex) {
        return false;
    }
    SDL_SetTextureScaleMode(tex, SDL_SCALEMODE_NEAREST);
    Uint8 *pixels = yuv_pixels(SDL_PIXELFORMAT_IYUV, 32, 32);
    SDL_UpdateYUVTexture(tex, NULL, pixels, 32, pixels + 32 * 32, 16, pixels + 32 * 32 + 16 * 16, 16);
    SDL_free(pixels);
    SDL_FRect a = fr(8, 8, 32, 32);
    SDL_RenderTexture(r, tex, NULL, &a);
    SDL_DestroyTexture(tex);
    return true;
}

static bool nv12_planes(SDL_Renderer *r)
{
    SDL_SetRenderDrawColor(r, 0, 0, 0, 255);
    SDL_RenderClear(r);
    SDL_Texture *tex = SDL_CreateTexture(r, SDL_PIXELFORMAT_NV12, SDL_TEXTUREACCESS_STATIC, 32, 32);
    if (!tex) {
        return false;
    }
    SDL_SetTextureScaleMode(tex, SDL_SCALEMODE_NEAREST);
    Uint8 *pixels = yuv_pixels(SDL_PIXELFORMAT_NV12, 32, 32);
    SDL_UpdateNVTexture(tex, NULL, pixels, 32, pixels + 32 * 32, 32);
    SDL_free(pixels);
    SDL_FRect a = fr(8, 8, 32, 32);
    SDL_RenderTexture(r, tex, NULL, &a);
    SDL_DestroyTexture(tex);
    return true;
}

static bool locked_texture(SDL_Renderer *r)
{
    SDL_SetRenderDrawColor(r, 0, 0, 0, 255);
    SDL_RenderClear(r);
    SDL_Texture *tex = SDL_CreateTexture(r, SDL_PIXELFORMAT_ABGR8888, SDL_TEXTUREACCESS_STREAMING, 16, 16);
    if (!tex) {
        return false;
    }
    SDL_SetTextureScaleMode(tex, SDL_SCALEMODE_NEAREST);
    void *pixels = NULL;
    int pitch = 0;
    SDL_Rect rect = { 4, 2, 8, 10 };
    if (!SDL_LockTexture(tex, &rect, &pixels, &pitch)) {
        SDL_DestroyTexture(tex);
        return false;
    }
    for (int y = 0; y < 10; y++) {
        for (int x = 0; x < 8; x++) {
            Uint8 *p = (Uint8 *)pixels + y * pitch + x * 4;
            p[0] = (Uint8)(x * 30);
            p[1] = (Uint8)(y * 25);
            p[2] = 99;
            p[3] = 255;
        }
    }
    SDL_UnlockTexture(tex);
    SDL_FRect a = fr(0, 0, 32, 32);
    SDL_RenderTexture(r, tex, NULL, &a);
    SDL_DestroyTexture(tex);
    return true;
}

static bool locked_nv12(SDL_Renderer *r)
{
    SDL_SetRenderDrawColor(r, 0, 0, 0, 255);
    SDL_RenderClear(r);
    SDL_Texture *tex = SDL_CreateTexture(r, SDL_PIXELFORMAT_NV12, SDL_TEXTUREACCESS_STREAMING, 32, 32);
    if (!tex) {
        return false;
    }
    SDL_SetTextureScaleMode(tex, SDL_SCALEMODE_NEAREST);
    Uint8 *src = yuv_pixels(SDL_PIXELFORMAT_NV12, 32, 32);
    void *pixels = NULL;
    int pitch = 0;
    if (!SDL_LockTexture(tex, NULL, &pixels, &pitch)) {
        SDL_free(src);
        SDL_DestroyTexture(tex);
        return false;
    }
    SDL_memcpy(pixels, src, 32 * 32 + 2 * 16 * 16);
    SDL_UnlockTexture(tex);
    SDL_free(src);
    SDL_FRect a = fr(8, 8, 32, 32);
    SDL_RenderTexture(r, tex, NULL, &a);
    SDL_DestroyTexture(tex);
    return true;
}

static bool custom_blend(SDL_Renderer *r)
{
    SDL_BlendMode mode = SDL_ComposeCustomBlendMode(SDL_BLENDFACTOR_ONE, SDL_BLENDFACTOR_ONE, SDL_BLENDOPERATION_REV_SUBTRACT,
                                                    SDL_BLENDFACTOR_ZERO, SDL_BLENDFACTOR_ONE, SDL_BLENDOPERATION_ADD);
    SDL_SetRenderDrawBlendMode(r, SDL_BLENDMODE_NONE);
    SDL_SetRenderDrawColor(r, 100, 150, 200, 255);
    SDL_RenderClear(r);
    SDL_SetRenderDrawBlendMode(r, mode);
    SDL_SetRenderDrawColor(r, 60, 60, 60, 128);
    SDL_FRect a = fr(10, 10, 30, 20);
    SDL_RenderFillRect(r, &a);
    return SDL_SetRenderDrawBlendMode(r, SDL_BLENDMODE_NONE);
}

static SDL_Vertex vtx(float x, float y, SDL_FColor c, float u, float v)
{
    SDL_Vertex out;
    out.position.x = x;
    out.position.y = y;
    out.color = c;
    out.tex_coord.x = u;
    out.tex_coord.y = v;
    return out;
}

static bool wrapping_geometry(SDL_Renderer *r)
{
    SDL_SetRenderDrawColor(r, 0, 0, 0, 255);
    SDL_RenderClear(r);
    SDL_Surface *s = pattern_surface(SDL_PIXELFORMAT_ARGB8888);
    SDL_Texture *tex = SDL_CreateTextureFromSurface(r, s);
    SDL_DestroySurface(s);
    if (!tex) {
        return false;
    }
    SDL_SetTextureScaleMode(tex, SDL_SCALEMODE_NEAREST);
    SDL_SetTextureBlendMode(tex, SDL_BLENDMODE_NONE);
    SDL_SetRenderTextureAddressMode(r, SDL_TEXTURE_ADDRESS_WRAP, SDL_TEXTURE_ADDRESS_WRAP);
    SDL_FColor white = { 1, 1, 1, 1 };
    SDL_Vertex verts[4] = {
        vtx(0, 0, white, 0, 0), vtx(32, 0, white, 2, 0), vtx(32, 32, white, 2, 2), vtx(0, 32, white, 0, 2)
    };
    int indices[6] = { 0, 1, 2, 0, 2, 3 };
    bool b = SDL_RenderGeometry(r, tex, verts, 4, indices, 6);
    SDL_SetRenderTextureAddressMode(r, SDL_TEXTURE_ADDRESS_AUTO, SDL_TEXTURE_ADDRESS_AUTO);
    SDL_DestroyTexture(tex);
    return b;
}

static bool colored_geometry(SDL_Renderer *r)
{
    SDL_SetRenderDrawColor(r, 0, 0, 0, 255);
    SDL_RenderClear(r);
    SDL_FColor red = { 1, 0, 0, 1 }, green = { 0, 1, 0, 1 }, blue = { 0, 0, 1, 1 };
    SDL_Vertex verts[3] = { vtx(4, 4, red, 0, 0), vtx(60, 10, green, 0, 0), vtx(20, 44, blue, 0, 0) };
    return SDL_RenderGeometry(r, NULL, verts, 3, NULL, 0);
}

static bool linear_scaling(SDL_Renderer *r)
{
    SDL_SetRenderDrawColor(r, 0, 0, 0, 255);
    SDL_RenderClear(r);
    SDL_Surface *s = pattern_surface(SDL_PIXELFORMAT_XRGB8888);
    SDL_Texture *tex = SDL_CreateTextureFromSurface(r, s);
    SDL_DestroySurface(s);
    if (!tex) {
        return false;
    }
    SDL_SetTextureScaleMode(tex, SDL_SCALEMODE_LINEAR);
    SDL_FRect a = fr(0, 0, 48, 40);
    SDL_RenderTexture(r, tex, NULL, &a);
    SDL_DestroyTexture(tex);
    return true;
}

static bool debug_text(SDL_Renderer *r)
{
    SDL_SetRenderDrawColor(r, 0, 0, 0, 255);
    SDL_RenderClear(r);
    SDL_SetRenderDrawColor(r, 255, 255, 255, 255);
    SDL_RenderDebugText(r, 2, 2, "Metal");
    return SDL_RenderDebugText(r, 2, 20, "SDL 3");
}

static bool has_format(const SDL_PixelFormat *formats, SDL_PixelFormat f)
{
    for (int i = 0; formats && formats[i] != SDL_PIXELFORMAT_UNKNOWN; i++) {
        if (formats[i] == f) {
            return true;
        }
    }
    return false;
}

/* The renderer's scenes. */
static void scenes(SDL_Renderer *r)
{
    const SDL_PixelFormat *formats = (const SDL_PixelFormat *)SDL_GetPointerProperty(SDL_GetRendererProperties(r), SDL_PROP_RENDERER_TEXTURE_FORMATS_POINTER, NULL);
    scene(r, "clear and fill", clear_and_fill);
    scene(r, "points and lines", points_and_lines);
    scene(r, "viewport and clip", viewport_and_clip);
    scene(r, "blend modes", blend_modes);
    SDL_PixelFormat rgb[4] = { SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_RGB24, SDL_PIXELFORMAT_XRGB8888 };
    for (int i = 0; i < 4; i++) {
        char name[64];
        texture_format = rgb[i];
        SDL_snprintf(name, sizeof(name), "texture %s", SDL_GetPixelFormatName(rgb[i]));
        scene(r, name, texture_scene);
    }
    scene(r, "streaming texture", streaming_texture);
    scene(r, "render target", render_target);
    SDL_PixelFormat packed[7] = { SDL_PIXELFORMAT_ABGR2101010, SDL_PIXELFORMAT_RGBA64_FLOAT, SDL_PIXELFORMAT_RGBA128_FLOAT,
                                  SDL_PIXELFORMAT_RGB565, SDL_PIXELFORMAT_RGBA5551, SDL_PIXELFORMAT_ARGB1555, SDL_PIXELFORMAT_RGBA4444 };
    int npacked = has_format(formats, SDL_PIXELFORMAT_RGB565) ? 7 : 3;
#if TARGET_OS_SIMULATOR
    /* The simulator's Metal refuses the packed 16-bit formats the renderer
     * lists (a validation assertion ends the process) */
    if (npacked == 7) {
        t("packed 16-bit formats: listed, not drawn");
        npacked = 3;
    }
#endif
    for (int i = 0; i < npacked; i++) {
        char name[64];
        texture_format = packed[i];
        SDL_snprintf(name, sizeof(name), "packed %s", SDL_GetPixelFormatName(packed[i]));
        scene(r, name, packed_texture);
    }
    scene(r, "palette texture", palette_texture);
    SDL_PixelFormat yuv[4] = { SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21 };
    for (int i = 0; i < 4; i++) {
        char name[64];
        texture_format = yuv[i];
        SDL_snprintf(name, sizeof(name), "yuv %s", SDL_GetPixelFormatName(yuv[i]));
        scene(r, name, yuv_texture);
    }
    scene(r, "IYUV planes", iyuv_planes);
    scene(r, "NV12 planes", nv12_planes);
    scene(r, "locked texture", locked_texture);
    scene(r, "locked NV12 texture", locked_nv12);
    scene(r, "custom blend mode", custom_blend);
    scene(r, "wrapping geometry", wrapping_geometry);
    scene(r, "colored geometry", colored_geometry);
    scene(r, "linear scaling", linear_scaling);
    scene(r, "debug text", debug_text);

    /* A partial read, and a read from a render target */
    SDL_SetRenderDrawColor(r, 0, 0, 0, 255);
    SDL_RenderClear(r);
    SDL_SetRenderDrawColor(r, 255, 0, 0, 255);
    SDL_FRect a = fr(10, 10, 4, 4);
    SDL_RenderFillRect(r, &a);
    SDL_Rect part = { 8, 8, 8, 8 };
    hash_surface("partial read", SDL_RenderReadPixels(r, &part));
    SDL_RenderPresent(r);
    SDL_Texture *target = SDL_CreateTexture(r, SDL_PIXELFORMAT_ABGR8888, SDL_TEXTUREACCESS_TARGET, 20, 10);
    SDL_SetRenderTarget(r, target);
    SDL_SetRenderDrawColor(r, 0, 0, 255, 255);
    SDL_RenderClear(r);
    hash_surface("target read", SDL_RenderReadPixels(r, NULL));
    SDL_SetRenderTarget(r, NULL);
    SDL_DestroyTexture(target);
}

/* The renderer's name, sizes, formats and properties. */
static void renderer_state(const char *label, SDL_Renderer *r, SDL_Window *w)
{
    SDL_PropertiesID props = SDL_GetRendererProperties(r);
    int ow = 0, oh = 0, pw = 0, ph = 0, vsync = -2;
    SDL_GetRenderOutputSize(r, &ow, &oh);
    SDL_GetWindowSizeInPixels(w, &pw, &ph);
    SDL_GetRenderVSync(r, &vsync);
    t("%s: name=%s output=%dx%d pixels=%dx%d max_texture_size=%lld vsync=%d", label, SDL_GetRendererName(r), ow, oh, pw, ph,
      (long long)SDL_GetNumberProperty(props, SDL_PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER, -1), vsync);
    List l;
    memset(&l, 0, sizeof(l));
    const SDL_PixelFormat *formats = (const SDL_PixelFormat *)SDL_GetPointerProperty(props, SDL_PROP_RENDERER_TEXTURE_FORMATS_POINTER, NULL);
    for (int i = 0; formats && formats[i] != SDL_PIXELFORMAT_UNKNOWN; i++) {
        list_add(&l, SDL_GetPixelFormatName(formats[i]));
    }
    t("%s: formats: %s", label, list_str(&l));
    id<MTLDevice> device = (__bridge id<MTLDevice>)SDL_GetPointerProperty(props, SDL_PROP_RENDERER_METAL_DEVICE_POINTER, NULL);
    id<MTLCommandQueue> queue = (__bridge id<MTLCommandQueue>)SDL_GetPointerProperty(props, SDL_PROP_RENDERER_METAL_COMMAND_QUEUE_POINTER, NULL);
    t("%s: device=%s queue=%s queue device=%s", label, device ? [device.name UTF8String] : "nil",
      queue ? "yes" : "no", yes_no(queue.device == device));
    CAMetalLayer *layer = (__bridge CAMetalLayer *)SDL_GetRenderMetalLayer(r);
    t("%s: layer: %s drawable=%gx%g format=%lu framebuffer_only=%s device=%s window flags=0x%llx", label,
      layer ? object_getClassName(layer) : "nil", layer.drawableSize.width, layer.drawableSize.height,
      (unsigned long)layer.pixelFormat, yes_no(layer.framebufferOnly), yes_no(layer.device == device),
      (unsigned long long)(SDL_GetWindowFlags(w) & ~SDL_WINDOW_OPENGL));
    UIView *v = vc_of(w).view;
    t("%s: root view: %s tag=%ld layer=%s", label, v ? object_getClassName(v) : "nil", (long)v.tag,
      yes_no((__bridge void *)v.layer == (__bridge void *)layer));
}

static int app_main(int argc, char *argv[])
{
    (void)argc; (void)argv;
    List l;
    printf("uikit_metal_ref: %.3f s: main\n", elapsed());
    bool b = SDL_Init(SDL_INIT_VIDEO);
    t("init video: %s", ok(b));
    t("video driver: %s", SDL_GetCurrentVideoDriver());

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

    /* M: a Metal window, its Metal view made and destroyed */
    SDL_Window *m = SDL_CreateWindow("M", 320, 480, SDL_WINDOW_METAL);
    t("M: create 320x480 metal: %s", ok(m != NULL));
    name_window(m, "M");
    events("M: created", 500);
    window_state("M: created", m);
    t("M: layer of no view: %s", SDL_Metal_GetLayer(NULL) ? "yes" : SDL_GetError());
    SDL_MetalView mv = SDL_Metal_CreateView(m);
    t("M: create view: %s", ok(mv != NULL));
    events("M: view", 500);
    view_state("M: view", m, mv);
    window_state("M: view", m);
    SDL_Metal_DestroyView(mv);
    t("M: view destroyed");
    events("M: view destroyed", 300);
    window_state("M: view destroyed", m);
    SDL_DestroyWindow(m);
    t("M: destroyed");
    events("M: destroyed", 500);

    /* H: a resizable high-density Metal window, turned with its view */
    SDL_Window *h = SDL_CreateWindow("H", 320, 480, SDL_WINDOW_METAL | SDL_WINDOW_RESIZABLE | SDL_WINDOW_HIGH_PIXEL_DENSITY);
    t("H: create 320x480 metal resizable high-density: %s", ok(h != NULL));
    name_window(h, "H");
    events("H: created", 500);
    window_state("H: created", h);
    SDL_MetalView hv = SDL_Metal_CreateView(h);
    t("H: create view: %s", ok(hv != NULL));
    events("H: view", 500);
    view_state("H: view", h, hv);
    turn(h, "H: landscape right", (1 << 3));
    view_state("H: landscape right", h, hv);
    turn(h, "H: portrait", (1 << 1));
    view_state("H: portrait", h, hv);
    SDL_Metal_DestroyView(hv);
    t("H: view destroyed");
    events("H: view destroyed", 300);
    window_state("H: view destroyed", h);

    /* The Metal renderer on H (the first one tried) */
    SDL_Renderer *r = SDL_CreateRenderer(h, NULL);
    t("H: create renderer: %s", ok(r != NULL));
    events("H: renderer", 500);
    renderer_state("H: renderer", r, h);
    t("H: renderer: command encoder: %s", SDL_GetRenderMetalCommandEncoder(r) ? "yes" : "no");
    SDL_RenderPresent(r);
    scenes(r);
    int vsync = -2;
    for (int i = 0; i < 4; i++) {
        int want[4] = { 1, 0, 2, -1 };
        bool vb = SDL_SetRenderVSync(r, want[i]);
        SDL_GetRenderVSync(r, &vsync);
        t("H: vsync %d: %s, vsync=%d", want[i], ok(vb), vsync);
        SDL_SetRenderDrawColor(r, 0, 0, 0, 255);
        SDL_RenderClear(r);
        SDL_RenderPresent(r);
    }
    events("H: scenes", 300);

    /* The drawable follows the window as it turns */
    turn(h, "H: renderer landscape right", (1 << 3));
    renderer_state("H: renderer landscape right", r, h);
    SDL_SetRenderDrawColor(r, 0, 128, 0, 255);
    SDL_RenderClear(r);
    int ow = 0, oh = 0;
    SDL_GetRenderOutputSize(r, &ow, &oh);
    SDL_Rect corner = { ow - RW, oh - RH, RW, RH };
    hash_surface("H: renderer landscape right corner", SDL_RenderReadPixels(r, &corner));
    SDL_RenderPresent(r);
    turn(h, "H: renderer portrait", (1 << 1));
    renderer_state("H: renderer portrait", r, h);
    scene(r, "again clear and fill", clear_and_fill);

    UIView *view_before = vc_of(h).view;
    SDL_DestroyRenderer(r);
    t("H: renderer destroyed");
    events("H: renderer destroyed", 300);
    window_state("H: renderer destroyed", h);
    /* Made again: the window's Metal view (GetWindowView()) */
    r = SDL_CreateRenderer(h, "metal");
    t("H: create metal renderer again: %s", ok(r != NULL));
    t("H: renderer again: same view=%s", yes_no(vc_of(h).view == view_before));
    renderer_state("H: renderer again", r, h);
    scene(r, "renderer again clear and fill", clear_and_fill);
    SDL_DestroyRenderer(r);
    t("H: renderer again destroyed");
    events("H: renderer again destroyed", 300);

    /* The GPU API: the simulator's device isn't enough for its Metal backend */
    t("gpu: supports MSL on metal: %s", yes_no(SDL_GPUSupportsShaderFormats(SDL_GPU_SHADERFORMAT_MSL, "metal")));
    SDL_GPUDevice *device = SDL_CreateGPUDevice(SDL_GPU_SHADERFORMAT_MSL | SDL_GPU_SHADERFORMAT_METALLIB, false, "metal");
    t("gpu: create metal device: %s", ok(device != NULL));
    if (device) {
        SDL_DestroyGPUDevice(device);
    }
    SDL_Renderer *gr = SDL_CreateRenderer(h, "gpu");
    t("H: create gpu renderer: %s", ok(gr != NULL));
    if (gr) {
        SDL_DestroyRenderer(gr);
    }
    events("H: gpu", 300);

    SDL_DestroyWindow(h);
    t("H: destroyed");
    events("H: destroyed", 500);

    SDL_Quit();
    t("quit");
    exit_line(0);
    exit(0);
}

int main(int argc, char *argv[])
{
    elapsed();
    int status = SDL_RunApp(argc, argv, app_main, NULL);
    printf("uikit_metal_ref: SDL_RunApp returned %d\n", status);
    exit_line(1);
    return 1;
}
