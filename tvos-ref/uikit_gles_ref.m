/* Upstream reference for sdl3/tests/uikit_gles.rs (diag branch only). */
#define SDL_MAIN_HANDLED
#include "common.h"
#include <SDL3/SDL_main.h>
#import <OpenGLES/EAGL.h>
#import <OpenGLES/EAGLDrawable.h>
#import <OpenGLES/ES3/gl.h>
#import <QuartzCore/CAEAGLLayer.h>

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
    static char buf[4][512];
    static int n;
    char *out = buf[n++ % 4];
    if (b) {
        return "ok";
    }
    SDL_snprintf(out, 512, "error: %s", SDL_GetError());
    return out;
}

static UIWindow *uiwindow_of(SDL_Window *w)
{
    return (__bridge UIWindow *)SDL_GetPointerProperty(SDL_GetWindowProperties(w), SDL_PROP_WINDOW_UIKIT_WINDOW_POINTER, NULL);
}

static UIViewController *vc_of(SDL_Window *w)
{
    return uiwindow_of(w).rootViewController;
}

static UIView *root_view_of(SDL_Window *w)
{
    return vc_of(w).view;
}

static const char *cname(id obj)
{
    return obj ? object_getClassName(obj) : "nil";
}

static const char *rect_str(CGRect r)
{
    static char buf[4][96];
    static int n;
    char *b = buf[n++ % 4];
    SDL_snprintf(b, 96, "%g,%g %gx%g", r.origin.x, r.origin.y, r.size.width, r.size.height);
    return b;
}

static void window_state(const char *label, SDL_Window *w)
{
    int ww = 0, wh = 0, pw = 0, ph = 0;
    SDL_GetWindowSize(w, &ww, &wh);
    SDL_GetWindowSizeInPixels(w, &pw, &ph);
    t("%s: size=%dx%d pixels=%dx%d density=%g flags=0x%llx", label, ww, wh, pw, ph,
      (double)SDL_GetWindowPixelDensity(w), (unsigned long long)SDL_GetWindowFlags(w));
    UIView *v = root_view_of(w);
    t("%s: root view: %s frame=%s", label, cname(v), rect_str(v ? v.frame : CGRectZero));
}

static void gl_props(SDL_Window *w, long long *fb, long long *rb, long long *resolve)
{
    SDL_PropertiesID p = SDL_GetWindowProperties(w);
    *fb = (long long)SDL_GetNumberProperty(p, SDL_PROP_WINDOW_UIKIT_OPENGL_FRAMEBUFFER_NUMBER, -1);
    *rb = (long long)SDL_GetNumberProperty(p, SDL_PROP_WINDOW_UIKIT_OPENGL_RENDERBUFFER_NUMBER, -1);
    *resolve = (long long)SDL_GetNumberProperty(p, SDL_PROP_WINDOW_UIKIT_OPENGL_RESOLVE_FRAMEBUFFER_NUMBER, -1);
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
    printf("uikit_gles_ref: %.3f s: turning to %s\n", elapsed(), label);
    if (@available(iOS 16.0, *)) {
        UIWindowSceneGeometryPreferencesIOS *prefs = [[UIWindowSceneGeometryPreferencesIOS alloc] initWithInterfaceOrientations:mask];
        [vc_of(w) setNeedsUpdateOfSupportedInterfaceOrientations];
        [uiwindow_of(w).windowScene requestGeometryUpdateWithPreferences:prefs errorHandler:nil];
    }
    pump_until(&l, "DISPLAY_ORIENTATION", 5000, 1500);
    printf("uikit_gles_ref: %.3f s: turned\n", elapsed());
    t("%s: events: %s", label, list_str(&l));
#endif
}

#include "scenes.h"

/* ------------------------------------------------------------------ */
/* OpenGL ES (linked: the same functions SDL_GL_GetProcAddress() finds) */

static const char *gl_string(GLenum name)
{
    static char buf[4][256];
    static int n;
    char *b = buf[n++ % 4];
    const GLubyte *s = glGetString(name);
    SDL_snprintf(b, 256, "%s", s ? (const char *)s : "(null)");
    return b;
}

