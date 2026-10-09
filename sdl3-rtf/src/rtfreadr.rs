// Rust translation of src/rtfreadr.c from SDL_rtf.
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
 * This file was adapted from Microsoft Rich Text Format Specification 1.6
 * http://msdn.microsoft.com/library/default.asp?url=/library/en-us/dnrtfspec/html/rtfspec.asp
 */

use sdl3::io::IoStream;

use crate::rtf::{FontEngine, FontFamily, FONT_BOLD, FONT_ITALIC, FONT_NORMAL, FONT_UNDERLINE};
use crate::rtfactn::c_str;
use crate::rtfdecl::*;
use crate::rtftype::*;
use crate::sdl_rtfreadr::{rtf_create_color, rtf_get_char};

/// `SDL_isdigit()`
fn is_digit(ch: i32) -> bool {
    (b'0' as i32..=b'9' as i32).contains(&ch)
}

/// `SDL_islower()`
fn is_lower(ch: i32) -> bool {
    (b'a' as i32..=b'z' as i32).contains(&ch)
}

/// `SDL_isalpha()`
fn is_alpha(ch: i32) -> bool {
    is_lower(ch) || (b'A' as i32..=b'Z' as i32).contains(&ch)
}

impl<E: FontEngine> Context<E> {
    /*
     * %%Function: ecAddFontEntry
     */
    pub(crate) fn ec_add_font_entry(
        &mut self,
        number: i32,
        name: &str,
        family: i32,
        charset: i32,
    ) -> i32 {
        let entry = FontEntry {
            number,
            name: name.to_owned(),
            family: FontFamily(family),
            charset,
            fonts: Vec::new(),
        };
        self.font_table.insert(0, entry);
        EC_OK
    }

    /*
     * %%Function: ecLookupFont
     */
    /// The font, as an index into the context's fonts
    pub(crate) fn ec_lookup_font(&mut self) -> Option<usize> {
        /* Figure out what size we should use */
        let mut size = self.chp.f_font_size;
        if size == 0 {
            size = 24;
        }

        /* Figure out what style we should use */
        let mut style = FONT_NORMAL;
        if self.chp.f_bold != 0 {
            style |= FONT_BOLD;
        }
        if self.chp.f_italic != 0 {
            style |= FONT_ITALIC;
        }
        if self.chp.f_underline != 0 {
            style |= FONT_UNDERLINE;
        }

        /* Search for the correct font */
        let mut entry = self
            .font_table
            .iter()
            .position(|e| e.number == self.chp.f_font);
        if entry.is_none() {
            /* Search for the first default font */
            entry = self
                .font_table
                .iter()
                .position(|e| e.family == FontFamily::DEFAULT);
        }
        let entry = match entry {
            Some(entry) => entry,
            None => {
                /* If we still didn't find a font, just use the first font */
                if !self.font_table.is_empty() {
                    0
                } else {
                    return None;
                }
            }
        };

        /* We found a font entry, now find the font */
        let e = &self.font_table[entry];
        if let Some(font) = e.fonts.iter().find(|f| size == f.size && style == f.style) {
            return Some(font.font);
        }
        let (name, family, charset) = (e.name.clone(), e.family, e.charset);

        /* Create a new font entry */
        let font = self.rtf_create_font(&name, family, charset, size, style)?;
        self.fonts.push(Some(font));
        let font = self.fonts.len() - 1;
        self.font_table[entry]
            .fonts
            .insert(0, RtfFont { font, size, style });
        Some(font)
    }

    /*
     * %%Function: ecClearFonts
     */
    pub(crate) fn ec_clear_fonts(&mut self) -> i32 {
        for entry in std::mem::take(&mut self.font_table) {
            for font in entry.fonts {
                if let Some(font) = self.fonts.get_mut(font.font).and_then(Option::take) {
                    self.rtf_free_font(font);
                }
            }
        }
        self.fonts.clear();
        EC_OK
    }

