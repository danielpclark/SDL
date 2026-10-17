/* FluidSynth - A Software Synthesizer
 *
 * Copyright (C) 2003  Peter Hanappe and others.
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public License
 * as published by the Free Software Foundation; either version 2.1 of
 * the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful, but
 * WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see
 * <https://www.gnu.org/licenses/>.
 */
// Modified 2026-10-17: translated into Rust from fluid_settings.c and
// fluid_settings.h in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE),
// with the registrations of fluid_settings_init() (the synth's, the
// shell's, the MIDI player's, the file renderer's and the audio and MIDI
// drivers': SDL_mixer's build has no drivers, libsndfile or readline).
// The settings tree's hash tables are `BTreeMap`s, its recursive mutex a
// `Mutex`. The update callbacks for realtime settings are left out (see
// synth.rs).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::fluidsynth_h::*;
use crate::utils::sys::{flog, FLUID_ERR};

const MAX_SETTINGS_TOKENS: usize = 8; /* currently only a max of 3 are used */
const MAX_SETTINGS_LABEL: usize = 256; /* max length of a settings variable label */

/// Translation of `fluid_str_setting_t`.
#[derive(Clone, Debug)]
struct FluidStrSetting {
    value: Option<String>,
    def: Option<String>,
    hints: i32,
    options: Vec<String>,
}

/// Translation of `fluid_num_setting_t`.
#[derive(Clone, Debug)]
struct FluidNumSetting {
    value: f64,
    def: f64,
    min: f64,
    max: f64,
    hints: i32,
}

/// Translation of `fluid_int_setting_t`.
#[derive(Clone, Debug)]
struct FluidIntSetting {
    value: i32,
    def: i32,
    min: i32,
    max: i32,
    hints: i32,
}

/// Translation of `fluid_setting_node_t`: common structure to all
/// settings nodes.
#[derive(Clone, Debug)]
enum FluidSettingNode {
    Str(FluidStrSetting),
    Num(FluidNumSetting),
    Int(FluidIntSetting),
    Set(BTreeMap<String, FluidSettingNode>),
}

impl FluidSettingNode {
    fn type_(&self) -> FluidType {
        match self {
            FluidSettingNode::Str(_) => FluidType::Str,
            FluidSettingNode::Num(_) => FluidType::Num,
            FluidSettingNode::Int(_) => FluidType::Int,
            FluidSettingNode::Set(_) => FluidType::Set,
        }
    }
}

/// Translation of `fluid_settings_t`: the settings tree.
#[derive(Clone, Debug, Default)]
pub(crate) struct SettingsInner {
    table: BTreeMap<String, FluidSettingNode>,
}

/// A settings object (`fluid_settings_t *`): a handle to a shared, locked
/// settings tree.
#[derive(Clone, Debug)]
pub struct Settings {
    inner: Arc<Mutex<SettingsInner>>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings::new()
    }
}

/// Translation of `fluid_strtok()` (fluid_sys.c) with a single delimiter:
/// the tokens, skipping empty ones.
fn fluid_strtok(s: &str, delim: char) -> impl Iterator<Item = &str> {
    s.split(delim).filter(|t| !t.is_empty())
}

/// Translation of `fluid_settings_tokenize()`: the tokens, or none on
/// error.
fn fluid_settings_tokenize(s: &str) -> Vec<&str> {
    if s.len() > MAX_SETTINGS_LABEL {
        flog!(
            FLUID_ERR,
            "Setting variable name exceeded max length of {} chars",
            MAX_SETTINGS_LABEL
        );
        return Vec::new();
    }

    let mut ptr = Vec::new();
    for tok in fluid_strtok(s, '.') {
        if ptr.len() >= MAX_SETTINGS_TOKENS {
            flog!(
                FLUID_ERR,
                "Setting variable name exceeded max token count of {}",
                MAX_SETTINGS_TOKENS
            );
            return Vec::new();
        } else {
            ptr.push(tok);
        }
    }

    ptr
}