static GLint gl_integer(GLenum pname)
{
    GLint v = 0;
    glGetIntegerv(pname, &v);
    return v;
}

static GLint rb_param(GLenum pname)
{
    GLint v = 0;
    glGetRenderbufferParameteriv(GL_RENDERBUFFER, pname, &v);
    return v;
}

static bool eagl_is(SDL_GLContext ctx)
{
    return (__bridge void *)[EAGLContext currentContext] == (void *)ctx;
}

static void attributes(const char *label, bool with_context)
{
    static const struct { const char *name; SDL_GLAttr attr; bool asks_gl; } attrs[] = {
        { "red", SDL_GL_RED_SIZE, true },
        { "green", SDL_GL_GREEN_SIZE, true },
        { "blue", SDL_GL_BLUE_SIZE, true },
        { "alpha", SDL_GL_ALPHA_SIZE, true },
        { "buffer", SDL_GL_BUFFER_SIZE, true },
        { "depth", SDL_GL_DEPTH_SIZE, true },
        { "stencil", SDL_GL_STENCIL_SIZE, true },
        { "double", SDL_GL_DOUBLEBUFFER, false },
        { "msbuffers", SDL_GL_MULTISAMPLEBUFFERS, true },
        { "mssamples", SDL_GL_MULTISAMPLESAMPLES, true },
        { "accelerated", SDL_GL_ACCELERATED_VISUAL, false },
        { "retained", SDL_GL_RETAINED_BACKING, false },
        { "major", SDL_GL_CONTEXT_MAJOR_VERSION, false },
        { "minor", SDL_GL_CONTEXT_MINOR_VERSION, false },
        { "profile", SDL_GL_CONTEXT_PROFILE_MASK, false },
        { "share", SDL_GL_SHARE_WITH_CURRENT_CONTEXT, false },
        { "srgb", SDL_GL_FRAMEBUFFER_SRGB_CAPABLE, false },
        { "accum_red", SDL_GL_ACCUM_RED_SIZE, false },
        { "stereo", SDL_GL_STEREO, false },
        { "release", SDL_GL_CONTEXT_RELEASE_BEHAVIOR, true },
    };
    char out[2048];
    size_t used = 0;
    out[0] = 0;
    for (size_t i = 0; i < SDL_arraysize(attrs); i++) {
        if (!with_context && attrs[i].asks_gl) {
            continue;
        }
        int v = 0;
        if (SDL_GL_GetAttribute(attrs[i].attr, &v)) {
            used += (size_t)SDL_snprintf(out + used, sizeof(out) - used, "%s%s=%d", used ? " " : "", attrs[i].name, v);
        } else {
            used += (size_t)SDL_snprintf(out + used, sizeof(out) - used, "%s%s=(%s)", used ? " " : "", attrs[i].name, SDL_GetError());
        }
    }
    t("%s: attributes: %s", label, out);
}

static const char *drawable_properties(UIView *view)
{
    static char buf[512];
    CAEAGLLayer *layer = (CAEAGLLayer *)view.layer;
    NSDictionary *props = layer.drawableProperties;
    NSNumber *retained = props[kEAGLDrawablePropertyRetainedBacking];
    NSString *format = props[kEAGLDrawablePropertyColorFormat];
    SDL_snprintf(buf, sizeof(buf), "%s scale=%g opaque=%s retained=%s format=%s", cname(layer), layer.contentsScale,
                 yes_no(layer.opaque), retained ? yes_no(retained.boolValue) : "nil", format ? [format UTF8String] : "nil");
    return buf;
}

