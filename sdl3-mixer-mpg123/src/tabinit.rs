/*
    tabinit.c: initialize tables...

    copyright ?-2008 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Michael Hipp
*/
// Modified 2026-10-17: translated into Rust from tabinit.c in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

//! The synth window (`decwin`, scaled by the output volume) and the 16 to
//! 8 bit conversion table. The cosine tables (`INT123_pnts`) are the
//! precomputed ones of `costabs.h`, in `costabs.rs`. `log()` is SDL's
//! (fdlibm's), which gives the same table as the C library's.

use sdl3::stdlib::math::log;

use crate::frame::Mpg123Handle;
use crate::mpg123::*;

static INTWINBASE: [i64; 257] = [
    0, -1, -1, -1, -1, -1, -1, -2, -2, -2, -2, -3, -3, -4, -4, -5, -5, -6, -7, -7, -8, -9, -10,
    -11, -13, -14, -16, -17, -19, -21, -24, -26, -29, -31, -35, -38, -41, -45, -49, -53, -58, -63,
    -68, -73, -79, -85, -91, -97, -104, -111, -117, -125, -132, -139, -147, -154, -161, -169, -176,
    -183, -190, -196, -202, -208, -213, -218, -222, -225, -227, -228, -228, -227, -224, -221, -215,
    -208, -200, -189, -177, -163, -146, -127, -106, -83, -57, -29, 2, 36, 72, 111, 153, 197, 244,
    294, 347, 401, 459, 519, 581, 645, 711, 779, 848, 919, 991, 1064, 1137, 1210, 1283, 1356, 1428,
    1498, 1567, 1634, 1698, 1759, 1817, 1870, 1919, 1962, 2001, 2032, 2057, 2075, 2085, 2087, 2080,
    2063, 2037, 2000, 1952, 1893, 1822, 1739, 1644, 1535, 1414, 1280, 1131, 970, 794, 605, 402,
    185, -45, -288, -545, -814, -1095, -1388, -1692, -2006, -2330, -2663, -3004, -3351, -3705,
    -4063, -4425, -4788, -5153, -5517, -5879, -6237, -6589, -6935, -7271, -7597, -7910, -8209,
    -8491, -8755, -8998, -9219, -9416, -9585, -9727, -9838, -9916, -9959, -9966, -9935, -9863,
    -9750, -9592, -9389, -9139, -8840, -8492, -8092, -7640, -7134, -6574, -5959, -5288, -4561,
    -3776, -2935, -2037, -1082, -70, 998, 2122, 3300, 4533, 5818, 7154, 8540, 9975, 11455, 12980,
    14548, 16155, 17799, 19478, 21189, 22929, 24694, 26482, 28289, 30112, 31947, 33791, 35640,
    37489, 39336, 41176, 43006, 44821, 46617, 48390, 50137, 51853, 53534, 55178, 56778, 58333,
    59838, 61289, 62684, 64019, 65290, 66494, 67629, 68692, 69679, 70590, 71420, 72169, 72835,
    73415, 73908, 74313, 74630, 74856, 74992, 75038,
];

/// Translation of `INT123_make_decode_tables()`.
pub(crate) fn make_decode_tables(fr: &mut Mpg123Handle<'_>) {
    let mut idx: i32 = 0;
    /* Scale is always based on 1.0 . */
    let mut scaleval = -0.5
        * (if fr.lastscale < 0.0 {
            fr.p.outscale
        } else {
            fr.lastscale
        });
    let Some(decwin) = fr.decwin.as_deref_mut() else {
        return;
    };
    let mut i = 0;
    let mut j: i32 = 0;
    while i < 256 {
        if idx < 512 + 16 {
            let v = (INTWINBASE[j as usize] as f64 * scaleval) as f32;
            decwin[(idx + 16) as usize] = v;
            decwin[idx as usize] = v;
        }

        if i % 32 == 31 {
            idx -= 1023;
        }
        if i % 64 == 63 {
            scaleval = -scaleval;
        }
        i += 1;
        j += 1;
        idx += 32;
    }

    /* i=256 */
    while i < 512 {
        if idx < 512 + 16 {
            let v = (INTWINBASE[j as usize] as f64 * scaleval) as f32;
            decwin[(idx + 16) as usize] = v;
            decwin[idx as usize] = v;
        }

        if i % 32 == 31 {
            idx -= 1023;
        }
        if i % 64 == 63 {
            scaleval = -scaleval;
        }
        i += 1;
        j -= 1;
        idx += 32;
    }
}