impl SettingsInner {
    /// Translation of `fluid_settings_init()`.
    fn fluid_settings_init(&mut self) {
        crate::synth::synth::fluid_synth_settings(self);
        self.fluid_shell_settings();
        crate::midi::midi::fluid_player_settings(self);
        self.fluid_file_renderer_settings();
        self.fluid_audio_driver_settings();
        self.fluid_midi_driver_settings();
    }

    /// Translation of `fluid_shell_settings()` (fluid_cmd.c).
    fn fluid_shell_settings(&mut self) {
        self.register_str("shell.prompt", Some(""), 0);
        self.register_int("shell.port", 0, 0, 65535, 0);
    }

    /// Translation of `fluid_file_renderer_settings()`
    /// (fluid_filerenderer.c, without libsndfile).
    fn fluid_file_renderer_settings(&mut self) {
        self.register_str("audio.file.name", Some("fluidsynth.raw"), 0);
        self.register_str("audio.file.type", Some("raw"), 0);
        self.add_option("audio.file.type", "raw");
        self.register_str("audio.file.format", Some("s16"), 0);
        self.add_option("audio.file.format", "s16");
        self.register_str("audio.file.endian", Some("cpu"), 0);
        self.add_option("audio.file.endian", "cpu");
    }

    /// Translation of `fluid_audio_driver_settings()` (fluid_adriver.c,
    /// with no drivers).
    fn fluid_audio_driver_settings(&mut self) {
        self.register_str("audio.sample-format", Some("16bits"), 0);
        self.add_option("audio.sample-format", "16bits");
        self.add_option("audio.sample-format", "float");

        if cfg!(windows) {
            self.register_int("audio.period-size", 512, 64, 8192, 0);
            self.register_int("audio.periods", 8, 2, 64, 0);
        } else {
            self.register_int("audio.period-size", 64, 64, 8192, 0);
            self.register_int("audio.periods", 16, 2, 64, 0);
        }

        self.register_int(
            "audio.realtime-prio",
            60, /* FLUID_DEFAULT_AUDIO_RT_PRIO */
            0,
            99,
            0,
        );

        self.register_str("audio.driver", Some(""), 0);

        /* (no drivers to add as options or to select as the default) */
    }

    /// Translation of `fluid_midi_driver_settings()` (fluid_mdriver.c,
    /// with no drivers).
    fn fluid_midi_driver_settings(&mut self) {
        self.register_int("midi.autoconnect", 0, 0, 1, FLUID_HINT_TOGGLED);

        self.register_int(
            "midi.realtime-prio",
            50, /* FLUID_DEFAULT_MIDI_RT_PRIO */
            0,
            99,
            0,
        );

        self.register_str("midi.driver", Some(""), 0);

        /* (no drivers to add as options or to select as the default) */
    }

    /// Translation of `fluid_settings_get()`: get a setting name, value and
    /// type.
    fn get(&self, name: &str) -> Option<&FluidSettingNode> {
        let tokens = fluid_settings_tokenize(name);
        let (last, prefix) = tokens.split_last()?;

        let mut table = &self.table;
        for tok in prefix {
            match table.get(*tok)? {
                FluidSettingNode::Set(t) => table = t,
                _ => return None,
            }
        }

        table.get(*last)
    }

    /// `fluid_settings_get()` for changing the node.
    fn get_mut(&mut self, name: &str) -> Option<&mut FluidSettingNode> {
        let tokens = fluid_settings_tokenize(name);
        let (last, prefix) = tokens.split_last()?;

        let mut table = &mut self.table;
        for tok in prefix {
            match table.get_mut(*tok)? {
                FluidSettingNode::Set(t) => table = t,
                _ => return None,
            }
        }

        table.get_mut(*last)
    }