static void context_state(const char *label, SDL_Window *w, SDL_GLContext ctx)
{
    EAGLContext *raw = (__bridge EAGLContext *)ctx;
    SDL_GLContext current = SDL_GL_GetCurrentContext();
    unsigned long api = (unsigned long)raw.API;
    t("%s: current: sdl=%s window=%s eagl=%s class=%s api=%lu", label, yes_no(current == ctx),
      yes_no(SDL_GL_GetCurrentWindow() == w), yes_no(eagl_is(ctx)), cname(raw), api);
    char shading[256];
    if (api >= 2) {
        SDL_snprintf(shading, sizeof(shading), "%s", gl_string(GL_SHADING_LANGUAGE_VERSION));
    } else {
        SDL_snprintf(shading, sizeof(shading), "-");
    }
    const char *version = gl_string(GL_VERSION);
    const char *renderer = gl_string(GL_RENDERER);
    const char *vendor = gl_string(GL_VENDOR);
    t("%s: version=%s renderer=%s vendor=%s shading=%s", label, version, renderer, vendor, shading);
    attributes(label, true);
    long long fb, rb, resolve;
    gl_props(w, &fb, &rb, &resolve);
    GLint bound_fb = gl_integer(GL_FRAMEBUFFER_BINDING);
    GLint bound_rb = gl_integer(GL_RENDERBUFFER_BINDING);
    t("%s: props: framebuffer=%lld renderbuffer=%lld resolve=%lld bound framebuffer=%d renderbuffer=%d", label, fb, rb,
      resolve, bound_fb, bound_rb);
    UIView *v = root_view_of(w);
    int pw = 0, ph = 0;
    SDL_GetWindowSizeInPixels(w, &pw, &ph);
    t("%s: view: %s super=%s frame=%s layer: %s", label, cname(v), class_getName(class_getSuperclass(object_getClass(v))),
      rect_str(v.frame), drawable_properties(v));
    GLint rw = rb_param(GL_RENDERBUFFER_WIDTH);
    GLint rh = rb_param(GL_RENDERBUFFER_HEIGHT);
    GLint rf = rb_param(GL_RENDERBUFFER_INTERNAL_FORMAT);
    t("%s: renderbuffer: %dx%d format=0x%x pixels=%dx%d", label, rw, rh, rf, pw, ph);
    GLint samples = gl_integer(GL_SAMPLES);
    GLint max_samples = gl_integer(GL_MAX_SAMPLES);
    GLenum error = glGetError();
    t("%s: samples=%d max_samples=%d error=0x%x", label, samples, max_samples, error);
}

static void draw_and_swap(const char *label, SDL_Window *w, bool read)
{
    glClearColor(0.25f, 0.5f, 0.75f, 1.0f);
    glClear(GL_COLOR_BUFFER_BIT);
    char pixel[64];
    if (read) {
        GLubyte p[4] = { 0, 0, 0, 0 };
        glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, p);
        SDL_snprintf(pixel, sizeof(pixel), "[%d, %d, %d, %d]", p[0], p[1], p[2], p[3]);
    } else {
        SDL_snprintf(pixel, sizeof(pixel), "-");
    }
    GLenum error = glGetError();
    bool swap = SDL_GL_SwapWindow(w);
    const char *swap_ok = ok(swap);
    GLenum error2 = glGetError();
    t("%s: clear: pixel=%s error=0x%x; swap: %s, error=0x%x", label, pixel, error, swap_ok, error2);
}

static void context_round(const char *label, SDL_Window *w, bool read)
{
    SDL_GLContext ctx = SDL_GL_CreateContext(w);
    t("%s: create context: %s", label, ok(ctx != NULL));
    if (!ctx) {
        return;
    }
    context_state(label, w, ctx);
    draw_and_swap(label, w, read);
    bool set = SDL_GL_SetSwapInterval(1);
    char set_ok[512];
    SDL_snprintf(set_ok, sizeof(set_ok), "%s", ok(set));
    int interval = 0;
    bool get = SDL_GL_GetSwapInterval(&interval);
    char get_s[512];
    if (get) {
        SDL_snprintf(get_s, sizeof(get_s), "%d", interval);
    } else {
        SDL_snprintf(get_s, sizeof(get_s), "error: %s", SDL_GetError());
    }
    t("%s: swap interval: set 1: %s; get: %s", label, set_ok, get_s);
    bool d = SDL_GL_DestroyContext(ctx);
    UIView *v = root_view_of(w);
    long long fb, rb, resolve;
    gl_props(w, &fb, &rb, &resolve);
    t("%s: destroy: %s; current: sdl=%s eagl=%s; root view: %s; props: %lld %lld %lld", label, ok(d),
      yes_no(SDL_GL_GetCurrentContext() != NULL), yes_no([EAGLContext currentContext] != nil), cname(v), fb, rb, resolve);
}

