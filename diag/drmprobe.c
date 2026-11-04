/* diag only: what a DRM card offers, the master probe upstream does, a dumb
 * buffer on the CRTC read back through GETFB2 + MAP_DUMB, and the debugfs CRC
 * of a few frames. Usage: drmprobe /dev/dri/cardN DEBUGFS_DIR */
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
#include <xf86drm.h>
#include <xf86drmMode.h>
#include <drm_fourcc.h>

static void props(int fd, uint32_t id, uint32_t type, const char *what)
{
    drmModeObjectPropertiesPtr p = drmModeObjectGetProperties(fd, id, type);
    if (!p) {
        printf("  %s %u: no properties (%s)\n", what, id, strerror(errno));
        return;
    }
    printf("  %s %u props:", what, id);
    for (uint32_t i = 0; i < p->count_props; i++) {
        drmModePropertyPtr pr = drmModeGetProperty(fd, p->props[i]);
        if (pr) {
            printf(" %s=%llu", pr->name, (unsigned long long)p->prop_values[i]);
            drmModeFreeProperty(pr);
        }
    }
    printf("\n");
    drmModeFreeObjectProperties(p);
}

static void crc(const char *dir, int which)
{
    char path[512], buf[4096];
    snprintf(path, sizeof(path), "%s/crtc-0/crc/control", dir);
    int c = open(path, O_WRONLY);
    if (c < 0) {
        printf("crc: open %s: %s\n", path, strerror(errno));
        return;
    }
    if (write(c, "auto", 4) < 0) {
        printf("crc: write auto: %s\n", strerror(errno));
    }
    close(c);
    snprintf(path, sizeof(path), "%s/crtc-0/crc/data", dir);
    int d = open(path, O_RDONLY);
    if (d < 0) {
        printf("crc: open %s: %s\n", path, strerror(errno));
        return;
    }
    for (int n = 0; n < 4; n++) {
        struct pollfd pfd = { d, POLLIN, 0 };
        int r = poll(&pfd, 1, 2000);
        if (r <= 0) {
            printf("crc[%d]: poll %d %s\n", which, r, r < 0 ? strerror(errno) : "timeout");
            break;
        }
        ssize_t len = read(d, buf, sizeof(buf) - 1);
        if (len <= 0) {
            printf("crc[%d]: read %zd %s\n", which, len, strerror(errno));
            break;
        }
        buf[len] = 0;
        printf("crc[%d]: %s", which, buf);
    }
    close(d);
}

