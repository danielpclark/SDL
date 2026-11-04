/* diag only: GBM + EGL (EGL_PLATFORM_GBM_KHR) + GLES2 on a DRM card, as
 * SDL's kmsdrm driver does: a gbm_surface, an EGL window surface on it, a
 * clear, eglSwapBuffers, the front buffer as a framebuffer on the CRTC.
 * Usage: egl_gbm /dev/dri/cardN */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <xf86drm.h>
#include <xf86drmMode.h>
#include <gbm.h>
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>

int main(int argc, char **argv)
{
    int fd = open(argv[1], O_RDWR | O_CLOEXEC);
    if (fd < 0) {
        printf("open: %s\n", strerror(errno));
        return 1;
    }
    struct gbm_device *gbm = gbm_create_device(fd);
    printf("gbm device %p backend %s\n", (void *)gbm, gbm ? gbm_device_get_backend_name(gbm) : "-");
    if (!gbm) return 1;
    printf("gbm XRGB8888 scanout|rendering supported %d\n", gbm_device_is_format_supported(gbm, GBM_FORMAT_XRGB8888, GBM_BO_USE_SCANOUT | GBM_BO_USE_RENDERING));
    printf("gbm ARGB8888 cursor|write supported %d\n", gbm_device_is_format_supported(gbm, GBM_FORMAT_ARGB8888, GBM_BO_USE_CURSOR | GBM_BO_USE_WRITE));
    const char *cext = eglQueryString(EGL_NO_DISPLAY, EGL_EXTENSIONS);
    printf("EGL client extensions: %s\n", cext ? cext : "(none)");
    PFNEGLGETPLATFORMDISPLAYEXTPROC gpd = (PFNEGLGETPLATFORMDISPLAYEXTPROC)eglGetProcAddress("eglGetPlatformDisplayEXT");
    EGLDisplay dpy = gpd ? gpd(EGL_PLATFORM_GBM_KHR, gbm, NULL) : eglGetDisplay((EGLNativeDisplayType)gbm);
    EGLint maj = 0, min = 0;
    if (!eglInitialize(dpy, &maj, &min)) {
        printf("eglInitialize failed 0x%x\n", eglGetError());
        return 1;
    }
    printf("EGL %d.%d vendor %s version %s\n", maj, min, eglQueryString(dpy, EGL_VENDOR), eglQueryString(dpy, EGL_VERSION));
    const char *ext = eglQueryString(dpy, EGL_EXTENSIONS);
    printf("EGL display extensions: %s\n", ext);
    printf("EGL_ANDROID_native_fence_sync: %s\n", strstr(ext, "EGL_ANDROID_native_fence_sync") ? "yes" : "no");
    printf("EGL_KHR_fence_sync: %s\n", strstr(ext, "EGL_KHR_fence_sync") ? "yes" : "no");
    eglBindAPI(EGL_OPENGL_ES_API);
    EGLint attrs[] = { EGL_SURFACE_TYPE, EGL_WINDOW_BIT, EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8,
                       EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE };
    EGLConfig cfgs[64];
    EGLint n = 0;
    eglChooseConfig(dpy, attrs, cfgs, 64, &n);
    EGLConfig cfg = NULL;
    for (int i = 0; i < n; i++) {
        EGLint vid = 0;
        eglGetConfigAttrib(dpy, cfgs[i], EGL_NATIVE_VISUAL_ID, &vid);
        if (vid == GBM_FORMAT_XRGB8888) {
            cfg = cfgs[i];
            break;
        }
    }
    printf("configs %d, XRGB8888 visual %s\n", n, cfg ? "found" : "missing");
    if (!cfg) return 1;

    drmModeResPtr res = drmModeGetResources(fd);
    drmModeConnectorPtr conn = NULL;
    for (int i = 0; res && i < res->count_connectors; i++) {
        drmModeConnectorPtr c = drmModeGetConnector(fd, res->connectors[i]);
        if (c && c->connection == DRM_MODE_CONNECTED && c->count_modes > 0 && c->connector_type != DRM_MODE_CONNECTOR_WRITEBACK) {
            conn = c;
            break;
        }
        if (c) drmModeFreeConnector(c);
    }
    if (!conn) {
        printf("no connector\n");
        return 1;
    }
    drmModeModeInfo mode = conn->modes[0];
    struct gbm_surface *gs = gbm_surface_create(gbm, mode.hdisplay, mode.vdisplay, GBM_FORMAT_XRGB8888,
                                                GBM_BO_USE_SCANOUT | GBM_BO_USE_RENDERING);
    printf("gbm_surface %p\n", (void *)gs);
    if (!gs) return 1;
    EGLint cattrs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
    EGLContext ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, cattrs);
    EGLSurface surf = eglCreateWindowSurface(dpy, cfg, (EGLNativeWindowType)gs, NULL);
    printf("context %p surface %p (0x%x)\n", ctx, surf, eglGetError());
    if (!eglMakeCurrent(dpy, surf, surf, ctx)) {
        printf("makecurrent failed 0x%x\n", eglGetError());
        return 1;
    }
    printf("GL_RENDERER %s GL_VERSION %s\n", glGetString(GL_RENDERER), glGetString(GL_VERSION));
    glClearColor(1, 0, 0, 1);
    glClear(GL_COLOR_BUFFER_BIT);
    printf("swap %d\n", eglSwapBuffers(dpy, surf));
    struct gbm_bo *bo = gbm_surface_lock_front_buffer(gs);
    printf("front bo %p %ux%u stride %u format 0x%x modifier 0x%llx planes %d\n", (void *)bo, gbm_bo_get_width(bo),
           gbm_bo_get_height(bo), gbm_bo_get_stride(bo), gbm_bo_get_format(bo), (unsigned long long)gbm_bo_get_modifier(bo),
           gbm_bo_get_plane_count(bo));
    uint32_t fb = 0;
    int r = drmModeAddFB(fd, gbm_bo_get_width(bo), gbm_bo_get_height(bo), 24, 32, gbm_bo_get_stride(bo),
                         gbm_bo_get_handle(bo).u32, &fb);
    printf("AddFB -> %d fb %u\n", r, fb);
    r = drmSetMaster(fd);
    r = drmModeSetCrtc(fd, res->crtcs[0], fb, 0, 0, &conn->connector_id, 1, &mode);
    printf("SetCrtc -> %d %s\n", r, r ? strerror(errno) : "");
    uint32_t stride = 0;
    void *map_data = NULL;
    uint32_t *p = gbm_bo_map(bo, 0, 0, 4, 4, GBM_BO_TRANSFER_READ, &stride, &map_data);
    printf("bo pixel 0x%08x\n", p ? p[0] : 0);
    if (p) gbm_bo_unmap(bo, map_data);
    return 0;
}