static void set_attr(SDL_GLAttr attr, int value)
{
    if (!SDL_GL_SetAttribute(attr, value)) {
        printf("uikit_gles_ref: SDL_GL_SetAttribute failed: %s\n", SDL_GetError());
    }
}

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
    SDL_GLContext ctx = SDL_GL_GetCurrentContext();
    UIView *v = root_view_of(w);
    long long fb, rb, resolve;
    gl_props(w, &fb, &rb, &resolve);
    GLint bound = gl_integer(GL_FRAMEBUFFER_BINDING);
    t("%s: context: %s eagl=%s window flags=0x%llx root view: %s props: %lld %lld %lld bound framebuffer=%d", label,
      ctx ? cname((__bridge id)ctx) : "nil", yes_no(ctx && eagl_is(ctx)), (unsigned long long)SDL_GetWindowFlags(w),
      cname(v), fb, rb, resolve, bound);
}

static void check_custom_blend(SDL_Renderer *r)
{
    custom_blend(r);
    SDL_Surface *s = SDL_RenderReadPixels(r, NULL);
    Uint8 cr, cg, cb, ca;
    SDL_ReadSurfacePixel(s, 20, 20, &cr, &cg, &cb, &ca);
    if (cr != 40 || cg != 90 || cb != 140 || ca != 255) {
        printf("uikit_gles_ref: custom blend at 20,20: %d,%d,%d,%d\n", cr, cg, cb, ca);
    }
    SDL_DestroySurface(s);
    SDL_RenderPresent(r);
}

static int assertions;

static SDL_AssertState SDLCALL count_assertions(const SDL_AssertData *data, void *userdata)
{
    (void)userdata;
    printf("uikit_gles_ref: assertion: %s\n", data->condition);
    assertions++;
    return SDL_ASSERTION_IGNORE;
}

static EAGLContext *other;
static SDL_GLContext ours;
static SDL_Window *anim_window;
static int nframes;
static bool frames[2];

static void SDLCALL animation(void *param)
{
    (void)param;
    if (nframes < 2) {
        frames[nframes++] = (__bridge void *)[EAGLContext currentContext] == (void *)ours;
        [EAGLContext setCurrentContext:other];
    }
    if (nframes == 2) {
        SDL_SetiOSAnimationCallback(anim_window, 1, NULL, NULL);
    }
}

static const char *power_state_name(SDL_PowerState s)
{
    switch (s) {
    case SDL_POWERSTATE_ERROR: return "Error";
    case SDL_POWERSTATE_UNKNOWN: return "Unknown";
    case SDL_POWERSTATE_ON_BATTERY: return "OnBattery";
    case SDL_POWERSTATE_NO_BATTERY: return "NoBattery";
    case SDL_POWERSTATE_CHARGING: return "Charging";
    case SDL_POWERSTATE_CHARGED: return "Charged";
    }
    return "?";
}

