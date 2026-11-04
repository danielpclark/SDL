// Declarations from the Linux kernel's <linux/kd.h>, <linux/keyboard.h>,
// <linux/vt.h> and <linux/tiocl.h> (uapi), written by hand for the console
// keyboard of this translation of Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The virtual console's keyboard and terminal interfaces: the ioctl
//! requests, structures and keysym encoding `SDL_evdev_kbd.c` uses (`libc`
//! doesn't declare them, but for `TIOCLINUX`). The values are the kernel's
//! ABI (the same on every architecture, but `TIOCLINUX`'s); the comments
//! name the C macros.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_short, c_uint, c_ulong};

// <linux/kd.h>

/// set led state [lights, not flags] (`KDSETLED`)
pub(crate) const KDSETLED: c_ulong = 0x4B32;

/// get keyboard type (`KDGKBTYPE`)
pub(crate) const KDGKBTYPE: c_ulong = 0x4B33;
/// (`KB_84`)
pub(crate) const KB_84: c_char = 0x01;
/// (`KB_101`)
pub(crate) const KB_101: c_char = 0x02;
/// (`KB_OTHER`)
pub(crate) const KB_OTHER: c_char = 0x03;

/// gets current keyboard mode (`KDGKBMODE`)
pub(crate) const KDGKBMODE: c_ulong = 0x4B44;
/// sets current keyboard mode (`KDSKBMODE`)
pub(crate) const KDSKBMODE: c_ulong = 0x4B45;
/// (`K_RAW`)
pub(crate) const K_RAW: c_int = 0x00;
/// (`K_XLATE`)
pub(crate) const K_XLATE: c_int = 0x01;
/// (`K_MEDIUMRAW`)
pub(crate) const K_MEDIUMRAW: c_int = 0x02;
/// (`K_UNICODE`; not defined in older Linux kernel headers)
pub(crate) const K_UNICODE: c_int = 0x03;
/// (`K_OFF`; not defined in older Linux kernel headers)
pub(crate) const K_OFF: c_int = 0x04;

/// `struct kbentry`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct kbentry {
    pub(crate) kb_table: u8,
    pub(crate) kb_index: u8,
    pub(crate) kb_value: u16,
}
/// gets one entry in translation table (`KDGKBENT`)
pub(crate) const KDGKBENT: c_ulong = 0x4B46;

/// `struct kbdiacr`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct kbdiacr {
    pub(crate) diacr: u8,
    pub(crate) base: u8,
    pub(crate) result: u8,
}
/// `struct kbdiacrs`
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct kbdiacrs {
    /// number of entries in following array
    pub(crate) kb_cnt: c_uint,
    /// MAX_DIACR from keyboard.h
    pub(crate) kbdiacr: [kbdiacr; MAX_DIACR],
}
/// read kernel accent table (`KDGKBDIACR`)
pub(crate) const KDGKBDIACR: c_ulong = 0x4B4A;

/// get led flags (not lights) (`KDGKBLED`)
pub(crate) const KDGKBLED: c_ulong = 0x4B64;
/// (`K_SCROLLLOCK`)
pub(crate) const K_SCROLLLOCK: u8 = 0x01;
/// (`K_NUMLOCK`)
pub(crate) const K_NUMLOCK: u8 = 0x02;
/// (`K_CAPSLOCK`)
pub(crate) const K_CAPSLOCK: u8 = 0x04;

// <linux/keyboard.h>

pub(crate) const KG_SHIFT: u8 = 0;
pub(crate) const KG_CTRL: u8 = 2;
pub(crate) const KG_ALT: u8 = 3;
pub(crate) const KG_ALTGR: u8 = 1;
pub(crate) const KG_SHIFTL: u8 = 4;
pub(crate) const KG_KANASHIFT: u8 = 4;
pub(crate) const KG_SHIFTR: u8 = 5;
pub(crate) const KG_CTRLL: u8 = 6;
pub(crate) const KG_CTRLR: u8 = 7;
pub(crate) const KG_CAPSSHIFT: u8 = 8;

pub(crate) const NR_SHIFT: usize = 9;

pub(crate) const NR_KEYS: usize = 256;
pub(crate) const MAX_NR_KEYMAPS: usize = 256;

/// `KTYP(x)`
pub(crate) const fn ktyp(x: u16) -> u8 {
    (x >> 8) as u8
}
/// `KVAL(x)`
pub(crate) const fn kval(x: u16) -> u8 {
    (x & 0xff) as u8
}
/// `K(t, v)`
pub(crate) const fn k(t: u8, v: u8) -> u16 {
    ((t as u16) << 8) | v as u16
}