/// Translation of `INT123_make_conv16to8_table()`. `fr->conv16to8` is
/// `conv16to8_buf + 4096`.
pub(crate) fn make_conv16to8_table(fr: &mut Mpg123Handle<'_>) -> i32 {
    let mode = fr.af.dec_enc;

    /*
     * ????: 8.0 is right but on SB cards '2.0' is a better value ???
     */
    let mul = 8.0;

    if fr.conv16to8_buf.is_none() {
        fr.conv16to8_buf = Some(Box::new([0; 8192]));
    }
    let noquiet = fr.noquiet();
    let _ = noquiet;
    let buf = fr.conv16to8_buf.as_deref_mut().unwrap();
    let conv = |i: i32| (i + 4096) as usize;

    match mode {
        MPG123_ENC_ULAW_8 => {
            let m = 127.0 / log(256.0);

            for i in -4096..4096 {
                /* dunno whether this is a valid transformation rule ?!?!? */
                let mut c1 = if i < 0 {
                    127 - (log(1.0 - 255.0 * i as f64 * mul / 32768.0) * m) as i32
                } else {
                    255 - (log(1.0 + 255.0 * i as f64 * mul / 32768.0) * m) as i32
                };
                if !(0..=255).contains(&c1) {
                    // Converror %d %d
                    return -1;
                }
                if c1 == 0 {
                    c1 = 2;
                }
                buf[conv(i)] = c1 as u8;
            }
        }
        MPG123_ENC_SIGNED_8 => {
            for i in -4096..4096 {
                buf[conv(i)] = (i >> 5) as u8;
            }
        }
        MPG123_ENC_UNSIGNED_8 => {
            for i in -4096..4096 {
                buf[conv(i)] = ((i >> 5) + 128) as u8;
            }
        }
        MPG123_ENC_ALAW_8 => {
            /*
                Let's believe Wikipedia (http://en.wikipedia.org/wiki/G.711) that this
                is the correct table:

                s0000000wxyza... 	n000wxyz  [0-31] -> [0-15]
                s0000001wxyza... 	n001wxyz  [32-63] -> [16-31]
                s000001wxyzab... 	n010wxyz  [64-127] -> [32-47]
                s00001wxyzabc... 	n011wxyz  [128-255] -> [48-63]
                s0001wxyzabcd... 	n100wxyz  [256-511] -> [64-79]
                s001wxyzabcde... 	n101wxyz  [512-1023] -> [80-95]
                s01wxyzabcdef... 	n110wxyz  [1024-2047] -> [96-111]
                s1wxyzabcdefg... 	n111wxyz  [2048-4095] -> [112-127]

                Let's extend to -4096, too.
                Also, bytes are xored with 0x55 for transmission.

                Since it sounds OK, I assume it is fine;-)
            */
            for i in 0..64u32 {
                buf[conv(i as i32)] = (i >> 1) as u8;
            }
            for i in 64..128u32 {
                buf[conv(i as i32)] = (((i >> 2) & 0xf) | (2 << 4)) as u8;
            }
            for i in 128..256u32 {
                buf[conv(i as i32)] = (((i >> 3) & 0xf) | (3 << 4)) as u8;
            }
            for i in 256..512u32 {
                buf[conv(i as i32)] = (((i >> 4) & 0xf) | (4 << 4)) as u8;
            }
            for i in 512..1024u32 {
                buf[conv(i as i32)] = (((i >> 5) & 0xf) | (5 << 4)) as u8;
            }
            for i in 1024..2048u32 {
                buf[conv(i as i32)] = (((i >> 6) & 0xf) | (6 << 4)) as u8;
            }
            for i in 2048..4096u32 {
                buf[conv(i as i32)] = (((i >> 7) & 0xf) | (7 << 4)) as u8;
            }

            for i in -4095..0 {
                buf[conv(i)] = buf[conv(-i)] | 0x80;
            }

            buf[conv(-4096)] = buf[conv(-4095)];

            for i in -4096..4096 {
                /* fr->conv16to8[i] = - i>>5; */
                buf[conv(i)] ^= 0x55;
            }
        }
        _ => {
            fr.err = MPG123_ERR_16TO8TABLE;
            // Unknown 8 bit encoding choice.
            return -1;
        }
    }

    0
}
