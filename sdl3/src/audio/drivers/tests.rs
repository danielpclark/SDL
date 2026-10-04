// Tests for the driver list and the bootstrap's fall-through, which have to
// pass whether or not this machine has a sound server, a sound card or the
// system libraries the platform drivers load.

use crate::audio::{audio_driver, current_audio_driver, num_audio_drivers};
use crate::init::InitFlags;

/// The deduplicated driver list, in upstream's `bootstrap[]` order.
pub(crate) const EXPECTED_DRIVERS: &[&str] = &[
    #[cfg(target_os = "linux")]
    "pipewire",
    #[cfg(target_os = "linux")]
    "pulseaudio",
    #[cfg(target_os = "linux")]
    "alsa",
    #[cfg(windows)]
    "wasapi",
    "disk",
    "dummy",
];

/// The drivers that initialize without being asked for by name.
const NON_DEMAND_ONLY: &[&str] = &[
    #[cfg(target_os = "linux")]
    "pipewire",
    #[cfg(target_os = "linux")]
    "pulseaudio",
    #[cfg(target_os = "linux")]
    "alsa",
    #[cfg(windows)]
    "wasapi",
];

/// Shuts audio down (and resets the driver hints) when dropped, so a
/// failing driver test doesn't leave the subsystem up for the next one.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // (used by the platform drivers' tests)
pub(crate) struct QuitAudioOnDrop;

impl Drop for QuitAudioOnDrop {
    fn drop(&mut self) {
        for _ in 0..8 {
            if current_audio_driver().is_none() {
                break;
            }
            crate::init::quit_subsystem(InitFlags::AUDIO);
        }
        for h in [
            crate::hints::AUDIO_DRIVER,
            crate::hints::AUDIO_INCLUDE_MONITORS,
            crate::hints::AUDIO_ALSA_DEFAULT_PLAYBACK_DEVICE,
            crate::hints::AUDIO_ALSA_DEFAULT_RECORDING_DEVICE,
        ] {
            crate::hints::reset(h);
        }
    }
}

/// Initialize audio with `hint` as the driver hint, returning the driver
/// that came up (and shutting it down again).
fn init_with(hint: Option<&str>) -> Option<&'static str> {
    crate::hints::reset(crate::hints::AUDIO_DRIVER);
    if let Some(h) = hint {
        crate::hints::set(crate::hints::AUDIO_DRIVER, h).unwrap();
    }
    let r = crate::init::init_subsystem(InitFlags::AUDIO);
    let name = current_audio_driver();
    assert_eq!(r.is_ok(), name.is_some());
    if r.is_ok() {
        crate::init::quit_subsystem(InitFlags::AUDIO);
    }
    crate::hints::reset(crate::hints::AUDIO_DRIVER);
    assert!(current_audio_driver().is_none());
    name
}

#[test]
fn driver_list_is_in_upstream_order() {
    let drivers: Vec<&str> = (0..num_audio_drivers())
        .map(|i| audio_driver(i).unwrap())
        .collect();
    assert_eq!(drivers, EXPECTED_DRIVERS);
}

#[test]
fn bootstrap_falls_through_to_the_next_driver() {
    let _l = crate::test_support::test_lock();

    // Which drivers work here, each asked for by name.
    let works: Vec<&str> = EXPECTED_DRIVERS
        .iter()
        .copied()
        .filter(|&d| init_with(Some(d)) == Some(d))
        .collect();
    eprintln!("note: audio drivers that initialize here: {works:?}");
    assert!(works.contains(&"dummy") && works.contains(&"disk"));

    // A list in the hint: the first one that initializes wins, so failing
    // drivers fall through to the next, and finally to dummy.
    let mut list: Vec<&str> = NON_DEMAND_ONLY.to_vec();
    list.push("dummy");
    let expected = list.iter().copied().find(|d| works.contains(d));
    assert_eq!(init_with(Some(&list.join(","))), expected);
    assert_eq!(init_with(Some("nosuch,dummy")), Some("dummy"));
    assert_eq!(init_with(Some("nosuch")), None);

    // No hint: the first non-demand-only driver that initializes, in
    // bootstrap order, or nothing. (PipeWire comes first in its "preferred"
    // form, which also gives up when the server has no session manager
    // with audio, or no devices; then it's tried again after PulseAudio.)
    let mut working = NON_DEMAND_ONLY
        .iter()
        .copied()
        .filter(|d| works.contains(d));
    let expected = working.next();
    let got = init_with(None);
    if got != expected {
        assert_eq!(
            expected,
            Some("pipewire"),
            "only PipeWire's preferred form can step aside"
        );
        assert_eq!(got, working.next());
    }
}
