// Tests for the Logitech wheel HIDAPI haptic driver.
// Copyright (C) 2025 Katharine Chui <katharine.chui@gmail.com>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `tests/lg4ff.txt` comes from running upstream's
//! SDL_hidapihaptic_lg4ff.c and SDL_hidapihaptic.c, with stubs for the SDL
//! functions they use (a joystick printing the effects sent to it, failing
//! them on request, as [`FakeJoystick`] does; a clock the script sets; no
//! effect thread, the script runs `lg4ff_timer()`) and SDL's own libm, on
//! scripted and generated operations.
//!
//! Each `case` line gives the wheel (vendor, product and version) and the
//! `SDL_HAPTIC_LG4FF_*` environment variables (`-` for unset); each `op`
//! line the clock and a driver call, followed by what it printed up to
//! `end`. The `frontend` part has calls of the `SDL_HIDAPI_*` functions,
//! which the HIDAPI layer's tests replay with a joystick.

use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::haptic::HapticLeftRight;

const TRANSCRIPT: &str = include_str!("tests/lg4ff.txt");

/// The environment variables of a case, in order.
const ENV_NAMES: [&str; 4] = [
    "SDL_HAPTIC_LG4FF_SPRING",
    "SDL_HAPTIC_LG4FF_DAMPER",
    "SDL_HAPTIC_LG4FF_FRICTION",
    "SDL_HAPTIC_LG4FF_GAIN",
];

/// The clock the script sets (`SDL_GetTicks()`).
static NOW: AtomicU64 = AtomicU64::new(0);

fn fake_now() -> u64 {
    NOW.load(Ordering::Relaxed)
}

/// `tag` and the bytes in hex, as the transcript prints them.
fn hex_line(tag: &str, data: &[u8]) -> String {
    let mut line = tag.to_owned();
    for byte in data {
        line.push_str(&format!(" {byte:02x}"));
    }
    line
}

/// The joystick of the transcript's stubs: it logs the effects sent to
/// it, failing `fail` of them after letting `skip` through.
pub(in crate::haptic::hidapi) struct FakeJoystick {
    pub(in crate::haptic::hidapi) vendor: u16,
    pub(in crate::haptic::hidapi) product: u16,
    pub(in crate::haptic::hidapi) version: u16,
    log: Mutex<Vec<String>>,
    /// (skip, fail)
    failures: Mutex<(u32, u32)>,
}

impl FakeJoystick {
    pub(in crate::haptic::hidapi) fn new(vendor: u16, product: u16, version: u16) -> FakeJoystick {
        FakeJoystick {
            vendor,
            product,
            version,
            log: Mutex::new(Vec::new()),
            failures: Mutex::new((0, 0)),
        }
    }

    fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.lock().unwrap())
    }

    fn fail(&self, skip: u32, fail: u32) {
        *self.failures.lock().unwrap() = (skip, fail);
    }
}

impl HapticJoystick for FakeJoystick {
    fn id(&self) -> crate::events::JoystickID {
        7
    }
    fn vendor(&self) -> u16 {
        self.vendor
    }
    fn product(&self) -> u16 {
        self.product
    }
    fn product_version(&self) -> u16 {
        self.version
    }
    fn send_effect(&self, data: &[u8]) -> Result<()> {
        let mut failures = self.failures.lock().unwrap();
        if failures.0 > 0 {
            failures.0 -= 1;
        } else if failures.1 > 0 {
            failures.1 -= 1;
            self.log.lock().unwrap().push(hex_line("fail", data));
            return Err(Error::new("send failed"));
        }
        self.log.lock().unwrap().push(hex_line("write", data));
        Ok(())
    }
}

/// The numbers of an effect's line, in order.
struct Numbers<'a, 'b>(&'b mut std::str::SplitWhitespace<'a>);

impl Numbers<'_, '_> {
    fn next<T: std::str::FromStr>(&mut self) -> T
    where
        T::Err: std::fmt::Debug,
    {
        self.0.next().expect("a number").parse().unwrap()
    }