pub(crate) const KT_LATIN: u8 = 0; // we depend on this being zero
pub(crate) const KT_LETTER: u8 = 11; // symbol that can be acted upon by CapsLock
pub(crate) const KT_FN: u8 = 1;
pub(crate) const KT_SPEC: u8 = 2;
pub(crate) const KT_PAD: u8 = 3;
pub(crate) const KT_DEAD: u8 = 4;
pub(crate) const KT_CONS: u8 = 5;
pub(crate) const KT_CUR: u8 = 6;
pub(crate) const KT_SHIFT: u8 = 7;
pub(crate) const KT_META: u8 = 8;
pub(crate) const KT_ASCII: u8 = 9;
pub(crate) const KT_LOCK: u8 = 10;
pub(crate) const KT_SLOCK: u8 = 12;
pub(crate) const KT_DEAD2: u8 = 13;
pub(crate) const KT_BRL: u8 = 14;

pub(crate) const K_HOLE: u16 = k(KT_SPEC, 0);
pub(crate) const K_ENTER: u16 = k(KT_SPEC, 1);
pub(crate) const K_CAPS: u16 = k(KT_SPEC, 7);
pub(crate) const K_NUM: u16 = k(KT_SPEC, 8);
pub(crate) const K_CAPSON: u16 = k(KT_SPEC, 13);
pub(crate) const K_COMPOSE: u16 = k(KT_SPEC, 14);
pub(crate) const K_BARENUMLOCK: u16 = k(KT_SPEC, 19);

pub(crate) const K_P0: u16 = k(KT_PAD, 0);
pub(crate) const K_PPLUS: u16 = k(KT_PAD, 10); // key-pad plus
pub(crate) const K_PENTER: u16 = k(KT_PAD, 14); // key-pad enter

pub(crate) const K_DGRAVE: u16 = k(KT_DEAD, 0);
pub(crate) const K_DACUTE: u16 = k(KT_DEAD, 1);
pub(crate) const K_DCIRCM: u16 = k(KT_DEAD, 2);
pub(crate) const K_DTILDE: u16 = k(KT_DEAD, 3);
pub(crate) const K_DDIERE: u16 = k(KT_DEAD, 4);
pub(crate) const K_DCEDIL: u16 = k(KT_DEAD, 5);

/// The number of dead keys (`NR_DEAD`): 6 in older kernel headers, 27 since
/// Linux 5.13 added the rest of X11's dead keys.
pub(crate) const NR_DEAD: usize = 27;

pub(crate) const K_SHIFT: u16 = k(KT_SHIFT, KG_SHIFT);
pub(crate) const K_CTRL: u16 = k(KT_SHIFT, KG_CTRL);
pub(crate) const K_ALT: u16 = k(KT_SHIFT, KG_ALT);
pub(crate) const K_ALTGR: u16 = k(KT_SHIFT, KG_ALTGR);
pub(crate) const K_CAPSSHIFT: u16 = k(KT_SHIFT, KG_CAPSSHIFT);

pub(crate) const K_ASC0: u16 = k(KT_ASCII, 0);
pub(crate) const K_HEX0: u16 = k(KT_ASCII, 10);

pub(crate) const K_SHIFTLOCK: u16 = k(KT_LOCK, KG_SHIFT);
pub(crate) const K_SHIFT_SLOCK: u16 = k(KT_SLOCK, KG_SHIFT);
pub(crate) const K_ALTGR_SLOCK: u16 = k(KT_SLOCK, KG_ALTGR);

pub(crate) const MAX_DIACR: usize = 256;

// <linux/vt.h>

/// `struct vt_mode`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct vt_mode {
    /// vt mode
    pub(crate) mode: c_char,
    /// if set, hang on writes if not active
    pub(crate) waitv: c_char,
    /// signal to raise on release req
    pub(crate) relsig: c_short,
    /// signal to raise on acquisition
    pub(crate) acqsig: c_short,
    /// unused (set to 0)
    pub(crate) frsig: c_short,
}
/// get mode of active vt (`VT_GETMODE`)
pub(crate) const VT_GETMODE: c_ulong = 0x5601;
/// set mode of active vt (`VT_SETMODE`)
pub(crate) const VT_SETMODE: c_ulong = 0x5602;
/// auto vt switching (`VT_AUTO`)
pub(crate) const VT_AUTO: c_char = 0x00;
/// process controls switching (`VT_PROCESS`)
pub(crate) const VT_PROCESS: c_char = 0x01;
/// acknowledge switch (`VT_ACKACQ`)
pub(crate) const VT_ACKACQ: c_int = 0x02;

/// release display (`VT_RELDISP`)
pub(crate) const VT_RELDISP: c_ulong = 0x5605;