    /// Translation of `fluid_settings_set()`: set a setting name, value and
    /// type, replacing it if already exists.
    fn set(&mut self, name: &str, value: FluidSettingNode) -> i32 {
        let tokens = fluid_settings_tokenize(name);

        if tokens.is_empty() {
            return FLUID_FAILED;
        }

        let num = tokens.len() - 1;
        let mut table = &mut self.table;

        for tok in &tokens[..num] {
            let node = table
                .entry(tok.to_string())
                .or_insert_with(|| FluidSettingNode::Set(BTreeMap::new()));

            match node {
                FluidSettingNode::Set(t) => table = t,
                _ => {
                    /* path ends prematurely */
                    flog!(
                        FLUID_ERR,
                        "'{}' is not a node. Name of the setting was '{}'",
                        tok,
                        name
                    );
                    return FLUID_FAILED;
                }
            }
        }

        table.insert(tokens[num].to_string(), value);

        FLUID_OK
    }

    /// Translation of `fluid_settings_register_str()`: registers a new
    /// string value for the specified setting.
    pub(crate) fn register_str(&mut self, name: &str, def: Option<&str>, hints: i32) -> i32 {
        if name.is_empty() {
            return FLUID_FAILED;
        }

        match self.get_mut(name) {
            None => {
                let node = FluidSettingNode::Str(FluidStrSetting {
                    value: def.map(str::to_string),
                    def: def.map(str::to_string),
                    hints,
                    options: Vec::new(),
                });
                self.set(name, node)
            }
            Some(FluidSettingNode::Str(setting)) => {
                /* if variable already exists, don't change its value. */
                setting.def = def.map(str::to_string);
                setting.hints = hints;
                FLUID_OK
            }
            Some(_) => {
                flog!(
                    FLUID_ERR,
                    "Failed to register string setting '{}' as it already exists with a different type",
                    name
                );
                FLUID_FAILED
            }
        }
    }

    /// Translation of `fluid_settings_register_num()`: registers a new
    /// float value for the specified setting.
    pub(crate) fn register_num(
        &mut self,
        name: &str,
        def: f64,
        min: f64,
        max: f64,
        mut hints: i32,
    ) -> i32 {
        if name.is_empty() {
            return FLUID_FAILED;
        }

        /* For now, all floating point settings are bounded below and above */
        hints |= FLUID_HINT_BOUNDED_BELOW | FLUID_HINT_BOUNDED_ABOVE;

        match self.get_mut(name) {
            None => {
                /* insert a new setting */
                let node = FluidSettingNode::Num(FluidNumSetting {
                    value: def,
                    def,
                    min,
                    max,
                    hints,
                });
                self.set(name, node)
            }
            Some(FluidSettingNode::Num(setting)) => {
                /* update the existing setting but don't change its value */
                setting.min = min;
                setting.max = max;
                setting.def = def;
                setting.hints = hints;
                FLUID_OK
            }
            Some(_) => {
                /* type mismatch */
                flog!(
                    FLUID_ERR,
                    "Failed to register numeric setting '{}' as it already exists with a different type",
                    name
                );
                FLUID_FAILED
            }
        }
    }

    /// Translation of `fluid_settings_register_int()`: registers a new
    /// integer value for the specified setting.
    pub(crate) fn register_int(
        &mut self,
        name: &str,
        def: i32,
        min: i32,
        max: i32,
        mut hints: i32,
    ) -> i32 {
        if name.is_empty() {
            return FLUID_FAILED;
        }

        /* For now, all integer settings are bounded below and above */
        hints |= FLUID_HINT_BOUNDED_BELOW | FLUID_HINT_BOUNDED_ABOVE;

        match self.get_mut(name) {
            None => {
                /* insert a new setting */
                let node = FluidSettingNode::Int(FluidIntSetting {
                    value: def,
                    def,
                    min,
                    max,
                    hints,
                });
                self.set(name, node)
            }
            Some(FluidSettingNode::Int(setting)) => {
                /* update the existing setting but don't change its value */
                setting.min = min;
                setting.max = max;
                setting.def = def;
                setting.hints = hints;
                FLUID_OK
            }
            Some(_) => {
                /* type mismatch */
                flog!(
                    FLUID_ERR,
                    "Failed to register int setting '{}' as it already exists with a different type",
                    name
                );
                FLUID_FAILED
            }
        }
    }