int main(int argc, char **argv)
{
    const char *card = argv[1];
    int fd = open(card, O_RDWR | O_CLOEXEC);
    if (fd < 0) {
        printf("open %s: %s\n", card, strerror(errno));
        return 1;
    }
    drmVersionPtr v = drmGetVersion(fd);
    if (v) {
        printf("driver %s %d.%d.%d (%s)\n", v->name, v->version_major, v->version_minor, v->version_patchlevel, v->desc);
        drmFreeVersion(v);
    }
    static const struct { const char *n; uint64_t c; } caps[] = {
        { "DUMB_BUFFER", DRM_CAP_DUMB_BUFFER }, { "PRIME", DRM_CAP_PRIME },
        { "CURSOR_WIDTH", DRM_CAP_CURSOR_WIDTH }, { "CURSOR_HEIGHT", DRM_CAP_CURSOR_HEIGHT },
        { "ADDFB2_MODIFIERS", DRM_CAP_ADDFB2_MODIFIERS }, { "ASYNC_PAGE_FLIP", DRM_CAP_ASYNC_PAGE_FLIP },
        { "CRTC_IN_VBLANK_EVENT", DRM_CAP_CRTC_IN_VBLANK_EVENT }, { "TIMESTAMP_MONOTONIC", DRM_CAP_TIMESTAMP_MONOTONIC },
    };
    for (size_t i = 0; i < sizeof(caps) / sizeof(caps[0]); i++) {
        uint64_t val = 0;
        int r = drmGetCap(fd, caps[i].c, &val);
        printf("cap %s = %llu (r=%d)\n", caps[i].n, (unsigned long long)val, r);
    }
    printf("client cap UNIVERSAL_PLANES r=%d\n", drmSetClientCap(fd, DRM_CLIENT_CAP_UNIVERSAL_PLANES, 1));
    printf("client cap ATOMIC r=%d\n", drmSetClientCap(fd, DRM_CLIENT_CAP_ATOMIC, 1));
    printf("client cap WRITEBACK r=%d\n", drmSetClientCap(fd, DRM_CLIENT_CAP_WRITEBACK_CONNECTORS, 1));

    /* upstream's master probe (KMSDRM_Available / check_modesetting) */
    int sm = drmSetMaster(fd);
    int am = drmAuthMagic(fd, 0);
    printf("drmSetMaster=%d drmAuthMagic(fd,0)=%d (-EACCES=%d: %s)\n", sm, am, -EACCES, am == -EACCES ? "not master" : "master");

    drmModeResPtr res = drmModeGetResources(fd);
    if (!res) {
        printf("no resources: %s\n", strerror(errno));
        return 1;
    }
    printf("res: fbs %d crtcs %d connectors %d encoders %d, %ux%u..%ux%u\n", res->count_fbs, res->count_crtcs,
           res->count_connectors, res->count_encoders, res->min_width, res->min_height, res->max_width, res->max_height);
    drmModeConnectorPtr conn = NULL;
    for (int i = 0; i < res->count_connectors; i++) {
        drmModeConnectorPtr c = drmModeGetConnector(fd, res->connectors[i]);
        if (!c) continue;
        printf("connector %u type %u (%s) id %u connection %d modes %d encoders %d mm %ux%u\n", c->connector_id,
               c->connector_type, drmModeGetConnectorTypeName(c->connector_type), c->connector_type_id, c->connection,
               c->count_modes, c->count_encoders, c->mmWidth, c->mmHeight);
        for (int m = 0; m < c->count_modes && m < 4; m++) {
            printf("  mode %s %ux%u@%u type 0x%x\n", c->modes[m].name, c->modes[m].hdisplay, c->modes[m].vdisplay,
                   c->modes[m].vrefresh, c->modes[m].type);
        }
        props(fd, c->connector_id, DRM_MODE_OBJECT_CONNECTOR, "connector");
        if (!conn && c->connection == DRM_MODE_CONNECTED && c->count_modes > 0 && c->connector_type != DRM_MODE_CONNECTOR_WRITEBACK) {
            conn = c;
        } else {
            drmModeFreeConnector(c);
        }
    }
    for (int i = 0; i < res->count_crtcs; i++) {
        props(fd, res->crtcs[i], DRM_MODE_OBJECT_CRTC, "crtc");
    }
    drmModePlaneResPtr pr = drmModeGetPlaneResources(fd);
    if (pr) {
        for (uint32_t i = 0; i < pr->count_planes; i++) {
            drmModePlanePtr p = drmModeGetPlane(fd, pr->planes[i]);
            if (p) {
                printf("plane %u formats %u possible_crtcs 0x%x\n", p->plane_id, p->count_formats, p->possible_crtcs);
                drmModeFreePlane(p);
            }
            props(fd, pr->planes[i], DRM_MODE_OBJECT_PLANE, "plane");
        }
        drmModeFreePlaneResources(pr);
    }
    if (!conn) {
        printf("no connected connector\n");
        return 1;
    }
    drmModeModeInfo mode = conn->modes[0];
    uint32_t crtc_id = res->crtcs[0];

    /* a dumb buffer on the CRTC (legacy SetCrtc) */
    struct drm_mode_create_dumb cd = { .width = mode.hdisplay, .height = mode.vdisplay, .bpp = 32 };
    if (drmIoctl(fd, DRM_IOCTL_MODE_CREATE_DUMB, &cd) < 0) {
        printf("create dumb: %s\n", strerror(errno));
        return 1;
    }
    struct drm_mode_map_dumb md = { .handle = cd.handle };
    drmIoctl(fd, DRM_IOCTL_MODE_MAP_DUMB, &md);
    uint32_t *px = mmap(NULL, cd.size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, md.offset);
    if (px == MAP_FAILED) {
        printf("mmap: %s\n", strerror(errno));
        return 1;
    }
    for (uint64_t i = 0; i < cd.size / 4; i++) px[i] = 0x00ff8000;
    uint32_t handles[4] = { cd.handle }, pitches[4] = { cd.pitch }, offsets[4] = { 0 }, fb = 0;
    int r = drmModeAddFB2(fd, cd.width, cd.height, DRM_FORMAT_XRGB8888, handles, pitches, offsets, &fb, 0);
    printf("AddFB2 %ux%u pitch %u -> %d fb %u\n", cd.width, cd.height, cd.pitch, r, fb);
    r = drmModeSetCrtc(fd, crtc_id, fb, 0, 0, &conn->connector_id, 1, &mode);
    printf("SetCrtc crtc %u connector %u mode %s -> %d %s\n", crtc_id, conn->connector_id, mode.name, r, r ? strerror(errno) : "");

    /* read it back: GETFB2 on the CRTC's framebuffer, then MAP_DUMB of its handle */
    drmModeCrtcPtr crtc = drmModeGetCrtc(fd, crtc_id);
    if (crtc) {
        drmModeFB2Ptr f2 = drmModeGetFB2(fd, crtc->buffer_id);
        if (f2) {
            printf("GETFB2 fb %u %ux%u fmt 0x%x mod 0x%llx handle %u pitch %u\n", f2->fb_id, f2->width, f2->height,
                   f2->pixel_format, (unsigned long long)f2->modifier, f2->handles[0], f2->pitches[0]);
            struct drm_mode_map_dumb m2 = { .handle = f2->handles[0] };
            int mr = drmIoctl(fd, DRM_IOCTL_MODE_MAP_DUMB, &m2);
            uint32_t *p2 = mr == 0 ? mmap(NULL, (size_t)f2->pitches[0] * f2->height, PROT_READ, MAP_SHARED, fd, m2.offset) : MAP_FAILED;
            if (p2 != MAP_FAILED) {
                printf("readback pixel[0]=0x%08x pixel[last]=0x%08x\n", p2[0], p2[(f2->pitches[0] / 4) * f2->height - 1]);
                munmap(p2, (size_t)f2->pitches[0] * f2->height);
            } else {
                printf("readback map: %d %s\n", mr, strerror(errno));
            }
            drmModeFreeFB2(f2);
        } else {
            printf("GETFB2: %s\n", strerror(errno));
        }
        drmModeFreeCrtc(crtc);
    }
    if (argc > 2) {
        crc(argv[2], 0);
        for (uint64_t i = 0; i < cd.size / 4; i++) px[i] = 0x000000ff;
        /* (the dumb buffer is scanned out as it is: vkms composes each vblank) */
        crc(argv[2], 1);
    }

    /* a page flip with an event, as KMSDRM_WaitPageflip waits for */
    r = drmModePageFlip(fd, crtc_id, fb, DRM_MODE_PAGE_FLIP_EVENT, NULL);
    printf("PageFlip -> %d %s\n", r, r ? strerror(errno) : "");
    if (r == 0) {
        struct pollfd pfd = { fd, POLLIN, 0 };
        int pr2 = poll(&pfd, 1, 2000);
        drmEventContext ev = { 0 };
        ev.version = DRM_EVENT_CONTEXT_VERSION;
        printf("flip event poll %d, handle %d\n", pr2, pr2 > 0 ? drmHandleEvent(fd, &ev) : -1);
    }

    /* an atomic test-only commit of the same state */
    drmModeAtomicReqPtr req = drmModeAtomicAlloc();
    printf("atomic alloc %s\n", req ? "ok" : "failed");
    if (req) drmModeAtomicFree(req);
    drmDropMaster(fd);
    return 0;
}