    fn direction(&mut self) -> HapticDirection {
        let kind = match self.next::<u8>() {
            0 => HapticDirectionType::Polar,
            1 => HapticDirectionType::Cartesian,
            2 => HapticDirectionType::Spherical,
            3 => HapticDirectionType::SteeringAxis,
            other => panic!("direction type {other}"),
        };
        HapticDirection {
            kind,
            dir: [self.next(), self.next(), self.next()],
        }
    }
}

/// An effect as the transcript prints it.
fn parse_effect(words: &mut std::str::SplitWhitespace<'_>) -> HapticEffect {
    let kind = words.next().unwrap();
    let mut n = Numbers(words);
    match kind {
        "constant" => HapticEffect::Constant(HapticConstant {
            direction: n.direction(),
            length: n.next(),
            delay: n.next(),
            level: n.next(),
            attack_length: n.next(),
            attack_level: n.next(),
            fade_length: n.next(),
            fade_level: n.next(),
            ..Default::default()
        }),
        "periodic" => {
            let waveform = match n.next::<u32>() {
                0x02 => HapticWaveform::Sine,
                0x04 => HapticWaveform::Square,
                0x08 => HapticWaveform::Triangle,
                0x10 => HapticWaveform::SawtoothUp,
                0x20 => HapticWaveform::SawtoothDown,
                other => panic!("periodic type {other}"),
            };
            HapticEffect::Periodic(HapticPeriodic {
                waveform,
                direction: n.direction(),
                length: n.next(),
                delay: n.next(),
                period: n.next(),
                magnitude: n.next(),
                offset: n.next(),
                phase: n.next(),
                attack_length: n.next(),
                attack_level: n.next(),
                fade_length: n.next(),
                fade_level: n.next(),
                ..Default::default()
            })
        }
        "condition" => {
            let kind = match n.next::<u32>() {
                0x080 => HapticConditionKind::Spring,
                0x100 => HapticConditionKind::Damper,
                0x200 => HapticConditionKind::Inertia,
                0x400 => HapticConditionKind::Friction,
                other => panic!("condition type {other}"),
            };
            HapticEffect::Condition(HapticCondition {
                kind,
                direction: n.direction(),
                length: n.next(),
                delay: n.next(),
                right_sat: [n.next(), 0, 0],
                left_sat: [n.next(), 0, 0],
                right_coeff: [n.next(), 0, 0],
                left_coeff: [n.next(), 0, 0],
                deadband: [n.next(), 0, 0],
                center: [n.next(), 0, 0],
                ..Default::default()
            })
        }
        "ramp" => HapticEffect::Ramp(HapticRamp {
            direction: n.direction(),
            length: n.next(),
            delay: n.next(),
            start: n.next(),
            end: n.next(),
            attack_length: n.next(),
            attack_level: n.next(),
            fade_length: n.next(),
            fade_level: n.next(),
            ..Default::default()
        }),
        "leftright" => HapticEffect::LeftRight(HapticLeftRight {
            length: n.next(),
            large_magnitude: n.next(),
            small_magnitude: n.next(),
        }),
        other => panic!("effect {other}"),
    }
}

/// An operation and what it printed.
struct Op {
    line: String,
    outputs: Vec<String>,
}

/// A case: its line and operations.
struct Case {
    line: String,
    ops: Vec<Op>,
}