    /// Translation of `fluid_settings_add_option()`: add an option to a
    /// string setting (like an enumeration value).
    pub(crate) fn add_option(&mut self, name: &str, s: &str) -> i32 {
        match self.get_mut(name) {
            Some(FluidSettingNode::Str(setting)) => {
                setting.options.push(s.to_string());
                setting.hints |= FLUID_HINT_OPTIONLIST;
                FLUID_OK
            }
            _ => FLUID_FAILED,
        }
    }
}

impl Settings {
    /// Translation of `new_fluid_settings()`: create a new settings object.
    pub fn new() -> Settings {
        let mut settings = SettingsInner::default();
        settings.fluid_settings_init();
        Settings {
            inner: Arc::new(Mutex::new(settings)),
        }
    }

    /// The locked settings tree.
    fn lock(&self) -> MutexGuard<'_, SettingsInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Translation of `fluid_settings_get_type()`: get the type of the
    /// setting with the given name.
    pub fn get_type(&self, name: &str) -> FluidType {
        if name.is_empty() {
            return FluidType::NoType;
        }

        self.lock()
            .get(name)
            .map_or(FluidType::NoType, FluidSettingNode::type_)
    }

    /// Translation of `fluid_settings_setstr()`: set a string value for a
    /// named setting (None for a NULL string).
    pub fn setstr(&self, name: &str, str_: Option<&str>) -> i32 {
        if name.is_empty() {
            return FLUID_FAILED;
        }

        let mut settings = self.lock();

        match settings.get_mut(name) {
            Some(FluidSettingNode::Str(setting)) => {
                setting.value = str_.map(str::to_string);
                FLUID_OK
            }
            _ => {
                flog!(FLUID_ERR, "Unknown string setting '{}'", name);
                FLUID_FAILED
            }
        }
    }

    /// Translation of `fluid_settings_dupstr()`: duplicate the value of a
    /// string setting (or of a boolean integer setting, as "yes" or "no").
    /// Returns None if the setting doesn't exist or has no value
    /// (FLUID_FAILED, or FLUID_OK with a NULL string upstream).
    pub fn dupstr(&self, name: &str) -> Option<String> {
        if name.is_empty() {
            return None;
        }

        let settings = self.lock();

        match settings.get(name) {
            Some(FluidSettingNode::Str(setting)) => setting.value.clone(),
            Some(FluidSettingNode::Int(setting)) if setting.hints & FLUID_HINT_TOGGLED != 0 => {
                Some(if setting.value != 0 { "yes" } else { "no" }.to_string())
            }
            _ => None,
        }
    }

    /// Translation of `fluid_settings_str_equal()`: test a string setting
    /// for some value.
    pub fn str_equal(&self, name: &str, s: &str) -> bool {
        if name.is_empty() {
            return false;
        }

        let settings = self.lock();

        match settings.get(name) {
            Some(FluidSettingNode::Str(setting)) => setting.value.as_deref() == Some(s),
            Some(FluidSettingNode::Int(setting)) if setting.hints & FLUID_HINT_TOGGLED != 0 => {
                (if setting.value != 0 { "yes" } else { "no" }) == s
            }
            _ => false,
        }
    }

    /// Translation of `fluid_settings_setnum()`: set a numeric value for a
    /// named setting.
    pub fn setnum(&self, name: &str, val: f64) -> i32 {
        if name.is_empty() {
            return FLUID_FAILED;
        }

        let mut settings = self.lock();

        match settings.get_mut(name) {
            Some(FluidSettingNode::Num(setting)) => {
                if val < setting.min || val > setting.max {
                    flog!(FLUID_ERR, "requested set value for '{}' out of range", name);
                    return FLUID_FAILED;
                }

                setting.value = val;
                FLUID_OK
            }
            _ => {
                flog!(FLUID_ERR, "Unknown numeric setting '{}'", name);
                FLUID_FAILED
            }
        }
    }

