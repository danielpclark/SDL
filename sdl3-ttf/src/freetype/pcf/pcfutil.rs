// Rust translation of src/pcf/pcfutil.c and src/pcf/pcfutil.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
//
// Copyright 1990, 1994, 1998  The Open Group
//
// Permission to use, copy, modify, distribute, and sell this software and its
// documentation for any purpose is hereby granted without fee, provided that
// the above copyright notice appear in all copies and that both that
// copyright notice and this permission notice appear in supporting
// documentation.
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.  IN NO EVENT SHALL THE
// OPEN GROUP BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN
// AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
// CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
//
// Except as contained in this notice, the name of The Open Group shall not be
// used in advertising or otherwise to promote the sale, use or other dealings
// in this Software without prior written authorization from The Open Group.
//
// $XFree86: xc/lib/font/util/utilbitmap.c,v 1.3 1999/08/22 08:58:58 dawes Exp $
//
// Author:  Keith Packard, MIT X Consortium
//
// Modified for use with FreeType. This is an altered (translated) version
// of the original software.

//! Bitmap byte and bit order utilities.

/// `BitOrderInvert`: Invert bit order within each BYTE of an array.
pub fn bit_order_invert(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        let mut val = *b as u32;

        val = ((val >> 1) & 0x55) | ((val << 1) & 0xAA);
        val = ((val >> 2) & 0x33) | ((val << 2) & 0xCC);
        val = ((val >> 4) & 0x0F) | ((val << 4) & 0xF0);

        *b = val as u8;
    }
}

/// `TwoByteSwap`: Invert byte order within each 16-bits of an array.
pub fn two_byte_swap(buf: &mut [u8]) {
    for b in buf.chunks_exact_mut(2) {
        b.swap(0, 1);
    }
}

/// `FourByteSwap`: Invert byte order within each 32-bits of an array.
pub fn four_byte_swap(buf: &mut [u8]) {
    for b in buf.chunks_exact_mut(4) {
        b.reverse();
    }
}
