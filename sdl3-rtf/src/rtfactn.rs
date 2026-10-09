// Rust translation of src/rtfactn.c from SDL_rtf.
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
 * This file was adapted from Microsoft Rich Text Format Specification 1.6
 * http://msdn.microsoft.com/library/default.asp?url=/library/en-us/dnrtfspec/html/rtfspec.asp
 */

use crate::rtf::FontEngine;
use crate::rtfdecl::*;
use crate::rtftype::*;

/* RTF parser tables */

const fn prop(actn: Actn, prop: PropType, offset: Field) -> Prop {
    Prop { actn, prop, offset }
}

/* Property descriptions */
pub(crate) static RGPROP: [Prop; IPROP_MAX] = [
    prop(Actn::Spec, PropType::Chp, Field::None), /* ipropFontFamily */
    prop(Actn::Word, PropType::Chp, Field::ChpFontCharset), /* ipropFontCharset */
    prop(Actn::Spec, PropType::Chp, Field::None), /* ipropColorRed */
    prop(Actn::Spec, PropType::Chp, Field::None), /* ipropColorGreen */
    prop(Actn::Spec, PropType::Chp, Field::None), /* ipropColorBlue */
    prop(Actn::Word, PropType::Chp, Field::ChpFont), /* ipropFont */
    prop(Actn::Word, PropType::Chp, Field::ChpFontSize), /* ipropFontSize */
    prop(Actn::Byte, PropType::Chp, Field::ChpBgColor), /* ipropBgColor */
    prop(Actn::Byte, PropType::Chp, Field::ChpFgColor), /* ipropFgColor */
    prop(Actn::Byte, PropType::Chp, Field::ChpBold), /* ipropBold */
    prop(Actn::Byte, PropType::Chp, Field::ChpItalic), /* ipropItalic */
    prop(Actn::Byte, PropType::Chp, Field::ChpUnderline), /* ipropUnderline */
    prop(Actn::Word, PropType::Pap, Field::PapXaLeft), /* ipropLeftInd */
    prop(Actn::Word, PropType::Pap, Field::PapXaRight), /* ipropRightInd */
    prop(Actn::Word, PropType::Pap, Field::PapXaFirst), /* ipropFirstInd */
    prop(Actn::Word, PropType::Sep, Field::SepCCols), /* ipropCols */
    prop(Actn::Word, PropType::Sep, Field::SepXaPgn), /* ipropPgnX */
    prop(Actn::Word, PropType::Sep, Field::SepYaPgn), /* ipropPgnY */
    prop(Actn::Word, PropType::Dop, Field::DopXaPage), /* ipropXaPage */
    prop(Actn::Word, PropType::Dop, Field::DopYaPage), /* ipropYaPage */
    prop(Actn::Word, PropType::Dop, Field::DopXaLeft), /* ipropXaLeft */
    prop(Actn::Word, PropType::Dop, Field::DopXaRight), /* ipropXaRight */
    prop(Actn::Word, PropType::Dop, Field::DopYaTop), /* ipropYaTop */
    prop(Actn::Word, PropType::Dop, Field::DopYaBottom), /* ipropYaBottom */
    prop(Actn::Word, PropType::Dop, Field::DopPgnStart), /* ipropPgnStart */
    prop(Actn::Byte, PropType::Sep, Field::SepSbk), /* ipropSbk */
    prop(Actn::Byte, PropType::Sep, Field::SepPgnFormat), /* ipropPgnFormat */
    prop(Actn::Byte, PropType::Dop, Field::DopFFacingp), /* ipropFacingp */
    prop(Actn::Byte, PropType::Dop, Field::DopFLandscape), /* ipropLandscape */
    prop(Actn::Byte, PropType::Pap, Field::PapJust), /* ipropJust */
    prop(Actn::Spec, PropType::Pap, Field::None), /* ipropPard */
    prop(Actn::Spec, PropType::Chp, Field::None), /* ipropPlain */
    prop(Actn::Spec, PropType::Sep, Field::None), /* ipropSectd */
];

const fn sym(sz_keyword: &'static str, dflt: i32, f_pass_dflt: bool, kwd: Kwd) -> Sym {
    Sym {
        sz_keyword,
        dflt,
        f_pass_dflt,
        kwd,
    }
}