    /*
     * %%Function: ecAddColorEntry
     */
    pub(crate) fn ec_add_color_entry(&mut self, r: i32, g: i32, b: i32) -> i32 {
        let entry = ColorEntry {
            color: Some(rtf_create_color(r, g, b)),
            r: (r & 0xFF) as u8,
            g: (g & 0xFF) as u8,
            b: (b & 0xFF) as u8,
            a: 0,
        };
        self.color_table.push(entry);
        EC_OK
    }

    /*
     * %%Function: ecLookupColor
     */
    pub(crate) fn ec_lookup_color(&self) -> Option<sdl3::video::Color> {
        let index = self.chp.f_fg_color as i32;

        if index < 0 {
            return None;
        }
        self.color_table.get(index as usize).and_then(|e| e.color)
    }

    /*
     * %%Function: ecClearColors
     */
    pub(crate) fn ec_clear_colors(&mut self) -> i32 {
        // FIXME (upstream): the color table isn't reset to NULL after it is
        // freed, so loading a second document into a context frees it
        // again and adds the new colors to the freed list; it is emptied
        // here.
        self.color_table.clear();
        EC_OK
    }

    /*
     * %%Function: ecAddLine
     */
    pub(crate) fn ec_add_line(&mut self) -> i32 {
        /* Lookup the current font */
        let Some(font) = self.ec_lookup_font() else {
            return EC_FONT_NOT_FOUND;
        };

        let line = Line {
            pap: self.pap,
            line_width: 0,
            line_height: self.rtf_get_line_spacing(font),
            tabs: 0,
            blocks: Vec::new(),
            surfaces: Vec::new(),
        };

        self.lines.push(line);
        EC_OK
    }

    /*
     * %%Function: ecAddTab
     */
    pub(crate) fn ec_add_tab(&mut self) -> i32 {
        /* Add the tabs to the last line added */
        if self.lines.is_empty() {
            let status = self.ec_add_line();

            if status != EC_OK {
                return status;
            }
        }
        if let Some(line) = self.lines.last_mut() {
            line.tabs += 1;
        }
        EC_OK
    }

    /*
     * %%Function: ecAddText
     */
    pub(crate) fn ec_add_text(&mut self, text: &str) -> i32 {
        /* Lookup the current font */
        let Some(font) = self.ec_lookup_font() else {
            return EC_FONT_NOT_FOUND;
        };

        /* Add the text to the last line added */
        if self.lines.is_empty() {
            let status = self.ec_add_line();

            if status != EC_OK {
                return status;
            }
        }

        let color = self.ec_lookup_color();
        let num_chars = text.len() + 1;
        let tabs = self.lines.last().map_or(0, |l| l.tabs);
        let mut byte_offsets = vec![0; num_chars];
        let mut pixel_offsets = vec![0; num_chars];
        let mut n =
            self.rtf_get_character_offsets(font, text, &mut byte_offsets, &mut pixel_offsets);
        // (the layout reads the offsets up to the character count: kept
        // inside the arrays here, should a font engine count more)
        n = n.min(num_chars as i32 - 1);
        let text_block = TextBlock {
            font,
            color,
            tabs,
            text: text.to_owned(),
            num_chars: n,
            byte_offsets,
            pixel_offsets,
            line_height: self.rtf_get_line_spacing(font),
        };

        let pap = self.pap;
        if let Some(line) = self.lines.last_mut() {
            line.pap = pap;
            line.tabs = 0;
            line.blocks.push(text_block);
        }
        EC_OK
    }

    /*
     * %%Function: ecClearLines
     */
    pub(crate) fn ec_clear_lines(&mut self) -> i32 {
        for line in std::mem::take(&mut self.lines) {
            self.free_line(line);
        }
        EC_OK
    }

    /*
     * %%Function: ecClearContext
     */
    pub(crate) fn ec_clear_context(&mut self) -> i32 {
        if !self.data.is_empty() {
            self.data = Vec::new();
            self.datapos = 0;
        }
        self.values = [0; 4];

        self.ec_clear_fonts();
        self.ec_clear_colors();

        self.title = None;
        self.subject = None;
        self.author = None;

        self.chp = Chp::default();
        self.pap = Pap::default();
        self.sep = Sep::default();
        self.dop = Dop::default();

        self.ec_clear_lines();

        self.display_width = 0;
        self.display_height = 0;

        EC_OK
    }

