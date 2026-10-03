// Rust translation of the SDL_vidpid_list functions of
// src/joystick/SDL_joystick.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Lists of USB vendor/product IDs, built from a list compiled into SDL and
//! extended (or trimmed) by a pair of hints. Translation of `SDL_vidpid_list`.

use std::sync::{Mutex, MutexGuard};

use super::tables::make_vidpid;
use crate::hints;

/// A VID/PID list. Translation of `SDL_vidpid_list`.
pub(crate) struct VidPidList {
    included_hint_name: Option<&'static str>,
    excluded_hint_name: Option<&'static str>,
    initial_entries: &'static [u32],
    state: Mutex<VidPidState>,
}

#[derive(Default)]
struct VidPidState {
    included_entries: Vec<u32>,
    excluded_entries: Vec<u32>,
    initialized: bool,
    watches: Vec<hints::Callback>,
}

impl VidPidList {
    /// A list with the hints that extend it and the entries compiled in.
    pub(crate) const fn new(
        included_hint_name: Option<&'static str>,
        excluded_hint_name: Option<&'static str>,
        initial_entries: &'static [u32],
    ) -> VidPidList {
        VidPidList {
            included_hint_name,
            excluded_hint_name,
            initial_entries,
            state: Mutex::new(VidPidState {
                included_entries: Vec::new(),
                excluded_entries: Vec::new(),
                initialized: false,
                watches: Vec::new(),
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, VidPidState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Rebuild the list from the initial entries and the hint values.
    /// Translation of `SDL_LoadVIDPIDListFromHints()`.
    pub(crate) fn load_from_hints(&self, included_list: Option<&str>, excluded_list: Option<&str>) {
        let mut state = self.state();
        load_from_hints(
            &mut state,
            self.initial_entries,
            included_list,
            excluded_list,
        );
    }

    /// Watch the list's hints and load it. Translation of `SDL_LoadVIDPIDList()`.
    pub(crate) fn load(&'static self) {
        let mut watches = Vec::new();
        if let Some(name) = self.included_hint_name {
            // Translation of `SDL_VIDPIDIncludedHintChanged()`.
            let watch = hints::watch(name, move |change| {
                if !self.state().initialized {
                    return;
                }
                let included_list = change.new_value;
                let excluded_list = self.excluded_hint_name.and_then(hints::get);
                self.load_from_hints(included_list, excluded_list.as_deref());
            });
            if let Ok(watch) = watch {
                watches.push(watch);
            }
        }

        if let Some(name) = self.excluded_hint_name {
            // Translation of `SDL_VIDPIDExcludedHintChanged()`.
            let watch = hints::watch(name, move |change| {
                if !self.state().initialized {
                    return;
                }
                let included_list = self.included_hint_name.and_then(hints::get);
                let excluded_list = change.new_value;
                self.load_from_hints(included_list.as_deref(), excluded_list);
            });
            if let Ok(watch) = watch {
                watches.push(watch);
            }
        }

        {
            let mut state = self.state();
            state.watches.extend(watches);
            state.initialized = true;
        }

        let hint_or_env = |name: Option<&str>| {
            let name = name?;
            hints::get(name).or_else(|| crate::stdlib::getenv_unsafe(name))
        };
        let included_list = hint_or_env(self.included_hint_name);
        let excluded_list = hint_or_env(self.excluded_hint_name);
        self.load_from_hints(included_list.as_deref(), excluded_list.as_deref());
    }

    /// Whether a device is in the list. Translation of `SDL_VIDPIDInList()`.
    pub(crate) fn contains(&self, vendor_id: u16, product_id: u16) -> bool {
        vidpid_in_list(&self.state(), vendor_id, product_id)
    }

    /// Whether the list has any included entries (`num_included_entries > 0`).
    pub(crate) fn has_included_entries(&self) -> bool {
        !self.state().included_entries.is_empty()
    }

    /// Stop watching the hints and empty the list. Translation of `SDL_FreeVIDPIDList()`.
    pub(crate) fn free(&self) {
        let watches = {
            let mut state = self.state();
            state.included_entries = Vec::new();
            state.excluded_entries = Vec::new();
            state.initialized = false;
            std::mem::take(&mut state.watches)
        };
        // (dropped without the list locked: removing a watch waits for the hint lock)
        drop(watches);
    }
}

/// A one-off list of the devices named by a hint value (as `SDL_HINT_GAMECONTROLLER_SENSOR_FUSION`
/// uses), with no initial entries and no exclusions.
pub(crate) fn vidpid_list_from_hint(hint: &str) -> impl Fn(u16, u16) -> bool {
    let mut state = VidPidState::default();
    load_from_hints(&mut state, &[], Some(hint), None);
    move |vendor_id, product_id| vidpid_in_list(&state, vendor_id, product_id)
}

fn load_from_hints(
    state: &mut VidPidState,
    initial: &[u32],
    included_list: Option<&str>,
    excluded_list: Option<&str>,
) {
    // Empty the list
    state.included_entries.clear();
    state.excluded_entries.clear();

    // Add the initial entries
    state.included_entries.extend_from_slice(initial);

    // Add the included entries from the hint
    load_vidpid_list_from_hint(included_list, &mut state.included_entries);

    // Add the excluded entries from the hint
    load_vidpid_list_from_hint(excluded_list, &mut state.excluded_entries);
}

fn vidpid_in_list(state: &VidPidState, vendor_id: u16, product_id: u16) -> bool {
    let vidpid = make_vidpid(vendor_id, product_id);

    if state.excluded_entries.contains(&vidpid) {
        return false;
    }
    state.included_entries.contains(&vidpid)
}

/// Parse `0xVVVV/0xPPPP` pairs (any separators) from a hint value, or from
/// the file it names with a leading `@`. Translation of `SDL_LoadVIDPIDListFromHint()`.
fn load_vidpid_list_from_hint(hint: Option<&str>, entries: &mut Vec<u32>) {
    let Some(hint) = hint else {
        return;
    };
    let file;
    let text: &[u8] = if let Some(path) = hint.strip_prefix('@') {
        match crate::io::load_file(path) {
            Ok(data) => {
                file = data;
                // (the file is read as a C string)
                let end = file.iter().position(|&b| b == 0).unwrap_or(file.len());
                &file[..end]
            }
            Err(_) => return,
        }
    } else {
        hint.as_bytes()
    };

    let find = |from: usize| {
        text[from..]
            .windows(2)
            .position(|w| w == b"0x")
            .map(|i| from + i)
    };
    let mut spot = 0;
    while let Some(start) = find(spot) {
        let (vendor, used) = crate::stdlib::string::strtol(&text[start..], 0);
        let mut entry = (vendor as u16 as u32) << 16;
        spot = start + used.max(1);
        let Some(start) = find(spot) else {
            break;
        };
        let (product, used) = crate::stdlib::string::strtol(&text[start..], 0);
        entry |= product as u16 as u32;
        spot = start + used.max(1);

        entries.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hint_entries() {
        let mut entries = Vec::new();
        load_vidpid_list_from_hint(Some("0x045e/0x028e, 0x054C/0x0268 0x1234"), &mut entries);
        assert_eq!(entries, vec![0x045e_028e, 0x054c_0268]);

        let in_list = vidpid_list_from_hint("0x0001/0x0002");
        assert!(in_list(1, 2));
        assert!(!in_list(2, 1));
    }
}