use Idest as D;
use Iprop as P;
use Kwd::{Char, Dest, Prop as KProp, Spec};

/* Keyword descriptions */
pub(crate) static RGSYM_RTF: &[Sym] = &[
    /* keyword, dflt, fPassDflt, kwd, idx */
    sym("fonttbl", 0, false, Dest(D::FontTable)),
    sym("fnil", FNIL, true, KProp(P::FontFamily)),
    sym("froman", FROMAN, true, KProp(P::FontFamily)),
    sym("fswiss", FSWISS, true, KProp(P::FontFamily)),
    sym("fmodern", FMODERN, true, KProp(P::FontFamily)),
    sym("fscript", FSCRIPT, true, KProp(P::FontFamily)),
    sym("fdecor", FDECOR, true, KProp(P::FontFamily)),
    sym("ftech", FTECH, true, KProp(P::FontFamily)),
    sym("fbidi", FBIDI, true, KProp(P::FontFamily)),
    sym("fcharset", 0, false, KProp(P::FontCharset)),
    sym("colortbl", 0, false, Dest(D::ColorTable)),
    sym("red", 0, false, KProp(P::ColorRed)),
    sym("green", 0, false, KProp(P::ColorGreen)),
    sym("blue", 0, false, KProp(P::ColorBlue)),
    sym("info", 0, false, Dest(D::Info)),
    sym("title", 0, false, Dest(D::Title)),
    sym("subject", 0, false, Dest(D::Subject)),
    sym("author", 0, false, Dest(D::Author)),
    sym("f", 0, false, KProp(P::Font)),
    sym("fs", 24, false, KProp(P::FontSize)),
    sym("cb", 1, false, KProp(P::BgColor)),
    sym("cf", 1, false, KProp(P::FgColor)),
    sym("b", 1, false, KProp(P::Bold)),
    sym("ul", 1, false, KProp(P::Underline)),
    sym("ulnone", 0, true, KProp(P::Underline)),
    sym("i", 1, false, KProp(P::Italic)),
    sym("li", 0, false, KProp(P::LeftInd)),
    sym("ri", 0, false, KProp(P::RightInd)),
    sym("fi", 0, false, KProp(P::FirstInd)),
    sym("cols", 1, false, KProp(P::Cols)),
    sym("sbknone", SBK_NON, true, KProp(P::Sbk)),
    sym("sbkcol", SBK_COL, true, KProp(P::Sbk)),
    sym("sbkeven", SBK_EVN, true, KProp(P::Sbk)),
    sym("sbkodd", SBK_ODD, true, KProp(P::Sbk)),
    sym("sbkpage", SBK_PG, true, KProp(P::Sbk)),
    sym("pgnx", 0, false, KProp(P::PgnX)),
    sym("pgny", 0, false, KProp(P::PgnY)),
    sym("pgndec", PG_DEC, true, KProp(P::PgnFormat)),
    sym("pgnucrm", PG_U_ROM, true, KProp(P::PgnFormat)),
    sym("pgnlcrm", PG_L_ROM, true, KProp(P::PgnFormat)),
    sym("pgnucltr", PG_U_LTR, true, KProp(P::PgnFormat)),
    sym("pgnlcltr", PG_L_LTR, true, KProp(P::PgnFormat)),
    sym("qc", JUST_C, true, KProp(P::Just)),
    sym("ql", JUST_L, true, KProp(P::Just)),
    sym("qr", JUST_R, true, KProp(P::Just)),
    sym("qj", JUST_F, true, KProp(P::Just)),
    sym("paperw", 12240, false, KProp(P::XaPage)),
    sym("paperh", 15480, false, KProp(P::YaPage)),
    sym("margl", 1800, false, KProp(P::XaLeft)),
    sym("margr", 1800, false, KProp(P::XaRight)),
    sym("margt", 1440, false, KProp(P::YaTop)),
    sym("margb", 1440, false, KProp(P::YaBottom)),
    sym("pgnstart", 1, true, KProp(P::PgnStart)),
    sym("facingp", 1, true, KProp(P::Facingp)),
    sym("landscape", 1, true, KProp(P::Landscape)),
    sym("line", 0, false, Char(b'\n' as i32)),
    sym("par", 0, false, Char(b'\n' as i32)),
    // (a C string that ends at its first character, as strcmp() sees it:
    // the keyword of a backslash followed by a NUL byte)
    sym("\0x0a", 0, false, Char(b'\n' as i32)),
    sym("\r", 0, false, Char(b'\n' as i32)),
    sym("\0x0d", 0, false, Char(b'\n' as i32)),
    sym("\n", 0, false, Char(b'\n' as i32)),
    sym("tab", 0, false, Char(b'\t' as i32)),
    sym("ldblquote", 0, false, Char(b'"' as i32)),
    sym("rdblquote", 0, false, Char(b'"' as i32)),
    sym("bin", 0, false, Spec(Ipfn::Bin)),
    sym("*", 0, false, Spec(Ipfn::SkipDest)),
    sym("'", 0, false, Spec(Ipfn::Hex)),
    sym("bkmkend", 0, false, Dest(D::Skip)),
    sym("bkmkstart", 0, false, Dest(D::Skip)),
    sym("buptim", 0, false, Dest(D::Skip)),
    sym("colortbl", 0, false, Dest(D::Skip)),
    sym("comment", 0, false, Dest(D::Skip)),
    sym("creatim", 0, false, Dest(D::Skip)),
    sym("doccomm", 0, false, Dest(D::Skip)),
    sym("fonttbl", 0, false, Dest(D::Skip)),
    sym("footer", 0, false, Dest(D::Skip)),
    sym("footerf", 0, false, Dest(D::Skip)),
    sym("footerl", 0, false, Dest(D::Skip)),
    sym("footerr", 0, false, Dest(D::Skip)),
    sym("footnote", 0, false, Dest(D::Skip)),
    sym("ftncn", 0, false, Dest(D::Skip)),
    sym("ftnsep", 0, false, Dest(D::Skip)),
    sym("ftnsepc", 0, false, Dest(D::Skip)),
    sym("header", 0, false, Dest(D::Skip)),
    sym("headerf", 0, false, Dest(D::Skip)),
    sym("headerl", 0, false, Dest(D::Skip)),
    sym("headerr", 0, false, Dest(D::Skip)),
    sym("info", 0, false, Dest(D::Skip)),
    sym("keywords", 0, false, Dest(D::Skip)),
    sym("operator", 0, false, Dest(D::Skip)),
    sym("pict", 0, false, Dest(D::Skip)),
    sym("printim", 0, false, Dest(D::Skip)),
    sym("private1", 0, false, Dest(D::Skip)),
    sym("revtim", 0, false, Dest(D::Skip)),
    sym("rxe", 0, false, Dest(D::Skip)),
    sym("stylesheet", 0, false, Dest(D::Skip)),
    sym("tc", 0, false, Dest(D::Skip)),
    sym("txe", 0, false, Dest(D::Skip)),
    sym("xe", 0, false, Dest(D::Skip)),
    sym("{", 0, false, Char(b'{' as i32)),
    sym("}", 0, false, Char(b'}' as i32)),
    sym("\\", 0, false, Char(b'\\' as i32)),
    sym("pard", 0, false, KProp(P::Pard)),
    sym("plain", 0, false, KProp(P::Plain)),
    sym("sectd", 0, false, KProp(P::Sectd)),
];