static int app_main(int argc, char *argv[])
{
    (void)argc; (void)argv;
    const unsigned long LANDSCAPE_RIGHT = 1 << 3;
    const unsigned long PORTRAIT = 1 << 1;
    printf("uikit_gles_ref: %.3f s: main\n", elapsed());
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

    attributes("defaults", false);

    /* D: a window asking for no graphics API (the OpenGL flag) */
    SDL_Window *d = SDL_CreateWindow("D", 320, 480, 0);
    t("D: create 320x480: %s", ok(d != NULL));
    name_window(d, "D");
    events("D: created", 500);
    window_state("D: created", d);

    /* The versions */
    int versions[5][2] = { { 2, 0 }, { 3, 0 }, { 1, 1 }, { 3, 1 }, { 4, 0 } };
    for (int i = 0; i < 5; i++) {
        char label[64];
        set_attr(SDL_GL_CONTEXT_MAJOR_VERSION, versions[i][0]);
        set_attr(SDL_GL_CONTEXT_MINOR_VERSION, versions[i][1]);
        SDL_snprintf(label, sizeof(label), "D: es %d.%d", versions[i][0], versions[i][1]);
        context_round(label, d, true);
    }
    events("D: versions", 300);
    set_attr(SDL_GL_CONTEXT_MAJOR_VERSION, 2);
    set_attr(SDL_GL_CONTEXT_MINOR_VERSION, 0);

    /* Share groups */
    SDL_GLContext a = SDL_GL_CreateContext(d);
    t("D: context A: %s", ok(a != NULL));
    UIView *a_view = root_view_of(d);
    GLuint texture = 0;
    glGenTextures(1, &texture);
    glBindTexture(GL_TEXTURE_2D, texture);
    set_attr(SDL_GL_SHARE_WITH_CURRENT_CONTEXT, 1);
    SDL_GLContext bc = SDL_GL_CreateContext(d);
    {
        bool same = ((__bridge EAGLContext *)a).sharegroup == ((__bridge EAGLContext *)bc).sharegroup;
        bool tex = glIsTexture(texture);
        bool cur = SDL_GL_GetCurrentContext() == bc;
        bool rv = root_view_of(d) == a_view;
        t("D: context B (shared): same group=%s texture=%s current=%s root view is A's=%s", yes_no(same), yes_no(tex),
          yes_no(cur), yes_no(rv));
    }
    set_attr(SDL_GL_SHARE_WITH_CURRENT_CONTEXT, 0);
    SDL_GLContext c = SDL_GL_CreateContext(d);
    {
        bool same = ((__bridge EAGLContext *)a).sharegroup == ((__bridge EAGLContext *)c).sharegroup;
        bool tex = glIsTexture(texture);
        t("D: context C (not shared): same group=%s texture=%s", yes_no(same), yes_no(tex));
    }
    {
        bool r = SDL_GL_MakeCurrent(d, a);
        const char *rok = ok(r);
        bool tex = glIsTexture(texture);
        bool rv = root_view_of(d) == a_view;
        bool e = eagl_is(a);
        t("D: A current again: %s texture=%s root view is A's=%s eagl=%s", rok, yes_no(tex), yes_no(rv), yes_no(e));
    }
    glDeleteTextures(1, &texture);
    SDL_GL_DestroyContext(bc);
    SDL_GL_DestroyContext(c);
    t("D: B and C destroyed: A current=%s root view is A's=%s", yes_no(SDL_GL_GetCurrentContext() == a),
      yes_no(root_view_of(d) == a_view));

    /* The current context restored */
    other = [[EAGLContext alloc] initWithAPI:kEAGLRenderingAPIOpenGLES2];
    [EAGLContext setCurrentContext:other];
    List seen;
    memset(&seen, 0, sizeof(seen));
    take_queued(&seen);
    t("D: pumped after another context: ours=%s", yes_no(eagl_is(a)));
    ours = a;
    anim_window = d;
    [EAGLContext setCurrentContext:other];
    SDL_SetiOSAnimationCallback(d, 1, animation, NULL);
    end = SDL_GetTicks() + 5000;
    while (nframes < 2 && SDL_GetTicks() < end) {
        memset(&seen, 0, sizeof(seen));
        take_queued(&seen);
        SDL_Delay(5);
    }
    {
        List fl;
        memset(&fl, 0, sizeof(fl));
        for (int i = 0; i < nframes; i++) {
            list_add(&fl, yes_no(frames[i]));
        }
        t("D: animation callbacks after another context: ours=%s", list_str(&fl));
    }
    [EAGLContext setCurrentContext:other];
    {
        bool r = SDL_RaiseWindow(d);
        const char *rok = ok(r);
        t("D: raised after another context: %s ours=%s", rok, yes_no(eagl_is(a)));
    }

    /* The battery monitoring the swaps turn off */
    SDL_SetAssertionHandler(count_assertions, NULL);
    int pct = -1, secs = -1;
    SDL_PowerState state = SDL_GetPowerInfo(&secs, &pct);
    {
        char percent[32];
        if (pct < 0) {
            SDL_snprintf(percent, sizeof(percent), "None");
        } else {
            SDL_snprintf(percent, sizeof(percent), "Some(%d)", pct);
        }
        t("D: power: state=%s percent=%s", power_state_name(state), percent);
    }
    SDL_GL_SwapWindow(d);
    int before = assertions;
    SDL_Delay(3100);
    SDL_GL_SwapWindow(d);
    int after = assertions;
    SDL_GL_SwapWindow(d);
    t("D: battery: assertions: swap at once %d, after 3 s %d, again %d", before, after, assertions);
    SDL_SetAssertionHandler(NULL, NULL);

    SDL_GL_DestroyContext(a);
    other = nil;
    {
        bool r = SDL_RaiseWindow(d);
        const char *rok = ok(r);
        t("D: A destroyed: raised: %s eagl=%s root view: %s", rok, yes_no([EAGLContext currentContext] != nil),
          cname(root_view_of(d)));
    }
    events("D: contexts", 300);

    /* The framebuffer's formats */
    set_attr(SDL_GL_FRAMEBUFFER_SRGB_CAPABLE, 1);
    context_round("D: srgb", d, true);
    SDL_SetHint(SDL_HINT_OPENGL_FORCE_SRGB_FRAMEBUFFER, "0");
    context_round("D: srgb forced off", d, true);
    set_attr(SDL_GL_FRAMEBUFFER_SRGB_CAPABLE, 0);
    SDL_SetHint(SDL_HINT_OPENGL_FORCE_SRGB_FRAMEBUFFER, "1");
    context_round("D: srgb forced on", d, true);
    SDL_ResetHint(SDL_HINT_OPENGL_FORCE_SRGB_FRAMEBUFFER);
    set_attr(SDL_GL_RED_SIZE, 5);
    set_attr(SDL_GL_GREEN_SIZE, 6);
    set_attr(SDL_GL_BLUE_SIZE, 5);
    set_attr(SDL_GL_ALPHA_SIZE, 0);
    context_round("D: rgb565", d, true);
    SDL_GL_ResetAttributes();
    set_attr(SDL_GL_DEPTH_SIZE, 24);
    set_attr(SDL_GL_STENCIL_SIZE, 8);
    context_round("D: depth 24 stencil 8", d, true);
    set_attr(SDL_GL_DEPTH_SIZE, 0);
    set_attr(SDL_GL_STENCIL_SIZE, 0);
    set_attr(SDL_GL_RETAINED_BACKING, 0);
    context_round("D: no depth, not retained", d, true);
    SDL_GL_ResetAttributes();
    set_attr(SDL_GL_MULTISAMPLEBUFFERS, 1);
    set_attr(SDL_GL_MULTISAMPLESAMPLES, 4);
    context_round("D: msaa 4 es 2.0", d, false);
    set_attr(SDL_GL_CONTEXT_MAJOR_VERSION, 3);
    context_round("D: msaa 4 es 3.0", d, false);
    set_attr(SDL_GL_RETAINED_BACKING, 0);
    set_attr(SDL_GL_MULTISAMPLESAMPLES, 64);
    context_round("D: msaa 64 es 3.0 not retained", d, false);
    set_attr(SDL_GL_CONTEXT_MAJOR_VERSION, 2);
    context_round("D: msaa 64 es 2.0 not retained", d, false);
    SDL_GL_ResetAttributes();
    events("D: formats", 300);

    SDL_DestroyWindow(d);
    t("D: destroyed");
    events("D: destroyed", 500);

    /* H: a resizable high-density OpenGL window, turned with a context */
    SDL_Window *h = SDL_CreateWindow("H", 320, 480, SDL_WINDOW_OPENGL | SDL_WINDOW_RESIZABLE | SDL_WINDOW_HIGH_PIXEL_DENSITY);
    t("H: create 320x480 opengl resizable high-density: %s", ok(h != NULL));
    name_window(h, "H");
    events("H: created", 500);
    window_state("H: created", h);
    set_attr(SDL_GL_CONTEXT_MAJOR_VERSION, 3);
    SDL_GLContext ctx = SDL_GL_CreateContext(h);
    t("H: create es 3.0 context: %s", ok(ctx != NULL));
    events("H: context", 300);
    context_state("H: context", h, ctx);
    draw_and_swap("H: context", h, true);
    turn(h, "H: landscape right", LANDSCAPE_RIGHT);
    context_state("H: landscape right", h, ctx);
    draw_and_swap("H: landscape right", h, true);
    turn(h, "H: portrait", PORTRAIT);
    context_state("H: portrait", h, ctx);
    SDL_GL_DestroyContext(ctx);
    SDL_GL_ResetAttributes();
    t("H: context destroyed");
    events("H: context destroyed", 300);
    window_state("H: context destroyed", h);

    /* The OpenGL ES 2 renderer on H */
    debug_text_label = "GLES2";
    SDL_Renderer *r = SDL_CreateRenderer(h, "opengles2");
    t("H: create opengles2 renderer: %s", ok(r != NULL));
    events("H: renderer", 500);
    renderer_state("H: renderer", r, h);
    SDL_RenderPresent(r);
    scenes(r);
    int vsync = -2;
    for (int i = 0; i < 4; i++) {
        int want[4] = { 1, 0, 2, -1 };
        bool vb = SDL_SetRenderVSync(r, want[i]);
        const char *vok = ok(vb);
        SDL_GetRenderVSync(r, &vsync);
        t("H: vsync %d: %s, vsync=%d", want[i], vok, vsync);
        SDL_SetRenderDrawColor(r, 0, 0, 0, 255);
        SDL_RenderClear(r);
        SDL_RenderPresent(r);
    }
    events("H: scenes", 300);

    /* The output follows the window as it turns */
    turn(h, "H: renderer landscape right", LANDSCAPE_RIGHT);
    renderer_state("H: renderer landscape right", r, h);
    SDL_SetRenderDrawColor(r, 0, 128, 0, 255);
    SDL_RenderClear(r);
    int ow = 0, oh = 0;
    SDL_GetRenderOutputSize(r, &ow, &oh);
    SDL_Rect corner = { ow - RW, oh - RH, RW, RH };
    hash_surface("H: renderer landscape right corner", SDL_RenderReadPixels(r, &corner));
    SDL_RenderPresent(r);
    turn(h, "H: renderer portrait", PORTRAIT);
    renderer_state("H: renderer portrait", r, h);
    scene(r, "again clear and fill", clear_and_fill);
    check_custom_blend(r);
    SDL_DestroyRenderer(r);
    t("H: renderer destroyed");
    events("H: renderer destroyed", 300);
    window_state("H: renderer destroyed", h);
    SDL_DestroyWindow(h);
    t("H: destroyed");
    events("H: destroyed", 500);

    /* The OpenGL ES 2 renderer on a Metal window: made again */
    SDL_Window *m = SDL_CreateWindow("M", 320, 480, SDL_WINDOW_METAL);
    t("M: create 320x480 metal: %s", ok(m != NULL));
    name_window(m, "M");
    events("M: created", 500);
    window_state("M: created", m);
    r = SDL_CreateRenderer(m, "opengles2");
    t("M: create opengles2 renderer: %s", ok(r != NULL));
    events("M: renderer", 500);
    window_state("M: renderer", m);
    renderer_state("M: renderer", r, m);
    scene(r, "M clear and fill", clear_and_fill);
    SDL_DestroyRenderer(r);
    events("M: renderer destroyed", 300);
    SDL_DestroyWindow(m);
    t("M: destroyed");
    events("M: destroyed", 500);

    /* The default renderer on a window asking for no graphics API */
    SDL_Window *w = SDL_CreateWindow("W", 320, 480, 0);
    t("W: create 320x480: %s", ok(w != NULL));
    name_window(w, "W");
    events("W: created", 500);
    window_state("W: created", w);
    r = SDL_CreateRenderer(w, NULL);
    t("W: create renderer: %s", ok(r != NULL));
    events("W: renderer", 500);
    t("W: renderer: name=%s", r ? SDL_GetRendererName(r) : "(none)");
    window_state("W: renderer", w);
    SDL_DestroyRenderer(r);
    events("W: renderer destroyed", 300);
    SDL_DestroyWindow(w);
    t("W: destroyed");
    events("W: destroyed", 500);

    SDL_Quit();
    t("quit");
    exit_line(0);
    exit(0);
}

int main(int argc, char *argv[])
{
    elapsed();
    int status = SDL_RunApp(argc, argv, app_main, NULL);
    printf("uikit_gles_ref: SDL_RunApp returned %d\n", status);
    exit_line(1);
    return 1;
}