// <linux/tiocl.h>, and TIOCLINUX of <asm/ioctls.h>

/// (`TIOCLINUX`: 0x541C, but on MIPS)
pub(crate) const TIOCLINUX: c_ulong = libc::TIOCLINUX as c_ulong;
/// write shift state (`TIOCL_GETSHIFTSTATE`)
pub(crate) const TIOCL_GETSHIFTSTATE: c_char = 6;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_match_the_kernel() {
        // (as a C program built with <linux/kd.h> and <linux/vt.h> gives
        // them, on x86-64 and the runner alike)
        assert_eq!((size_of::<kbentry>(), align_of::<kbentry>()), (4, 2));
        assert_eq!(std::mem::offset_of!(kbentry, kb_index), 1);
        assert_eq!(std::mem::offset_of!(kbentry, kb_value), 2);
        assert_eq!((size_of::<kbdiacr>(), align_of::<kbdiacr>()), (3, 1));
        assert_eq!((size_of::<kbdiacrs>(), align_of::<kbdiacrs>()), (772, 4));
        assert_eq!(std::mem::offset_of!(kbdiacrs, kbdiacr), 4);
        assert_eq!((size_of::<vt_mode>(), align_of::<vt_mode>()), (8, 2));
        assert_eq!(std::mem::offset_of!(vt_mode, waitv), 1);
        assert_eq!(std::mem::offset_of!(vt_mode, relsig), 2);
        assert_eq!(std::mem::offset_of!(vt_mode, acqsig), 4);
        assert_eq!(std::mem::offset_of!(vt_mode, frsig), 6);
    }

    #[test]
    fn values_match_the_kernel() {
        // <linux/kd.h>
        assert_eq!(
            [KDSETLED, KDGKBTYPE, KDGKBMODE, KDSKBMODE, KDGKBENT, KDGKBDIACR, KDGKBLED],
            [0x4b32, 0x4b33, 0x4b44, 0x4b45, 0x4b46, 0x4b4a, 0x4b64]
        );
        assert_eq!([KB_84, KB_101, KB_OTHER], [1, 2, 3]);
        assert_eq!(
            [K_RAW, K_XLATE, K_MEDIUMRAW, K_UNICODE, K_OFF],
            [0, 1, 2, 3, 4]
        );
        assert_eq!([K_SCROLLLOCK, K_NUMLOCK, K_CAPSLOCK], [1, 2, 4]);
        // <linux/keyboard.h>
        assert_eq!(
            (NR_KEYS, MAX_NR_KEYMAPS, NR_SHIFT, MAX_DIACR),
            (256, 256, 9, 256)
        );
        assert_eq!(
            [
                KG_SHIFT,
                KG_ALTGR,
                KG_CTRL,
                KG_ALT,
                KG_SHIFTL,
                KG_SHIFTR,
                KG_CTRLL,
                KG_CTRLR,
                KG_CAPSSHIFT
            ],
            [0, 1, 2, 3, 4, 5, 6, 7, 8]
        );
        assert_eq!(
            [
                KT_LATIN, KT_FN, KT_SPEC, KT_PAD, KT_DEAD, KT_CONS, KT_CUR, KT_SHIFT, KT_META,
                KT_ASCII, KT_LOCK, KT_LETTER, KT_SLOCK, KT_DEAD2, KT_BRL
            ],
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]
        );
        assert_eq!(
            [
                K_HOLE,
                K_ENTER,
                K_CAPS,
                K_NUM,
                K_CAPSON,
                K_COMPOSE,
                K_BARENUMLOCK
            ],
            [0x200, 0x201, 0x207, 0x208, 0x20d, 0x20e, 0x213]
        );
        assert_eq!(
            [K_SHIFT, K_CAPSSHIFT, K_DGRAVE, K_DCEDIL],
            [0x700, 0x708, 0x400, 0x405]
        );
        assert_eq!(NR_DEAD, 0x1b);
        assert_eq!(
            (ktyp(0xfb61), kval(0xfb61), k(KT_LATIN, b'a')),
            (0xfb, 0x61, 0x61)
        );
        // <linux/vt.h>
        assert_eq!(
            [VT_GETMODE, VT_SETMODE, VT_RELDISP],
            [0x5601, 0x5602, 0x5605]
        );
        assert_eq!((VT_AUTO, VT_PROCESS, VT_ACKACQ), (0, 1, 2));
        // <linux/tiocl.h>, <asm-generic/ioctls.h>
        assert_eq!(TIOCL_GETSHIFTSTATE, 6);
        #[cfg(not(any(target_arch = "mips", target_arch = "mips64")))]
        assert_eq!(TIOCLINUX, 0x541c);
    }
}