/// A C string: the bytes before the first NUL
pub(crate) fn c_str(s: &[u8]) -> &[u8] {
    match s.iter().position(|&c| c == 0) {
        Some(n) => &s[..n],
        None => s,
    }
}

/// `pb[offset] = (unsigned char) val` on an `int` field: the low byte, as
/// on the little-endian platforms
fn set_low_byte(field: &mut i32, val: u8) {
    *field = (*field & !0xFF) | val as i32;
}

impl<E: FontEngine> Context<E> {
    /// `pb[rgprop[iprop].offset] = (unsigned char) val;` (`actnByte`)
    fn set_prop_byte(&mut self, offset: Field, val: u8) -> i32 {
        match offset {
            Field::ChpBgColor => self.chp.f_bg_color = val as i8,
            Field::ChpFgColor => self.chp.f_fg_color = val as i8,
            Field::ChpBold => self.chp.f_bold = val as i8,
            Field::ChpItalic => self.chp.f_italic = val as i8,
            Field::ChpUnderline => self.chp.f_underline = val as i8,
            Field::PapJust => set_low_byte(&mut self.pap.just, val),
            Field::SepSbk => set_low_byte(&mut self.sep.sbk, val),
            Field::SepPgnFormat => set_low_byte(&mut self.sep.pgn_format, val),
            Field::DopFFacingp => self.dop.f_facingp = val as i8,
            Field::DopFLandscape => self.dop.f_landscape = val as i8,
            _ => return EC_BAD_TABLE,
        }
        EC_OK
    }