    /// Translation of `fluid_settings_getnum()`: get the numeric value of a
    /// named setting.
    pub fn getnum(&self, name: &str, val: &mut f64) -> i32 {
        if name.is_empty() {
            return FLUID_FAILED;
        }

        match self.lock().get(name) {
            Some(FluidSettingNode::Num(setting)) => {
                *val = setting.value;
                FLUID_OK
            }
            _ => FLUID_FAILED,
        }
    }

    /// Translation of `fluid_settings_getnum_float()`: float-typed wrapper
    /// for fluid_settings_getnum.
    pub fn getnum_float(&self, name: &str, val: &mut f32) -> i32 {
        let mut tmp = 0.0;

        if self.getnum(name, &mut tmp) == FLUID_OK {
            *val = tmp as f32;
            return FLUID_OK;
        }

        FLUID_FAILED
    }

    /// Translation of `fluid_settings_getnum_range()`: get the range of
    /// values of a numeric setting.
    pub fn getnum_range(&self, name: &str, min: &mut f64, max: &mut f64) -> i32 {
        if name.is_empty() {
            return FLUID_FAILED;
        }

        match self.lock().get(name) {
            Some(FluidSettingNode::Num(setting)) => {
                *min = setting.min;
                *max = setting.max;
                FLUID_OK
            }
            _ => FLUID_FAILED,
        }
    }

    /// Translation of `fluid_settings_setint()`: set an integer value for
    /// a setting.
    pub fn setint(&self, name: &str, val: i32) -> i32 {
        if name.is_empty() {
            return FLUID_FAILED;
        }

        let mut settings = self.lock();

        match settings.get_mut(name) {
            Some(FluidSettingNode::Int(setting)) => {
                if val < setting.min || val > setting.max {
                    flog!(
                        FLUID_ERR,
                        "requested set value for setting '{}' out of range",
                        name
                    );
                    return FLUID_FAILED;
                }

                setting.value = val;
                FLUID_OK
            }
            _ => {
                flog!(FLUID_ERR, "Unknown integer parameter '{}'", name);
                FLUID_FAILED
            }
        }
    }

    /// Translation of `fluid_settings_getint()`: get an integer value
    /// setting.
    pub fn getint(&self, name: &str, val: &mut i32) -> i32 {
        if name.is_empty() {
            return FLUID_FAILED;
        }

        match self.lock().get(name) {
            Some(FluidSettingNode::Int(setting)) => {
                *val = setting.value;
                FLUID_OK
            }
            _ => FLUID_FAILED,
        }
    }

    /// Translation of `fluid_settings_getint_range()`: get the range of
    /// values of an integer setting.
    pub fn getint_range(&self, name: &str, min: &mut i32, max: &mut i32) -> i32 {
        if name.is_empty() {
            return FLUID_FAILED;
        }

        match self.lock().get(name) {
            Some(FluidSettingNode::Int(setting)) => {
                *min = setting.min;
                *max = setting.max;
                FLUID_OK
            }
            _ => FLUID_FAILED,
        }
    }
}

/// C's `atoi()` (without its undefined behaviour on overflow, which
/// wraps here).
fn atoi(s: &str) -> i32 {
    let s = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut n: i32 = 0;
    for c in digits.bytes() {
        if !c.is_ascii_digit() {
            break;
        }
        n = n.wrapping_mul(10).wrapping_add((c - b'0') as i32);
    }
    if neg {
        n.wrapping_neg()
    } else {
        n
    }
}

/// Translation of `fluid_settings_split_csv()`: split a comma-separated
/// list of integers and fill the passed in buffer with the parsed values.
///
/// @param str the comma-separated string to split
/// @param buf user-supplied buffer to hold the parsed numbers
/// @return number of parsed values or -1 on failure
pub(crate) fn fluid_settings_split_csv(str_: &str, buf: &mut [i32]) -> i32 {
    let mut n = 0;

    for tok in fluid_strtok(str_, ',') {
        if n >= buf.len() {
            break;
        }
        buf[n] = atoi(tok);
        n += 1;
    }

    n as i32
}
