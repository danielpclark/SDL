// Rust translation of the TTF_Text part of src/SDL_ttf.c from SDL_ttf.
// Copyright (C) 2001-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `TTF_Text`: not translated yet; the font keeps weak references to its
//! texts so that changing the font marks them for a new layout.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ttf::FontData;

/// `TTF_TextData` (the parts the font updates)
#[derive(Debug, Default)]
pub(crate) struct TextData {
    /// `needs_layout_update`
    pub needs_layout_update: bool,
}

/// `RemoveOneTextCallback`: the font is closing, the text loses it
pub(crate) fn detach_font(text: &Rc<RefCell<TextData>>, _font: &Rc<RefCell<FontData>>) {
    if let Ok(mut text) = text.try_borrow_mut() {
        text.needs_layout_update = true;
    }
}