/// The cases of the transcript, and its `frontend` calls (name and
/// lines).
fn parse() -> (Vec<Case>, Vec<(String, Vec<String>)>) {
    let mut cases: Vec<Case> = Vec::new();
    let mut calls: Vec<(String, Vec<String>)> = Vec::new();
    let mut lines = TRANSCRIPT.lines().filter(|l| !l.starts_with('#'));
    while let Some(line) = lines.next() {
        if let Some(case) = line.strip_prefix("case ") {
            cases.push(Case {
                line: case.to_owned(),
                ops: Vec::new(),
            });
        } else if let Some(op) = line.strip_prefix("op ") {
            let outputs = lines
                .by_ref()
                .take_while(|l| *l != "end")
                .map(str::to_owned)
                .collect();
            cases.last_mut().expect("an op in a case").ops.push(Op {
                line: op.to_owned(),
                outputs,
            });
        } else if let Some(call) = line.strip_prefix("call ") {
            let outputs = lines
                .by_ref()
                .take_while(|l| *l != "endcall")
                .map(str::to_owned)
                .collect();
            calls.push((call.to_owned(), outputs));
        } else {
            assert!(
                matches!(line, "endcase" | "frontend" | "endfrontend"),
                "unexpected line {line}"
            );
        }
    }
    (cases, calls)
}

/// The `frontend` calls of the transcript.
pub(in crate::haptic::hidapi) fn frontend_calls() -> Vec<(String, Vec<String>)> {
    parse().1
}

/// `result` and the error lines of a call's result, as the stubs print
/// them (the error first).
fn result_lines<T>(result: &Result<T>, ok: impl FnOnce(&T) -> String, failed: &str) -> Vec<String> {
    match result {
        Ok(value) => vec![format!("result {}", ok(value))],
        // (upstream sets no error for an update with bad parameters)
        Err(e) if e.message() == "Couldn't update effect" => vec![format!("result {failed}")],
        Err(e) => vec![format!("error {}", e.message()), format!("result {failed}")],
    }
}

/// Run an operation on the wheel, returning what it printed.
fn run_op(op: &str, joystick: &Arc<FakeJoystick>, ctx: &mut Option<Lg4ffHaptic>) -> Vec<String> {
    let mut words = op.split_whitespace();
    NOW.store(words.next().unwrap().parse().unwrap(), Ordering::Relaxed);
    let name = words.next().unwrap();
    let number = |words: &mut std::str::SplitWhitespace<'_>| -> i64 {
        words.next().unwrap().parse().unwrap()
    };

    let mut lines = Vec::new();
    if name == "open" {
        let fake: Arc<dyn HapticJoystick> = joystick.clone();
        match Lg4ffHaptic::open(fake, fake_now, false) {
            Ok(opened) => {
                lines.push(format!(
                    "features {} effects {} playing {} axes {}",
                    opened.features().0,
                    opened.num_effects(),
                    opened.num_effects_playing(),
                    opened.num_axes()
                ));
                lines.push("result 1".to_owned());
                *ctx = Some(opened);
            }
            Err(e) => {
                lines.push(format!("error {}", e.message()));
                lines.push("result 0".to_owned());
            }
        }
    } else if name == "fail" {
        let skip = number(&mut words) as u32;
        let fail = number(&mut words) as u32;
        joystick.fail(skip, fail);
    } else {
        let ctx_ref = ctx.as_mut().expect("an open wheel");
        let unit = |_: &()| "1".to_owned();
        lines = match name {
            "create" => {
                let effect = parse_effect(&mut words);
                result_lines(&ctx_ref.create_effect(&effect), |id| id.to_string(), "-1")
            }
            "update" => {
                let id = number(&mut words) as HapticEffectID;
                let effect = parse_effect(&mut words);
                result_lines(&ctx_ref.update_effect(id, &effect), unit, "0")
            }
            "run" => {
                let id = number(&mut words) as HapticEffectID;
                let iterations = number(&mut words) as u32;
                result_lines(&ctx_ref.run_effect(id, iterations), unit, "0")
            }
            "stop" => {
                let id = number(&mut words) as HapticEffectID;
                result_lines(&ctx_ref.stop_effect(id), unit, "0")
            }
            "destroy" => {
                ctx_ref.destroy_effect(number(&mut words) as HapticEffectID);
                Vec::new()
            }
            "status" => {
                let status = ctx_ref.effect_status(number(&mut words) as HapticEffectID);
                vec![format!("result {}", u8::from(status))]
            }
            "gain" => result_lines(&ctx_ref.set_gain(number(&mut words) as i32), unit, "0"),
            "autocenter" => result_lines(
                &ctx_ref.set_autocenter(number(&mut words) as i32),
                unit,
                "0",
            ),
            "pause" => result_lines(&ctx_ref.pause(), unit, "0"),
            "resume" => result_lines(&ctx_ref.resume(), unit, "0"),
            "stopall" => result_lines(&ctx_ref.stop_effects(), unit, "0"),
            "timer" => {
                let status = ctx_ref.timer(fake_now());
                let peak = ctx_ref.shared.lock().peak_ffb_level;
                vec![format!("result {status} peak {peak}")]
            }
            "close" => {
                if let Some(mut closing) = ctx.take() {
                    closing.close();
                }
                joystick.fail(0, 0);
                Vec::new()
            }
            other => panic!("unknown operation {other}"),
        };
    }
    assert!(words.next().is_none(), "left-over words in {op}");

    let mut outputs = joystick.take_log();
    outputs.append(&mut lines);
    outputs
}