    /// `(*(int *) (pb + rgprop[iprop].offset)) = val;` (`actnWord`)
    fn set_prop_word(&mut self, offset: Field, val: i32) -> i32 {
        match offset {
            Field::ChpFontCharset => self.chp.f_font_charset = val,
            Field::ChpFont => self.chp.f_font = val,
            Field::ChpFontSize => self.chp.f_font_size = val,
            Field::PapXaLeft => self.pap.xa_left = val,
            Field::PapXaRight => self.pap.xa_right = val,
            Field::PapXaFirst => self.pap.xa_first = val,
            Field::SepCCols => self.sep.c_cols = val,
            Field::SepXaPgn => self.sep.xa_pgn = val,
            Field::SepYaPgn => self.sep.ya_pgn = val,
            Field::DopXaPage => self.dop.xa_page = val,
            Field::DopYaPage => self.dop.ya_page = val,
            Field::DopXaLeft => self.dop.xa_left = val,
            Field::DopXaRight => self.dop.xa_right = val,
            Field::DopYaTop => self.dop.ya_top = val,
            Field::DopYaBottom => self.dop.ya_bottom = val,
            Field::DopPgnStart => self.dop.pgn_start = val,
            _ => return EC_BAD_TABLE,
        }
        EC_OK
    }

    /*
     * %%Function: ecApplyPropChange
     *
     * Set the property identified by _iprop_ to the value _val_.
     *
     */
    pub(crate) fn ec_apply_prop_change(&mut self, iprop: Iprop, val: i32) -> i32 {
        if self.rds == Rds::Skip {
            /* If we're skipping text, */
            return EC_OK; /* don't do anything. */
        }

        let prop = &RGPROP[iprop as usize];
        // (the structure is picked by the field: every PROPTYPE has one)
        let _ = prop.prop;
        match prop.actn {
            Actn::Byte => self.set_prop_byte(prop.offset, val as u8),
            Actn::Word => self.set_prop_word(prop.offset, val),
            Actn::Spec => self.ec_parse_special_property(iprop, val),
        }
    }

    /*
     * %%Function: ecParseSpecialProperty
     *
     * Set a property that requires code to evaluate.
     */
    pub(crate) fn ec_parse_special_property(&mut self, iprop: Iprop, val: i32) -> i32 {
        match iprop {
            Iprop::FontFamily => {
                self.values[0] = val;
                EC_OK
            }
            Iprop::ColorRed => {
                self.values[0] = val;
                EC_OK
            }
            Iprop::ColorGreen => {
                self.values[1] = val;
                EC_OK
            }
            Iprop::ColorBlue => {
                self.values[2] = val;
                EC_OK
            }
            Iprop::Pard => {
                self.pap = Pap::default();
                EC_OK
            }
            Iprop::Plain => {
                self.chp = Chp::default();
                EC_OK
            }
            Iprop::Sectd => {
                self.sep = Sep::default();
                EC_OK
            }
            _ => EC_BAD_TABLE,
        }
    }

