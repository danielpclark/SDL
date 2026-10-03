// Rust translation of src/joystick/SDL_steam_virtual_gamepad.c and
// SDL_steam_virtual_gamepad.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Steam virtual gamepad information: when a game runs under Steam
//! Input, Steam names a file (in the `SteamVirtualGamepadInfo` environment
//! variable) describing the real controller behind each virtual gamepad
//! slot.

use std::cell::RefCell;

use super::assert_joysticks_locked;
use super::gamepad::GamepadType;
use crate::thread::ReentrantMutex;

/// Translation of `SDL_SteamVirtualGamepadInfo`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SteamVirtualGamepadInfo {
    pub(crate) handle: u64,
    pub(crate) name: Option<String>,
    pub(crate) vendor_id: u16,
    pub(crate) product_id: u16,
    pub(crate) gamepad_type: GamepadType,
}

/// The state guarded by `SDL_event_lock` upstream.
struct State {
    /// Translation of `SDL_steam_virtual_gamepad_info_file`.
    file: Option<String>,
    /// Translation of `SDL_steam_virtual_gamepad_info_file_mtime`.
    file_mtime: u64,
    /// Translation of `SDL_steam_virtual_gamepad_info_check_time`.
    check_time: u64,
    /// Translation of `SDL_steam_virtual_gamepad_info` (`None` until loaded).
    info: Option<Vec<Option<SteamVirtualGamepadInfo>>>,
}

static STATE: ReentrantMutex<RefCell<State>> = ReentrantMutex::new(RefCell::new(State {
    file: None,
    file_mtime: 0,
    check_time: 0,
    info: None,
}));

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let guard = STATE.lock();
    let mut state = guard.borrow_mut();
    f(&mut state)
}

/// Translation of `GetFileModificationTime()`.
fn file_modification_time(file: &str) -> u64 {
    let Ok(metadata) = std::fs::metadata(file) else {
        return 0;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.mtime() as u64
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.last_write_time()
    }
    #[cfg(not(any(unix, windows)))]
    {
        metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs())
    }
}

/// Translation of `SDL_FreeSteamVirtualGamepadInfo()`.
fn free_steam_virtual_gamepad_info(state: &mut State) {
    assert_joysticks_locked();

    state.info = None;
}

/// Translation of `AddVirtualGamepadInfo()`.
fn add_virtual_gamepad_info(state: &mut State, slot: i32, info: &mut SteamVirtualGamepadInfo) {
    assert_joysticks_locked();

    let Ok(slot) = usize::try_from(slot) else {
        return;
    };

    let slots = state.info.get_or_insert_with(Vec::new);
    if slot >= slots.len() {
        slots.resize(slot + 1, None);
    }

    if slots[slot].is_some() {
        // We already have this slot info
        return;
    }

    slots[slot] = Some(std::mem::take(info));
}

/// Translation of `SDL_InitSteamVirtualGamepadInfo()`.
pub(super) fn init_steam_virtual_gamepad_info() {
    assert_joysticks_locked();

    // The file isn't available inside the macOS sandbox
    if crate::init::sandbox() == crate::init::Sandbox::MacOS {
        return;
    }

    if let Some(file) =
        crate::stdlib::getenv_unsafe("SteamVirtualGamepadInfo").filter(|f| !f.is_empty())
    {
        #[cfg(target_os = "linux")]
        {
            // Older versions of Wine will blacklist the Steam Virtual Gamepad if
            // it appears to have the real controller's VID/PID, so ignore this.
            if crate::filesystem::get_exe_name().is_ok_and(|exe| exe == "wine64-preloader") {
                crate::log::debug!(
                    crate::log::Category::Input,
                    "Wine launched by Steam, ignoring SteamVirtualGamepadInfo"
                );
                return;
            }
        }
        with_state(|s| s.file = Some(file));
    }
    update_steam_virtual_gamepad_info();
}

/// Translation of `SDL_SteamVirtualGamepadEnabled()`.
pub(super) fn steam_virtual_gamepad_enabled() -> bool {
    assert_joysticks_locked();

    with_state(|s| s.info.is_some())
}

/// Parse `[slot %d]` the way `SDL_sscanf()` does: `Some(slot)` if the
/// number matched.
fn parse_slot(line: &str) -> Option<i32> {
    let rest = line.strip_prefix("[slot")?;
    let (value, used) = crate::stdlib::string::strtol(rest, 10);
    (used > 0).then_some(value as i32)
}

/// Reload the file if it changed (checked at most every 3 seconds); returns
/// whether it was reloaded. Translation of `SDL_UpdateSteamVirtualGamepadInfo()`.
pub(super) fn update_steam_virtual_gamepad_info() -> bool {
    const UPDATE_CHECK_INTERVAL_MS: u64 = 3000;

    assert_joysticks_locked();

    let Some(file) = with_state(|s| s.file.clone()) else {
        return false;
    };

    let now = crate::timer::ticks_ms();
    let check_time = with_state(|s| s.check_time);
    if check_time != 0 && now < check_time + UPDATE_CHECK_INTERVAL_MS {
        return false;
    }
    with_state(|s| s.check_time = now);

    let mtime = file_modification_time(&file);
    if mtime == 0 || mtime == with_state(|s| s.file_mtime) {
        return false;
    }

    let Ok(data) = crate::io::load_file(&file) else {
        return false;
    };

    with_state(|state| {
        free_steam_virtual_gamepad_info(state);

        let mut slot = -1;
        let mut info = SteamVirtualGamepadInfo::default();

        for line in data.split(|&b| b == b'\r' || b == b'\n') {
            // (each line is read as a C string)
            let line = &line[..line.iter().position(|&b| b == 0).unwrap_or(line.len())];
            if line.is_empty() {
                continue;
            }
            let line = String::from_utf8_lossy(line);

            if let Some(new_slot) = parse_slot(&line) {
                if slot >= 0 {
                    add_virtual_gamepad_info(state, slot, &mut info);
                }
                slot = new_slot;
            } else if let Some((key, value)) = line.split_once('=') {
                match key {
                    "name" => info.name = Some(value.to_string()),
                    "VID" => info.vendor_id = crate::stdlib::string::strtoul(value, 0).0 as u16,
                    "PID" => info.product_id = crate::stdlib::string::strtoul(value, 0).0 as u16,
                    "type" => info.gamepad_type = GamepadType::from_string(value),
                    "handle" => info.handle = crate::stdlib::string::strtoull(value, 0).0,
                    _ => {}
                }
            }
        }
        if slot >= 0 {
            add_virtual_gamepad_info(state, slot, &mut info);
        }

        state.file_mtime = mtime;
    });

    true
}

/// The information for a slot. Translation of `SDL_GetSteamVirtualGamepadInfo()`.
pub(super) fn steam_virtual_gamepad_info(slot: i32) -> Option<SteamVirtualGamepadInfo> {
    assert_joysticks_locked();

    let slot = usize::try_from(slot).ok()?;
    with_state(|s| s.info.as_ref()?.get(slot)?.clone())
}

/// Translation of `SDL_QuitSteamVirtualGamepadInfo()`.
pub(super) fn quit_steam_virtual_gamepad_info() {
    assert_joysticks_locked();

    with_state(|s| {
        if s.file.is_some() {
            free_steam_virtual_gamepad_info(s);
            s.file = None;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_lines() {
        assert_eq!(parse_slot("[slot 3]"), Some(3));
        assert_eq!(parse_slot("[slot   -2"), Some(-2));
        assert_eq!(parse_slot("[slot]"), None);
        assert_eq!(parse_slot("slot 1"), None);
    }
}
