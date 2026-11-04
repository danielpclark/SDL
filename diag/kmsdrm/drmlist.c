/* diag only: a card's CRTCs, encoders, connectors (status, modes) and planes. */
#include <xf86drm.h>
#include <xf86drmMode.h>
#include <fcntl.h>
#include <stdio.h>
#include <unistd.h>

int main(int argc, char **argv)
{
    int fd = open(argv[1], O_RDWR | O_CLOEXEC);
    if (fd < 0) {
        perror(argv[1]);
        return 1;
    }
    drmVersion *v = drmGetVersion(fd);
    printf("drmlist: %s: driver %s, master %d\n", argv[1], v ? v->name : "?", drmSetMaster(fd));
    drmSetClientCap(fd, DRM_CLIENT_CAP_UNIVERSAL_PLANES, 1);
    drmModeRes *res = drmModeGetResources(fd);
    if (!res) {
        printf("drmlist: no resources\n");
        return 1;
    }
    printf("drmlist: %d crtcs, %d encoders, %d connectors, min %dx%d max %dx%d\n", res->count_crtcs,
           res->count_encoders, res->count_connectors, res->min_width, res->min_height, res->max_width, res->max_height);
    for (int i = 0; i < res->count_crtcs; i++) {
        drmModeCrtc *c = drmModeGetCrtc(fd, res->crtcs[i]);
        printf("drmlist: crtc %u: %s %dx%d fb %u\n", res->crtcs[i], c && c->mode_valid ? c->mode.name : "(off)",
               c ? (int)c->width : 0, c ? (int)c->height : 0, c ? c->buffer_id : 0);
        drmModeFreeCrtc(c);
    }
    for (int i = 0; i < res->count_encoders; i++) {
        drmModeEncoder *e = drmModeGetEncoder(fd, res->encoders[i]);
        printf("drmlist: encoder %u: type %u crtc %u possible_crtcs 0x%x\n", res->encoders[i], e ? e->encoder_type : 0,
               e ? e->crtc_id : 0, e ? e->possible_crtcs : 0);
        drmModeFreeEncoder(e);
    }
    for (int i = 0; i < res->count_connectors; i++) {
        drmModeConnector *c = drmModeGetConnector(fd, res->connectors[i]);
        if (!c) {
            continue;
        }
        printf("drmlist: connector %u: type %u-%u status %d encoder %u encoders %d modes %d", c->connector_id,
               c->connector_type, c->connector_type_id, c->connection, c->encoder_id, c->count_encoders, c->count_modes);
        for (int m = 0; m < c->count_modes && m < 3; m++) {
            printf(" %s@%u%s", c->modes[m].name, c->modes[m].vrefresh, c->modes[m].type & DRM_MODE_TYPE_PREFERRED ? "*" : "");
        }
        printf("\n");
        drmModeFreeConnector(c);
    }
    drmModePlaneRes *pr = drmModeGetPlaneResources(fd);
    for (unsigned i = 0; pr && i < pr->count_planes; i++) {
        drmModePlane *p = drmModeGetPlane(fd, pr->planes[i]);
        printf("drmlist: plane %u: possible_crtcs 0x%x fb %u formats %u\n", pr->planes[i], p ? p->possible_crtcs : 0,
               p ? p->fb_id : 0, p ? p->count_formats : 0);
        drmModeFreePlane(p);
    }
    return 0;
}