    /*
     * %%Function: ecTranslateKeyword.
     *
     * Step 3.
     * Search rgsymRtf for szKeyword and evaluate it appropriately.
     *
     * Inputs:
     * szKeyword:   The RTF control to evaluate.
     * param:       The parameter of the RTF control.
     * fParam:      true if the control had a parameter; (that is, if param
     *                    is valid)
     *              false if it did not.
     */
    pub(crate) fn ec_translate_keyword(
        &mut self,
        sz_keyword: &[u8],
        mut param: i32,
        f_param: bool,
    ) -> i32 {
        /* search for szKeyword in rgsymRtf */
        let sz_keyword = c_str(sz_keyword);
        let Some(sym) = RGSYM_RTF
            .iter()
            .find(|s| c_str(s.sz_keyword.as_bytes()) == sz_keyword)
        else {
            /* control word not found */
            if self.f_skip_dest_if_unk {
                /* if this is a new destination */
                self.rds = Rds::Skip; /* skip the destination */
            }
            /* else just discard it */
            self.f_skip_dest_if_unk = false;
            return EC_OK;
        };

        /* found it!  use kwd and idx to determine what to do with it. */
        self.f_skip_dest_if_unk = false;
        match sym.kwd {
            Kwd::Prop(iprop) => {
                if sym.f_pass_dflt || !f_param {
                    param = sym.dflt;
                }
                self.ec_apply_prop_change(iprop, param)
            }
            Kwd::Char(ch) => self.ec_parse_char(ch),
            Kwd::Dest(idest) => self.ec_change_dest(idest),
            Kwd::Spec(ipfn) => self.ec_parse_special_keyword(ipfn),
        }
    }

    /*
     * %%Function: ecChangeDest
     *
     * Change to the destination specified by idest.
     * There's usually more to do here than this...
     */
    pub(crate) fn ec_change_dest(&mut self, idest: Idest) -> i32 {
        if self.rds == Rds::Skip {
            /* if we're skipping text, */
            return EC_OK; /* don't do anything */
        }

        match idest {
            Idest::FontTable => {
                self.rds = Rds::FontTable;
                self.datapos = 0;
            }
            Idest::ColorTable => {
                self.rds = Rds::ColorTable;
                self.values[0] = 0;
                self.values[1] = 0;
                self.values[2] = 0;
            }
            Idest::Info => {
                self.rds = Rds::Info;
                self.datapos = 0;
            }
            Idest::Title => {
                self.rds = Rds::Title;
                self.datapos = 0;
            }
            Idest::Subject => {
                self.rds = Rds::Subject;
                self.datapos = 0;
            }
            Idest::Author => {
                self.rds = Rds::Author;
                self.datapos = 0;
            }
            _ => {
                self.rds = Rds::Skip; /* when in doubt, skip it... */
            }
        }
        EC_OK
    }

    /// `ctx->data[ctx->datapos] = '\0';` then the C string in `data`, or
    /// `None` when it is empty (`*ctx->data ? SDL_strdup(ctx->data) : NULL`)
    fn take_data_string(&mut self) -> Option<String> {
        // FIXME (upstream): data is NULL when no character was printed
        // since the document started loading ({\title} as the first
        // text), and this dereferences it; it is an empty string here.
        if self.data.is_empty() {
            return None;
        }
        self.data[self.datapos] = 0;
        let s = c_str(&self.data);
        if s.is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(s).into_owned())
        }
    }

    /*
     * %%Function: ecEndGroupAction
     *
     * The destination specified by rds is coming to a close.
     * If there's any cleanup that needs to be done, do it now.
     */
    pub(crate) fn ec_end_group_action(&mut self, _rds: Rds) -> i32 {
        match self.rds {
            Rds::Title => {
                self.title = self.take_data_string();
                self.datapos = 0;
            }
            Rds::Subject => {
                self.subject = self.take_data_string();
                self.datapos = 0;
            }
            Rds::Author => {
                self.author = self.take_data_string();
                self.datapos = 0;
            }
            _ => {}
        }
        EC_OK
    }

    /*
     * %%Function: ecParseSpecialKeyword
     *
     * Evaluate an RTF control that needs special processing.
     */
    pub(crate) fn ec_parse_special_keyword(&mut self, ipfn: Ipfn) -> i32 {
        if self.rds == Rds::Skip && ipfn != Ipfn::Bin {
            /* if we're skipping, and it's not */
            return EC_OK; /* the \bin keyword, ignore it. */
        }
        match ipfn {
            Ipfn::Bin => {
                self.ris = Ris::Bin;
                self.cb_bin = self.l_param;
            }
            Ipfn::SkipDest => {
                self.f_skip_dest_if_unk = true;
            }
            Ipfn::Hex => {
                self.ris = Ris::Hex;
            }
        }
        EC_OK
    }
}

/* vi: set ts=4 sw=4 expandtab: */