    /*
     * %%Function: ecRtfGetChar
     */
    pub(crate) fn ec_rtf_get_char(&mut self, stream: &mut IoStream<'_>, ch: &mut i32) -> i32 {
        if self.nextch >= 0 {
            *ch = self.nextch;
            self.nextch = -1;
        } else {
            let mut c = 0u8;

            if rtf_get_char(stream, &mut c) != 1 {
                return EC_END_OF_FILE;
            }
            *ch = c as i32;
        }
        EC_OK
    }

    /*
     * %%Function: ecRtfUngetChar
     */
    pub(crate) fn ec_rtf_unget_char(&mut self, ch: i32) -> i32 {
        self.nextch = ch;
        EC_OK
    }

    /*
     * %%Function: ecRtfParse
     *
     * Step 1:
     * Isolate RTF keywords and send them to ecParseRtfKeyword;
     * Push and pop state at the start and end of RTF groups;
     * Send text to ecParseChar for further processing.
     */
    pub(crate) fn ec_rtf_parse(&mut self, stream: &mut IoStream<'_>) -> i32 {
        let mut ch = 0;
        let mut ec;
        let mut c_nibble = 2;
        let mut b: i32 = 0;

        while self.ec_rtf_get_char(stream, &mut ch) == EC_OK {
            if self.c_group < 0 {
                return EC_STACK_UNDERFLOW;
            }
            if self.ris == Ris::Bin {
                /* if we're parsing binary data, handle it directly */
                ec = self.ec_parse_char(ch);
                if ec != EC_OK {
                    return ec;
                }
            } else {
                match ch as u8 {
                    b'{' => {
                        self.ec_process_data();
                        ec = self.ec_push_rtf_state();
                        if ec != EC_OK {
                            return ec;
                        }
                    }
                    b'}' => {
                        self.ec_process_data();
                        ec = self.ec_pop_rtf_state();
                        if ec != EC_OK {
                            return ec;
                        }
                    }
                    b'\\' => {
                        self.ec_process_data();
                        ec = self.ec_parse_rtf_keyword(stream);
                        if ec != EC_OK {
                            return ec;
                        }
                    }
                    0x0d | 0x0a => { /* cr and lf are noise characters... */ }
                    _ => {
                        if self.ris == Ris::Norm {
                            ec = self.ec_parse_char(ch);
                            if ec != EC_OK {
                                return ec;
                            }
                        } else {
                            /* parsing hex data */
                            if self.ris != Ris::Hex {
                                return EC_ASSERTION;
                            }
                            b <<= 4;
                            if is_digit(ch) {
                                b += ch - b'0' as i32;
                            } else if is_lower(ch) {
                                if ch < b'a' as i32 || ch > b'f' as i32 {
                                    return EC_INVALID_HEX;
                                }
                                b += 10 + (ch - b'a' as i32);
                            } else {
                                if ch < b'A' as i32 || ch > b'F' as i32 {
                                    return EC_INVALID_HEX;
                                }
                                b += 10 + (ch - b'A' as i32);
                            }
                            c_nibble -= 1;
                            if c_nibble == 0 {
                                ec = self.ec_parse_char(b);
                                if ec != EC_OK {
                                    return ec;
                                }
                                c_nibble = 2;
                                b = 0;
                                self.ris = Ris::Norm;
                            }
                        } /* end else (ris != risNorm) */
                    }
                } /* switch */
            } /* else (ris != risBin) */
        } /* while */
        if self.c_group < 0 {
            return EC_STACK_UNDERFLOW;
        }
        if self.c_group > 0 {
            return EC_UNMATCHED_BRACE;
        }
        EC_OK
    }

    /*
     * %%Function: ecPushRtfState
     *
     * Save relevant info on a linked list of SAVE structures.
     */
    pub(crate) fn ec_push_rtf_state(&mut self) -> i32 {
        let psave_new = Save {
            chp: self.chp,
            pap: self.pap,
            sep: self.sep,
            dop: self.dop,
            rds: self.rds,
            ris: self.ris,
        };
        self.ris = Ris::Norm;
        self.psave.push(psave_new);
        self.c_group += 1;
        EC_OK
    }

