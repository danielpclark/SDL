/* diag only: sizes, offsets and constants for PR 3's sys.rs, kd.rs and the runner */
#include <stdio.h>
#include <stddef.h>
#include <stdint.h>
#include <xf86drm.h>
#include <xf86drmMode.h>
#include <gbm.h>
#include <drm_fourcc.h>
#include <linux/kd.h>
#include <linux/vt.h>
#include <linux/keyboard.h>
#include <linux/tiocl.h>
#include <linux/input.h>
#include <linux/uinput.h>
#include <sys/ioctl.h>
#include <termios.h>

#define S(t) printf("size %s %zu align %zu\n", #t, sizeof(t), _Alignof(t))
#define O(t, f) printf("off %s.%s %zu\n", #t, #f, offsetof(t, f))
#define C(c) printf("const %s 0x%llx\n", #c, (unsigned long long)(c))

int main(void)
{
    S(drmModeRes); O(drmModeRes, count_fbs); O(drmModeRes, fbs); O(drmModeRes, count_crtcs); O(drmModeRes, crtcs);
    O(drmModeRes, count_connectors); O(drmModeRes, connectors); O(drmModeRes, count_encoders); O(drmModeRes, encoders);
    O(drmModeRes, min_width); O(drmModeRes, max_width); O(drmModeRes, min_height); O(drmModeRes, max_height);
    S(drmModeModeInfo); O(drmModeModeInfo, clock); O(drmModeModeInfo, hdisplay); O(drmModeModeInfo, vscan);
    O(drmModeModeInfo, vrefresh); O(drmModeModeInfo, flags); O(drmModeModeInfo, type); O(drmModeModeInfo, name);
    S(drmModeFB); O(drmModeFB, fb_id); O(drmModeFB, width); O(drmModeFB, height); O(drmModeFB, pitch); O(drmModeFB, bpp); O(drmModeFB, depth); O(drmModeFB, handle);
    S(drmModeFB2); O(drmModeFB2, fb_id); O(drmModeFB2, pixel_format); O(drmModeFB2, modifier); O(drmModeFB2, flags); O(drmModeFB2, handles); O(drmModeFB2, pitches); O(drmModeFB2, offsets);
    S(drmModePropertyBlobRes); O(drmModePropertyBlobRes, id); O(drmModePropertyBlobRes, length); O(drmModePropertyBlobRes, data);
    S(drmModePropertyRes); O(drmModePropertyRes, prop_id); O(drmModePropertyRes, flags); O(drmModePropertyRes, name);
    O(drmModePropertyRes, count_values); O(drmModePropertyRes, values); O(drmModePropertyRes, count_enums); O(drmModePropertyRes, enums);
    O(drmModePropertyRes, count_blobs); O(drmModePropertyRes, blob_ids);
    S(struct drm_mode_property_enum); O(struct drm_mode_property_enum, value); O(struct drm_mode_property_enum, name);
    S(drmModeCrtc); O(drmModeCrtc, crtc_id); O(drmModeCrtc, buffer_id); O(drmModeCrtc, x); O(drmModeCrtc, y); O(drmModeCrtc, width);
    O(drmModeCrtc, height); O(drmModeCrtc, mode_valid); O(drmModeCrtc, mode); O(drmModeCrtc, gamma_size);
    S(drmModeEncoder); O(drmModeEncoder, encoder_id); O(drmModeEncoder, encoder_type); O(drmModeEncoder, crtc_id); O(drmModeEncoder, possible_crtcs); O(drmModeEncoder, possible_clones);
    S(drmModeConnector); O(drmModeConnector, connector_id); O(drmModeConnector, encoder_id); O(drmModeConnector, connector_type);
    O(drmModeConnector, connector_type_id); O(drmModeConnector, connection); O(drmModeConnector, mmWidth); O(drmModeConnector, mmHeight);
    O(drmModeConnector, subpixel); O(drmModeConnector, count_modes); O(drmModeConnector, modes); O(drmModeConnector, count_props);
    O(drmModeConnector, props); O(drmModeConnector, prop_values); O(drmModeConnector, count_encoders); O(drmModeConnector, encoders);
    S(drmModeObjectProperties); O(drmModeObjectProperties, count_props); O(drmModeObjectProperties, props); O(drmModeObjectProperties, prop_values);
    S(drmModePlane); O(drmModePlane, count_formats); O(drmModePlane, formats); O(drmModePlane, plane_id); O(drmModePlane, crtc_id);
    O(drmModePlane, fb_id); O(drmModePlane, crtc_x); O(drmModePlane, crtc_y); O(drmModePlane, x); O(drmModePlane, y);
    O(drmModePlane, possible_crtcs); O(drmModePlane, gamma_size);
    S(drmModePlaneRes); O(drmModePlaneRes, count_planes); O(drmModePlaneRes, planes);
    S(drmEventContext); O(drmEventContext, version); O(drmEventContext, vblank_handler); O(drmEventContext, page_flip_handler);
    O(drmEventContext, page_flip_handler2); O(drmEventContext, sequence_handler);
    S(drmVersion); O(drmVersion, version_major); O(drmVersion, version_minor); O(drmVersion, version_patchlevel);
    O(drmVersion, name_len); O(drmVersion, name); O(drmVersion, date_len); O(drmVersion, date); O(drmVersion, desc_len); O(drmVersion, desc);
    S(union gbm_bo_handle); S(drm_magic_t);
    S(struct drm_mode_create_dumb); O(struct drm_mode_create_dumb, handle); O(struct drm_mode_create_dumb, pitch); O(struct drm_mode_create_dumb, size);
    S(struct drm_mode_map_dumb); O(struct drm_mode_map_dumb, offset);
    S(struct drm_mode_destroy_dumb);
    S(struct drm_event); S(struct drm_event_vblank); O(struct drm_event_vblank, tv_sec); O(struct drm_event_vblank, sequence); O(struct drm_event_vblank, crtc_id);
    C(DRM_EVENT_CONTEXT_VERSION); C(DRM_IOCTL_MODE_CREATE_DUMB); C(DRM_IOCTL_MODE_MAP_DUMB); C(DRM_IOCTL_MODE_DESTROY_DUMB);
    C(DRM_IOCTL_MODE_GETFB2); C(DRM_IOCTL_SET_MASTER); C(DRM_IOCTL_DROP_MASTER); C(DRM_IOCTL_AUTH_MAGIC);
    C(DRM_CAP_DUMB_BUFFER); C(DRM_CAP_CURSOR_WIDTH); C(DRM_CAP_CURSOR_HEIGHT); C(DRM_CAP_ADDFB2_MODIFIERS); C(DRM_CAP_PRIME);
    C(DRM_CLIENT_CAP_UNIVERSAL_PLANES); C(DRM_CLIENT_CAP_ATOMIC); C(DRM_CLIENT_CAP_WRITEBACK_CONNECTORS);
    C(DRM_MODE_PAGE_FLIP_EVENT); C(DRM_MODE_ATOMIC_TEST_ONLY); C(DRM_MODE_ATOMIC_NONBLOCK); C(DRM_MODE_ATOMIC_ALLOW_MODESET);
    C(DRM_MODE_OBJECT_CRTC); C(DRM_MODE_OBJECT_CONNECTOR); C(DRM_MODE_OBJECT_PLANE); C(DRM_MODE_FB_MODIFIERS);
    C(DRM_MODE_CURSOR_BO); C(DRM_MODE_CURSOR_MOVE); C(DRM_PLANE_TYPE_OVERLAY); C(DRM_PLANE_TYPE_PRIMARY); C(DRM_PLANE_TYPE_CURSOR);
    C(DRM_MODE_TYPE_PREFERRED); C(DRM_MODE_FLAG_INTERLACE); C(DRM_MODE_CONNECTOR_VIRTUAL); C(DRM_MODE_CONNECTOR_WRITEBACK);
    C(DRM_MODE_PROP_RANGE); C(DRM_MODE_PROP_ENUM); C(DRM_MODE_PROP_BLOB); C(DRM_MODE_PROP_BITMASK); C(DRM_MODE_PROP_OBJECT); C(DRM_MODE_PROP_SIGNED_RANGE);
    C(DRM_FORMAT_XRGB8888); C(DRM_FORMAT_ARGB8888); C(DRM_FORMAT_RGB565); C(DRM_FORMAT_MOD_INVALID); C(DRM_FORMAT_MOD_LINEAR);
    C(GBM_FORMAT_XRGB8888); C(GBM_FORMAT_ARGB8888); C(GBM_BO_USE_SCANOUT); C(GBM_BO_USE_CURSOR); C(GBM_BO_USE_RENDERING); C(GBM_BO_USE_WRITE); C(GBM_BO_USE_LINEAR);
    /* kd.h / vt.h / keyboard.h / tiocl.h */
    S(struct kbentry); O(struct kbentry, kb_index); O(struct kbentry, kb_value);
    S(struct kbsentry); S(struct kbdiacr); S(struct kbdiacrs); O(struct kbdiacrs, kbdiacr);
    S(struct kbdiacruc); S(struct kbdiacrsuc); O(struct kbdiacrsuc, kbdiacruc);
    S(struct kbkeycode); S(struct kbd_repeat);
    S(struct vt_mode); O(struct vt_mode, waitv); O(struct vt_mode, relsig); O(struct vt_mode, acqsig); O(struct vt_mode, frsig);
    S(struct vt_stat); O(struct vt_stat, v_signal); O(struct vt_stat, v_state);
    C(KDGKBTYPE); C(KB_84); C(KB_101); C(KB_OTHER); C(KDGETLED); C(KDSETLED); C(LED_SCR); C(LED_NUM); C(LED_CAP);
    C(KDGKBLED); C(KDSKBLED); C(K_SCROLLLOCK); C(K_NUMLOCK); C(K_CAPSLOCK);
    C(KDGKBMODE); C(KDSKBMODE); C(K_RAW); C(K_XLATE); C(K_MEDIUMRAW); C(K_UNICODE); C(K_OFF);
    C(KDGKBENT); C(KDSKBENT); C(KDGKBSENT); C(KDGKBDIACR); C(KDGKBDIACRUC); C(KDSETMODE); C(KDGETMODE); C(KD_TEXT); C(KD_GRAPHICS);
    C(VT_OPENQRY); C(VT_GETMODE); C(VT_SETMODE); C(VT_AUTO); C(VT_PROCESS); C(VT_ACKACQ); C(VT_GETSTATE); C(VT_RELDISP);
    C(VT_ACTIVATE); C(VT_WAITACTIVE); C(VT_DISALLOCATE);
    C(TIOCLINUX); C(TIOCL_GETSHIFTSTATE); C(TIOCSCTTY); C(TIOCNOTTY);
    C(NR_KEYS); C(MAX_NR_KEYMAPS); C(MAX_NR_FUNC); C(KG_SHIFT); C(KG_CTRL); C(KG_ALT); C(KG_ALTGR); C(KG_SHIFTL); C(KG_SHIFTR);
    C(KG_CTRLL); C(KG_CTRLR); C(KG_CAPSSHIFT); C(NR_SHIFT); C(NR_LOCK); C(KT_LATIN); C(KT_FN); C(KT_SPEC); C(KT_PAD);
    C(KT_DEAD); C(KT_CONS); C(KT_CUR); C(KT_SHIFT); C(KT_META); C(KT_ASCII); C(KT_LOCK); C(KT_LETTER); C(KT_SLOCK); C(KT_DEAD2); C(KT_BRL);
    C(K_SHIFT); C(K_CAPSSHIFT); C(K_ENTER); C(K_CAPS); C(K_NUM); C(K_COMPOSE); C(K_CAPSON); C(K_BARENUMLOCK); C(NR_DEAD); C(K_HOLE); C(K_NOSUCHMAP);
    C(K(KT_LATIN, 'a')); C(K_DGRAVE); C(K_DCEDIL);
    /* input / uinput */
    S(struct input_event); S(struct input_absinfo); S(struct uinput_setup); O(struct uinput_setup, name); O(struct uinput_setup, ff_effects_max);
    S(struct uinput_abs_setup); O(struct uinput_abs_setup, absinfo);
    C(UI_DEV_CREATE); C(UI_DEV_DESTROY); C(UI_DEV_SETUP); C(UI_ABS_SETUP); C(UI_SET_EVBIT); C(UI_SET_KEYBIT); C(UI_SET_RELBIT);
    C(UI_SET_ABSBIT); C(UI_SET_PROPBIT); C(UI_GET_SYSNAME(64)); C(EVIOCGMTSLOTS(4 + 4 * 10)); C(EVIOCGRAB);
    C(ABS_MT_SLOT); C(ABS_MT_TOUCH_MAJOR); C(ABS_MT_POSITION_X); C(ABS_MT_POSITION_Y); C(ABS_MT_TRACKING_ID); C(ABS_MT_PRESSURE);
    C(REL_WHEEL); C(REL_HWHEEL); C(REL_WHEEL_HI_RES); C(REL_HWHEEL_HI_RES); C(BTN_LEFT); C(BTN_TASK); C(INPUT_PROP_DIRECT); C(INPUT_PROP_POINTER);
    return 0;
}
