// Tests of the Android haptic driver (the program spawned on the device:
// no Java VM, so the haptic handler finds nothing and plays nothing; in an
// app: sdl3/tests/android_joystick.rs).
//
// The vibrator is added and removed as the haptic handler's natives would
// do it (`nativeAddHaptic()`, `nativeRemoveHaptic()`).

use super::super::{haptic_name_for_id, haptics, Haptic, HapticLeftRight};
use super::*;
use crate::init::{self, InitFlags};

#[test]
fn a_vibrator_from_the_haptic_handler() {
    let _l = crate::test_support::test_lock();
    init::init_subsystem(InitFlags::HAPTIC).unwrap();
    let before = haptics();

    assert!(add_haptic(4243, "VIBRATOR_SERVICE"));
    let list = haptics();
    assert_eq!(list.len(), before.len() + 1);
    let id = *list.last().unwrap();
    assert_eq!(
        haptic_name_for_id(id).unwrap().as_deref(),
        Some("VIBRATOR_SERVICE")
    );

    let haptic = Haptic::open(id).unwrap();
    assert_eq!(haptic.name().unwrap().as_deref(), Some("VIBRATOR_SERVICE"));
    assert_eq!(haptic.features().unwrap(), HapticFeatures::LEFTRIGHT);
    assert_eq!(haptic.max_effects().unwrap(), 1);
    assert_eq!(haptic.max_effects_playing().unwrap(), 1);
    assert_eq!(haptic.num_axes().unwrap(), 0);
    assert!(haptic.rumble_supported());

    // The effect is never allocated: each one takes the first slot
    let effect = HapticEffect::LeftRight(HapticLeftRight {
        length: 100,
        large_magnitude: 0x8000,
        small_magnitude: 0x4000,
    });
    assert_eq!(haptic.create_effect(&effect).unwrap(), 0);
    assert_eq!(haptic.create_effect(&effect).unwrap(), 0);
    assert!(haptic.run_effect(0, 1).is_ok());
    assert_eq!(haptic.effect_status(0).ok(), None);
    assert!(haptic.stop_effect(0).is_ok());
    haptic.destroy_effect(0);
    assert!(haptic.init_rumble().is_ok());
    assert!(haptic.play_rumble(0.5, 100).is_ok());
    assert!(haptic.stop_rumble().is_ok());

    // Removed while open: it was
    assert!(remove_haptic(4243));
    assert!(!remove_haptic(4243));
    assert_eq!(haptics(), before);
    drop(haptic);

    // Added again: closed, a new instance
    assert!(add_haptic(4243, "VIBRATOR_SERVICE"));
    let again = *haptics().last().unwrap();
    assert_ne!(again, id);
    assert!(!remove_haptic(4243));

    init::quit_subsystem(InitFlags::HAPTIC);
}