/// Set the environment variables of a case (`-` unsets).
fn set_env(values: &[&str]) {
    for (name, value) in ENV_NAMES.iter().zip(values) {
        if *value == "-" {
            crate::stdlib::unsetenv_unsafe(name).unwrap();
        } else {
            crate::stdlib::setenv_unsafe(name, value, true).unwrap();
        }
    }
}

#[test]
fn transcript() {
    let _l = crate::test_support::test_lock();

    let (cases, calls) = parse();
    assert!(cases.len() > 40, "{} cases", cases.len());
    assert!(!calls.is_empty());

    for case in &cases {
        let (label, config) = case.line.split_once('|').unwrap();
        let words: Vec<&str> = config.split_whitespace().collect();
        let hex = |w: &str| u16::from_str_radix(w, 16).unwrap();
        let joystick = Arc::new(FakeJoystick::new(
            hex(words[0]),
            hex(words[1]),
            hex(words[2]),
        ));
        set_env(&words[3..7]);

        let mut ctx = None;
        for op in &case.ops {
            let outputs = run_op(&op.line, &joystick, &mut ctx);
            assert_eq!(outputs, op.outputs, "{label}: op {}", op.line);
        }
        assert!(ctx.is_none(), "{label}: not closed");
    }

    set_env(&["-"; 4]);
}

#[test]
fn slot_commands() {
    // TRANSLATE_FORCE truncates to the byte: -0x8000 is 0, 0x7fff is 0xff
    assert_eq!(translate_force(-0x8000), 0x00);
    assert_eq!(translate_force(-0x9000), 0x00);
    assert_eq!(translate_force(0), 0x80);
    assert_eq!(translate_force(0x7fff), 0xff);
    assert_eq!(translate_force(0x10000), 0xff);
    assert_eq!(scale_value_u16(0x12345, 8), 0xff);
    assert_eq!(scale_value_u16(-1, 4), 0xf);
    assert_eq!(scale_coeff(-0x4000, 4), 0x8);

    // A constant slot goes from op 1 to 0xc without counting as a change
    let mut slot = Slot::new(0, SlotType::Constant);
    let parameters = EffectParameters::default();
    update_slot(&mut slot, &parameters);
    assert_eq!(slot.current_cmd, [0x11, 0, 0x80, 0, 0, 0, 0]);
    assert!(slot.is_updated);
    slot.is_updated = false;
    update_slot(&mut slot, &parameters);
    assert_eq!(slot.current_cmd, [0x1c, 0, 0x80, 0, 0, 0, 0]);
    assert!(!slot.is_updated);

    // The directions
    let steering = HapticDirection {
        kind: HapticDirectionType::SteeringAxis,
        dir: [0; 3],
    };
    assert_eq!(to_linux_direction(&steering), 0x4000);
    let east = HapticDirection {
        kind: HapticDirectionType::Polar,
        dir: [9000, 0, 0],
    };
    assert_eq!(to_linux_direction(&east), 0x4000);
}
