// Rust translation of src/base/ftpsprop.c from FreeType (2.13.2, as
// SDL_ttf's external/freetype pins it).
// Copyright (C) 2017-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Get and set properties of PostScript drivers (body).
//! See `ftdriver.h' for available properties.
//!
//! `CFF_CONFIG_OPTION_OLD_ENGINE` and `T1_CONFIG_OPTION_OLD_ENGINE` are
//! undefined, so the only hinting engine is Adobe's;
//! `FT_CONFIG_OPTION_ENVIRONMENT_PROPERTIES` is defined.

use super::super::fttypes::*;
use super::super::psaux::{PsDriverRec, FT_HINTING_ADOBE};
use super::ftobjs::*;

/// `ft_strtol( s, &ep, 10 )`: the value and the number of bytes read
/// (0 when there is no number).
fn ft_strtol(s: &[u8]) -> (FtLong, usize) {
    let mut i = 0;
    while i < s.len() && (s[i] == b' ' || (b'\t'..=b'\r').contains(&s[i])) {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'-' || s[i] == b'+') {
        neg = s[i] == b'-';
        i += 1;
    }
    let start = i;
    let mut v: FtLong = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        v = v.saturating_mul(10).saturating_add((s[i] - b'0') as FtLong);
        i += 1;
    }
    if i == start {
        return (0, 0);
    }
    (if neg { -v } else { v }, i)
}

/// `ps_property_set`
pub fn ps_property_set(
    module: &FtModuleRec, /* PS_Driver */
    property_name: &str,
    value: &FtPropertyValue,
    value_is_string: bool,
) -> FtResult<()> {
    let error = Ok(());

    if property_name == "darkening-parameters" {
        let mut dp: [FtInt; 8] = [0; 8];

        /* FT_CONFIG_OPTION_ENVIRONMENT_PROPERTIES */
        if value_is_string {
            let FtPropertyValue::Str(s) = value else {
                return Err(FT_ERR_INVALID_ARGUMENT);
            };
            let mut s = s.as_bytes();

            /* eight comma-separated numbers */
            for slot in dp.iter_mut().take(7) {
                let (v, n) = ft_strtol(s);
                *slot = v as FtInt;
                if n == 0 || s.get(n) != Some(&b',') {
                    return Err(FT_ERR_INVALID_ARGUMENT);
                }

                s = &s[n + 1..];
            }

            let (v, n) = ft_strtol(s);
            dp[7] = v as FtInt;
            if !(s.get(n).is_none() || s.get(n) == Some(&b' ')) || n == 0 {
                return Err(FT_ERR_INVALID_ARGUMENT);
            }
        } else {
            match value {
                FtPropertyValue::IntArray(a) if a.len() >= 8 => dp.copy_from_slice(&a[..8]),
                _ => return Err(FT_ERR_INVALID_ARGUMENT),
            }
        }

        let [x1, y1, x2, y2, x3, y3, x4, y4] = dp;

        if x1 < 0
            || x2 < 0
            || x3 < 0
            || x4 < 0
            || y1 < 0
            || y2 < 0
            || y3 < 0
            || y4 < 0
            || x1 > x2
            || x2 > x3
            || x3 > x4
            || y1 > 500
            || y2 > 500
            || y3 > 500
            || y4 > 500
        {
            return Err(FT_ERR_INVALID_ARGUMENT);
        }

        module.with_props(|driver: &mut PsDriverRec| driver.darken_params = dp);

        return error;
    } else if property_name == "hinting-engine" {
        /* FT_CONFIG_OPTION_ENVIRONMENT_PROPERTIES */
        if value_is_string {
            let FtPropertyValue::Str(s) = value else {
                return Err(FT_ERR_INVALID_ARGUMENT);
            };

            if s == "adobe" {
                module.with_props(|driver: &mut PsDriverRec| {
                    driver.hinting_engine = FT_HINTING_ADOBE
                });
            } else {
                return Err(FT_ERR_INVALID_ARGUMENT);
            }
            return error;
        } else {
            let hinting_engine = match *value {
                FtPropertyValue::UInt(v) => v,
                FtPropertyValue::Int(v) => v as FtUInt,
                _ => return Err(FT_ERR_INVALID_ARGUMENT),
            };

            if hinting_engine == FT_HINTING_ADOBE {
                module
                    .with_props(|driver: &mut PsDriverRec| driver.hinting_engine = hinting_engine);
                return error;
            } else {
                return Err(FT_ERR_UNIMPLEMENTED_FEATURE);
            }
        }
    } else if property_name == "no-stem-darkening" {
        /* FT_CONFIG_OPTION_ENVIRONMENT_PROPERTIES */
        let no_stem_darkening = if value_is_string {
            let FtPropertyValue::Str(s) = value else {
                return Err(FT_ERR_INVALID_ARGUMENT);
            };
            let (nsd, _) = ft_strtol(s.as_bytes());

            nsd != 0
        } else {
            match *value {
                FtPropertyValue::Bool(b) => b,
                FtPropertyValue::Int(v) => v as FtByte != 0,
                FtPropertyValue::UInt(v) => v as FtByte != 0,
                _ => return Err(FT_ERR_INVALID_ARGUMENT),
            }
        };

        module.with_props(|driver: &mut PsDriverRec| driver.no_stem_darkening = no_stem_darkening);

        return error;
    } else if property_name == "random-seed" {
        /* FT_CONFIG_OPTION_ENVIRONMENT_PROPERTIES */
        let mut random_seed: FtInt32 = if value_is_string {
            let FtPropertyValue::Str(s) = value else {
                return Err(FT_ERR_INVALID_ARGUMENT);
            };
            ft_strtol(s.as_bytes()).0 as FtInt32
        } else {
            match *value {
                FtPropertyValue::Int(v) => v,
                FtPropertyValue::UInt(v) => v as FtInt32,
                _ => return Err(FT_ERR_INVALID_ARGUMENT),
            }
        };

        if random_seed < 0 {
            random_seed = 0;
        }

        module.with_props(|driver: &mut PsDriverRec| driver.random_seed = random_seed);

        return error;
    }

    Err(FT_ERR_MISSING_PROPERTY)
}

/// `ps_property_get`
pub fn ps_property_get(
    module: &FtModuleRec, /* PS_Driver */
    property_name: &str,
) -> FtResult<FtPropertyValue> {
    let driver = module
        .with_props(|driver: &mut PsDriverRec| *driver)
        .unwrap_or_default();

    if property_name == "darkening-parameters" {
        return Ok(FtPropertyValue::IntArray(driver.darken_params.to_vec()));
    } else if property_name == "hinting-engine" {
        return Ok(FtPropertyValue::UInt(driver.hinting_engine));
    } else if property_name == "no-stem-darkening" {
        return Ok(FtPropertyValue::Bool(driver.no_stem_darkening));
    }

    Err(FT_ERR_MISSING_PROPERTY)
}