    /*
     * %%Function: ecPopRtfState
     *
     * If we're ending a destination (that is, the destination is changing),
     * call ecEndGroupAction.
     * Always restore relevant info from the top of the SAVE list.
     */
    pub(crate) fn ec_pop_rtf_state(&mut self) -> i32 {
        let Some(&psave) = self.psave.last() else {
            return EC_STACK_UNDERFLOW;
        };

        if self.rds != psave.rds {
            let ec = self.ec_end_group_action(self.rds);
            if ec != EC_OK {
                return ec;
            }
        }

        self.chp = psave.chp;
        self.pap = psave.pap;
        self.sep = psave.sep;
        self.dop = psave.dop;
        self.rds = psave.rds;
        self.ris = psave.ris;

        self.psave.pop();
        self.c_group -= 1;
        EC_OK
    }

    /*
     * %%Function: ecParseRtfKeyword
     *
     * Step 2:
     * get a control word (and its associated value) and
     * call ecTranslateKeyword to dispatch the control.
     */
    pub(crate) fn ec_parse_rtf_keyword(&mut self, stream: &mut IoStream<'_>) -> i32 {
        let mut ch = 0;
        let mut f_param = false;
        let mut f_neg = false;
        let mut param = 0;
        // FIXME (upstream): the keyword (char szKeyword[30]) and its
        // parameter (char szParameter[20]) are read into fixed buffers
        // without a bound, overflowing them on a longer keyword or
        // parameter; they grow as needed here.
        let mut sz_keyword: Vec<u8> = Vec::new();
        let mut sz_parameter: Vec<u8> = Vec::new();

        if self.ec_rtf_get_char(stream, &mut ch) != EC_OK {
            return EC_END_OF_FILE;
        }
        if !is_alpha(ch) {
            /* a control symbol; no delimiter. */
            sz_keyword.push(ch as u8);
            return self.ec_translate_keyword(&sz_keyword, 0, f_param);
        }
        while is_alpha(ch) {
            sz_keyword.push(ch as u8);
            if self.ec_rtf_get_char(stream, &mut ch) != EC_OK {
                return EC_END_OF_FILE;
            }
        }
        if ch == b'-' as i32 {
            f_neg = true;
            if self.ec_rtf_get_char(stream, &mut ch) != EC_OK {
                return EC_END_OF_FILE;
            }
        }
        if is_digit(ch) {
            f_param = true; /* a digit after the control means we have a parameter */
            while is_digit(ch) {
                sz_parameter.push(ch as u8);
                if self.ec_rtf_get_char(stream, &mut ch) != EC_OK {
                    return EC_END_OF_FILE;
                }
            }
            /* SDL_atoi() and SDL_strtol(): strtol() saturates */
            self.l_param = sdl3::stdlib::string::strtol(&sz_parameter[..], 10).0;
            param = self.l_param as i32;
            if f_neg {
                param = param.wrapping_neg();
            }
            if f_neg {
                self.l_param = self.l_param.wrapping_neg();
            }
        }
        if ch != b' ' as i32 {
            self.ec_rtf_unget_char(ch);
        }
        self.ec_translate_keyword(&sz_keyword, param, f_param)
    }

    /*
     * %%Function: ecParseChar
     *
     * Route the character to the appropriate destination stream.
     */
    pub(crate) fn ec_parse_char(&mut self, ch: i32) -> i32 {
        if self.ris == Ris::Bin {
            self.cb_bin = self.cb_bin.wrapping_sub(1);
            if self.cb_bin <= 0 {
                self.ris = Ris::Norm;
            }
        }
        match self.rds {
            Rds::Norm => {
                /* Output a character. Properties are valid at this point. */
                if ch == b'\t' as i32 {
                    return self.ec_tabstop();
                }
                if ch == b'\r' as i32 {
                    return self.ec_linebreak();
                }
                if ch == b'\n' as i32 {
                    return self.ec_paragraph();
                }
                self.ec_print_char(ch)
            }
            Rds::Skip => {
                /* Toss this character. */
                EC_OK
            }
            Rds::FontTable => {
                if ch == b';' as i32 {
                    // FIXME (upstream): data is NULL when no character was
                    // printed since the document started loading (a first
                    // font without a name), and this dereferences it; the
                    // name is empty here.
                    let name = if self.data.is_empty() {
                        String::new()
                    } else {
                        self.data[self.datapos] = 0;
                        String::from_utf8_lossy(c_str(&self.data)).into_owned()
                    };
                    self.ec_add_font_entry(
                        self.chp.f_font,
                        &name,
                        self.values[0],
                        self.chp.f_font_charset,
                    );
                    self.datapos = 0;
                    self.values[0] = 0;
                } else {
                    return self.ec_print_char(ch);
                }
                EC_OK
            }
            Rds::ColorTable => {
                if ch == b';' as i32 {
                    self.ec_add_color_entry(self.values[0], self.values[1], self.values[2]);
                    self.values[0] = 0;
                    self.values[1] = 0;
                    self.values[2] = 0;
                }
                EC_OK
            }
            Rds::Title | Rds::Subject | Rds::Author => self.ec_print_char(ch),
            _ => {
                /* handle other destinations.... */
                EC_OK
            }
        }
    }

    /*
     * %%Function: ecPrintChar
     *
     * Add a character to the output text
     */
    pub(crate) fn ec_print_char(&mut self, mut ch: i32) -> i32 {
        if self.datapos as isize >= self.data.len() as isize - 4 {
            /* 256 byte chunk size */
            let datamax = self.data.len() + 256;
            self.data.resize(datamax, 0);
        }
        /* Some common characters aren't in TrueType font maps */
        if ch == 147 || ch == 148 {
            ch = b'"' as i32;
        }

        /* Convert character into UTF-8 */
        let ch = ch as u32;
        let mut put = |c: u32| {
            self.data[self.datapos] = c as u8;
            self.datapos += 1;
        };
        if ch <= 0x7f {
            put(ch);
        } else if ch <= 0x7ff {
            put(0xc0 | (ch >> 6));
            put(0x80 | (ch & 0x3f));
        } else if ch <= 0xffff {
            put(0xe0 | (ch >> 12));
            put(0x80 | ((ch >> 6) & 0x3f));
            put(0x80 | (ch & 0x3f));
        } else {
            put(0xf0 | (ch >> 18));
            put(0x80 | ((ch >> 12) & 0x3f));
            put(0x80 | ((ch >> 6) & 0x3f));
            put(0x80 | (ch & 0x3f));
        }
        EC_OK
    }

    /*
     * %%Function: ecProcessData
     *
     * Flush the output text
     */
    pub(crate) fn ec_process_data(&mut self) -> i32 {
        let mut status = EC_OK;

        if self.rds == Rds::Norm && self.datapos > 0 {
            self.data[self.datapos] = 0;
            let text = String::from_utf8_lossy(c_str(&self.data)).into_owned();
            status = self.ec_add_text(&text);
            self.datapos = 0;
        }
        status
    }

    /*
     * %%Function: ecTabstop
     *
     * Flush the output text and add a tabstop
     */
    pub(crate) fn ec_tabstop(&mut self) -> i32 {
        let mut status = self.ec_process_data();
        if status == EC_OK {
            status = self.ec_add_tab();
        }
        status
    }

    /*
     * %%Function: ecLinebreak
     *
     * Flush the output text and move to the next line
     */
    pub(crate) fn ec_linebreak(&mut self) -> i32 {
        let mut status = self.ec_process_data();
        if status == EC_OK {
            status = self.ec_add_line();
        }
        status
    }

    /*
     * %%Function: ecParagraph
     *
     * Flush the output text and start a new paragraph
     */
    pub(crate) fn ec_paragraph(&mut self) -> i32 {
        self.ec_linebreak()
    }

    /// `FreeLine()`
    fn free_line(&mut self, line: Line) {
        for surface in line.surfaces {
            self.rtf_free_surface(surface.surface);
        }
        /* the text blocks (FreeTextBlock()) are dropped with the line */
    }
}

/* vi: set ts=4 sw=4 expandtab: */
