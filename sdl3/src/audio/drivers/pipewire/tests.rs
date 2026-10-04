// Tests for the PipeWire driver: the SPA pod, JSON and dictionary helpers
// against the bytes and results of the C headers (PipeWire 1.2.7's
// spa_format_audio_raw_build()/_parse() and spa_json_*, printed by a
// throwaway C program), the format and channel tables, and, when a
// PipeWire server is reachable, playback, recording and hotplug through it.

use super::spa::*;
use super::*;
use crate::audio::{
    audio_device_name, current_audio_driver, playback_devices, recording_devices, AudioDevice,
    AudioStream, AUDIO_DEVICE_DEFAULT_PLAYBACK,
};
use crate::init::InitFlags;

use std::time::{Duration, Instant};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Build like `spa_format_audio_raw_build()`, then parse like
/// `spa_format_audio_raw_parse()` into an info filled with 0xAA bytes.
fn build_and_parse(info: &SpaAudioInfoRaw) -> (String, i32, SpaAudioInfoRaw) {
    let mut buf = [0u64; 128];
    // SAFETY: viewing the u64 buffer as bytes.
    let bytes = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), 1024) };
    let mut b = SpaPodBuilder::new(bytes);
    let range = spa_format_audio_raw_build(&mut b, SPA_PARAM_ENUM_FORMAT, info).unwrap();
    let pod_bytes = b.bytes(range).to_vec();
    let pod = Pod::new(&pod_bytes).unwrap();
    let mut out = SpaAudioInfoRaw {
        format: 0xAAAA_AAAA,
        flags: 0xAAAA_AAAA,
        rate: 0xAAAA_AAAA,
        channels: 0xAAAA_AAAA,
        position: [0xAAAA_AAAA; SPA_AUDIO_MAX_CHANNELS],
    };
    let r = spa_format_audio_raw_parse(pod, &mut out);
    (hex(&pod_bytes), r, out)
}

fn info(format: u32, rate: u32, channels: u32, pos: &[u32], flags: u32) -> SpaAudioInfoRaw {
    let mut i = SpaAudioInfoRaw {
        format,
        rate,
        channels,
        flags,
        ..SpaAudioInfoRaw::default()
    };
    i.position[..pos.len()].copy_from_slice(pos);
    i
}

#[test]
fn audio_raw_pods_match_the_c_headers() {
    const AA: u32 = 0xAAAA_AAAA;
    // (the info, the C builder's bytes, the C parser's result, its format,
    // rate, channels and flags, and the first two positions)
    type Case = (SpaAudioInfoRaw, &'static str, i32, [u32; 4], u32, [u32; 2]);
    let cases: [Case; 5] = [
        (
            info(SPA_AUDIO_FORMAT_U8, 22050, 1, &[SPA_AUDIO_CHANNEL_MONO], 0),
            "a00000000f000000030004000300000001000000000000000400000003000000010000000000000002000000000000000400000003000000010000000000000001000100000000000400000003000000020100000000000003000100000000000400000004000000225600000000000004000100000000000400000004000000010000000000000005000100000000000c0000000d00000004000000030000000200000000000000",
            4, [258, 22050, 1, 0], 0, [2, AA],
        ),
        (
            info(SPA_AUDIO_FORMAT_S16_LE, 48000, 2, &[3, 4], 0),
            "a00000000f00000003000400030000000100000000000000040000000300000001000000000000000200000000000000040000000300000001000000000000000100010000000000040000000300000003010000000000000300010000000000040000000400000080bb0000000000000400010000000000040000000400000002000000000000000500010000000000100000000d00000004000000030000000300000004000000",
            4, [259, 48000, 2, 0], 0, [3, 4],
        ),
        (
            info(SPA_AUDIO_FORMAT_F32_LE, 44100, 6, &[3, 4, 5, 6, 12, 13], 0),
            "b00000000f0000000300040003000000010000000000000004000000030000000100000000000000020000000000000004000000030000000100000000000000010001000000000004000000030000001b010000000000000300010000000000040000000400000044ac0000000000000400010000000000040000000400000006000000000000000500010000000000200000000d0000000400000003000000030000000400000005000000060000000c0000000d000000",
            4, [283, 44100, 6, 0], 0, [3, 4],
        ),
        (
            info(SPA_AUDIO_FORMAT_UNKNOWN, 0, 2, &[3, 4], SPA_AUDIO_FLAG_UNPOSITIONED),
            "500000000f0000000300040003000000010000000000000004000000030000000100000000000000020000000000000004000000030000000100000000000000040001000000000004000000040000000200000000000000",
            1, [AA, AA, 2, 0], 1, [AA, AA],
        ),
        (
            info(SPA_AUDIO_FORMAT_S32_BE, 96000, 0, &[], 0),
            "680000000f0000000300040003000000010000000000000004000000030000000100000000000000020000000000000004000000030000000100000000000000010001000000000004000000030000000c01000000000000030001000000000004000000040000000077010000000000",
            2, [268, 96000, AA, 0], 1, [AA, AA],
        ),
    ];
    for (i, (info, bytes, res, [format, rate, channels, _], flags, pos)) in cases.iter().enumerate()
    {
        let (got, r, out) = build_and_parse(info);
        assert_eq!(got, *bytes, "case {i}: built pod");
        assert_eq!(r, *res, "case {i}: parse result");
        assert_eq!(
            (out.format, out.rate, out.channels, out.flags),
            (*format, *rate, *channels, *flags),
            "case {i}"
        );
        assert_eq!(out.position[..2], pos[..], "case {i}: positions");
    }

    // The builder gives up when the pod doesn't fit.
    let mut small = [0u64; 4];
    // SAFETY: viewing the u64 buffer as bytes.
    let bytes = unsafe { std::slice::from_raw_parts_mut(small.as_mut_ptr().cast::<u8>(), 32) };
    let mut b = SpaPodBuilder::new(bytes);
    assert!(spa_format_audio_raw_build(&mut b, SPA_PARAM_ENUM_FORMAT, &cases[1].0).is_none());
}

/// A Format object with an Enum choice of formats, a Range choice of rates
/// (48000, 8000..192000) and an Int channel count of 2, as the C builder
/// makes it.
const RANGE_POD: &str = "800000000f000000030004000300000001000100000000001c00000013000000030000000000000004000000030000001b0100001b010000030100000000000003000100000000001c000000130000000100000000000000040000000400000080bb0000401f000000ee020000000000040001000000000004000000040000000200000000000000";

#[test]
fn params_are_read_like_the_c_helpers() {
    let bytes = unhex(RANGE_POD);
    let pod = Pod::new(&bytes).unwrap();

    // C: "range-parse 1 fmt=0 rate=0 ch=2 flags=1" (choices aren't collected)
    let mut out = SpaAudioInfoRaw::default();
    assert_eq!(spa_format_audio_raw_parse(pod, &mut out), 1);
    assert_eq!(
        (out.format, out.rate, out.channels, out.flags),
        (0, 0, 2, 1)
    );

    assert_eq!(
        get_range_param(pod, SPA_FORMAT_AUDIO_RATE),
        Some((48000, 8000, 192000))
    );
    assert_eq!(
        get_range_param(pod, SPA_FORMAT_AUDIO_CHANNELS),
        None,
        "not a choice"
    );
    assert_eq!(get_range_param(pod, 0x10001), None, "an enum, not a range");
    assert_eq!(get_int_param(pod, SPA_FORMAT_AUDIO_CHANNELS), Some(2));
    assert_eq!(
        get_int_param(pod, SPA_FORMAT_AUDIO_RATE),
        None,
        "a choice, not an int"
    );
    assert_eq!(get_int_param(pod, 0x12345), None, "no such key");

    // Not an object: nothing to find.
    let int_pod = unhex("040000000400000002000000");
    assert_eq!(
        get_int_param(Pod::new(&int_pod).unwrap(), SPA_FORMAT_AUDIO_CHANNELS),
        None
    );
    // A truncated pod is rejected.
    assert!(Pod::new(&bytes[..100]).is_none());
}

#[test]
fn json_like_the_c_tokenizer() {
    // The results of get_name_from_json() in C.
    for (json, expected) in [
        (
            "{ \"name\": \"alsa_output.pci-0000_00_1f.3.analog-stereo\" }",
            Some("alsa_output.pci-0000_00_1f.3.analog-stereo"),
        ),
        ("{\"name\":\"sdl-null-sink\"}", Some("sdl-null-sink")),
        ("{ name = bare.value }", Some("bare.value")),
        ("[ \"not an object\" ]", None),
        ("{ \"name\" }", None),
        (
            "{ \"nam\\u00e9\": \"esc\\\"aped \\u00e9\\ud83d\\ude00 \\n\" }",
            None,
        ),
        ("{ \"toolongkey\": \"x\" }", None),
        ("", None),
    ] {
        assert_eq!(
            get_name_from_json(json.as_bytes()).as_deref(),
            expected,
            "{json}"
        );
    }

    // The session services array walk of client_info(): (found, strings read)
    for (services, expected) in [
        ("[ \"video\", \"audio\" ]", (true, 2)),
        ("[ video ]", (false, 1)),
        ("\"audio\"", (false, 0)),
        ("[ \"au\\u0064io\" ]", (true, 1)),
    ] {
        let mut found = false;
        let mut n = 0;
        let mut iter0 = SpaJson::new(services.as_bytes());
        if let Ok(mut iter1) = iter0.enter_array() {
            while let Ok(element) = iter1.get_string(PW_MAX_IDENTIFIER_LENGTH) {
                n += 1;
                if element == b"audio" {
                    found = true;
                    break;
                }
            }
        }
        assert_eq!((found, n), expected, "{services}");
    }

    // Escapes, including \u with a surrogate pair, are decoded (C: 610962c3a9f09f9880).
    let mut j = SpaJson::new(b"[\"a\\tb\\u00e9\\ud83d\\ude00\"]");
    let mut a = j.enter_array().unwrap();
    assert_eq!(hex(&a.get_string(64).unwrap()), "610962c3a9f09f9880");
    // An unknown escape is a parse error (C: -1).
    let mut j = SpaJson::new(b"[\"a\\q\"]");
    assert_eq!(j.enter_array().unwrap().get_string(64), Err(-1));
    // Strings that don't fit are an error.
    let mut j = SpaJson::new(b"[\"abcdef\"]");
    assert!(j.enter_array().unwrap().get_string(8).is_err());
}

#[test]
fn dict_lookup() {
    let k1 = c"media.class";
    let v1 = c"Audio/Sink";
    let k2 = c"node.name";
    let v2 = c"sdl-null-sink";
    for flags in [0, 1] {
        let items = [
            SpaDictItem {
                key: k1.as_ptr(),
                value: v1.as_ptr(),
            },
            SpaDictItem {
                key: k2.as_ptr(),
                value: v2.as_ptr(),
            },
        ];
        let dict = SpaDict {
            flags, // unsorted, then SPA_DICT_FLAG_SORTED (the keys are in order)
            n_items: 2,
            items: items.as_ptr(),
        };
        // SAFETY: a valid dictionary of C strings.
        unsafe {
            assert_eq!(spa_dict_lookup(&dict, "node.name"), Some(v2));
            assert_eq!(spa_dict_lookup(&dict, "media.class"), Some(v1));
            assert_eq!(spa_dict_lookup(&dict, "node.description"), None);
        }
    }
    // SAFETY: NULL is allowed.
    assert_eq!(unsafe { spa_dict_lookup(ptr::null(), "x") }, None);
}

#[test]
fn version_parsing() {
    let mut v = [0; 3];
    assert_eq!(sscanf_version("1.2.7", &mut v), 3);
    assert_eq!(v, [1, 2, 7]);
    assert_eq!(sscanf_version("0.3.48-dev", &mut v), 3);
    assert_eq!(v, [0, 3, 48]);
    let mut v = [0; 3];
    assert_eq!(sscanf_version("1.4", &mut v), 2);
    assert_eq!(v, [1, 4, 0]);
    assert_eq!(sscanf_version("x", &mut v), 0);
}

#[test]
fn spa_info_from_sdl_specs() {
    let mut i = SpaAudioInfoRaw::default();
    initialize_spa_info(&AudioSpec::new(AudioFormat::S16LE, 2, 48000), &mut i);
    assert_eq!(
        (i.format, i.channels, i.rate),
        (SPA_AUDIO_FORMAT_S16_LE, 2, 48000)
    );
    assert_eq!(
        i.position[..2],
        [SPA_AUDIO_CHANNEL_FL, SPA_AUDIO_CHANNEL_FR]
    );
    // SPA_AUDIO_CHANNEL_*: MONO 2, FL 3, FR 4, FC 5, LFE 6, SL 7, SR 8, RC 11, RL 12, RR 13.
    // The layouts of SDL_audio.h: 4.1 is FL FR LFE BL BR, 6.1 is FL FR FC LFE BC SL SR.
    for (channels, map) in [
        (1, &[2][..]),
        (3, &[3, 4, 6][..]),
        (4, &[3, 4, 12, 13][..]),
        (5, &[3, 4, 6, 12, 13][..]),
        (6, &[3, 4, 5, 6, 12, 13][..]),
        (7, &[3, 4, 5, 6, 11, 7, 8][..]),
        (8, &[3, 4, 5, 6, 12, 13, 7, 8][..]),
    ] {
        let mut i = SpaAudioInfoRaw::default();
        initialize_spa_info(&AudioSpec::new(AudioFormat::F32, channels, 44100), &mut i);
        assert_eq!(&i.position[..channels as usize], map, "{channels} channels");
    }
    for (sdl, spa) in [
        (AudioFormat::U8, 0x102),
        (AudioFormat::S8, 0x101),
        (AudioFormat::S16LE, 0x103),
        (AudioFormat::S16BE, 0x104),
        (AudioFormat::S32LE, 0x10b),
        (AudioFormat::S32BE, 0x10c),
        (AudioFormat::F32LE, 0x11b),
        (AudioFormat::F32BE, 0x11c),
    ] {
        let mut i = SpaAudioInfoRaw::default();
        initialize_spa_info(&AudioSpec::new(sdl, 2, 48000), &mut i);
        assert_eq!(i.format, spa);
        assert_eq!(spa_format_to_sdl(spa), sdl);
    }
    assert_eq!(spa_format_to_sdl(0x107), AudioFormat::UNKNOWN); // S24_32_LE
}

#[test]
fn abi_layouts() {
    use std::mem::{offset_of, size_of};
    #[cfg(target_pointer_width = "64")]
    {
        assert_eq!(size_of::<SpaHook>(), 48);
        assert_eq!(size_of::<SpaInterface>(), 32);
        assert_eq!(offset_of!(PwCoreInfo, version), 24);
        assert_eq!(offset_of!(PwCoreInfo, props), 48);
        assert_eq!(offset_of!(PwNodeInfo, props), 48);
        assert_eq!(offset_of!(PwNodeInfo, n_params), 64);
        assert_eq!(offset_of!(PwClientInfo, props), 16);
        assert_eq!(size_of::<SpaParamInfo>(), 32);
        assert_eq!(size_of::<SpaData>(), 40);
        assert_eq!(offset_of!(SpaData, data), 24);
        assert_eq!(size_of::<PwBuffer>(), 40);
        assert_eq!(size_of::<PwStreamEvents>(), 8 * 12);
        assert_eq!(size_of::<PwCoreEvents>(), 8 * 10);
    }
}

/// Whether libpipewire loads here; reports the skip when it doesn't.
fn have_libpipewire() -> bool {
    match SharedObject::load(PIPEWIRE_LIBRARY) {
        Ok(_) => true,
        Err(e) => {
            crate::test_support::skip("pipewire", e.message());
            false
        }
    }
}

fn wait_for(what: &dyn Fn() -> bool) -> bool {
    let start = Instant::now();
    while !what() && start.elapsed() < Duration::from_secs(10) {
        crate::events::pump(); // (disconnects are handled on the main thread)
        std::thread::sleep(Duration::from_millis(10));
    }
    what()
}

#[test]
fn playback_recording_and_hotplug_through_a_server() {
    let _l = crate::test_support::test_lock();
    if !have_libpipewire() {
        return;
    }
    let _cleanup = crate::audio::drivers::tests::QuitAudioOnDrop;
    crate::hints::set(crate::hints::AUDIO_DRIVER, "pipewire").unwrap();
    let r = crate::init::init_subsystem(InitFlags::AUDIO);
    crate::hints::reset(crate::hints::AUDIO_DRIVER);
    if let Err(e) = r {
        crate::test_support::skip("pipewire", format_args!("no server ({})", e.message()));
        return;
    }
    assert_eq!(current_audio_driver(), Some("pipewire"));
    let devices = playback_devices().unwrap();
    if devices.is_empty() {
        crate::test_support::skip("pipewire", "the PipeWire server has no sinks");
        crate::init::quit_subsystem(InitFlags::AUDIO);
        return;
    }
    let names: Vec<String> = devices
        .iter()
        .map(|&d| audio_device_name(d).unwrap())
        .collect();
    eprintln!("note: PipeWire sinks: {names:?}");

    // Playback: the stream drains into the default sink.
    let dev = AudioDevice::open(AUDIO_DEVICE_DEFAULT_PLAYBACK, None).unwrap();
    let (spec, frames) = dev.format().unwrap();
    assert!(frames >= PW_MIN_SAMPLES);
    let stream = AudioStream::new(Some(&spec), None).unwrap();
    dev.bind(&stream).unwrap();
    stream
        .put_data(&vec![0x11u8; spec.frame_size() * frames as usize * 4])
        .unwrap();
    assert!(
        wait_for(&|| stream.queued() == 0),
        "the device pulled the data"
    );
    drop(dev);
    drop(stream);

    // Recording, when there's a source.
    if let Some(&source) = recording_devices().unwrap().first() {
        let rec = AudioDevice::open(source, None).unwrap();
        let (rspec, _) = rec.format().unwrap();
        let rstream = AudioStream::new(None, Some(&rspec)).unwrap();
        rec.bind(&rstream).unwrap();
        assert!(wait_for(&|| rstream.available() > 0), "the source recorded");
        drop(rec);
        drop(rstream);
    }

    // Hotplug, when pw-cli is there to make a node: it appears, becomes the
    // default through the metadata, and disappears again.
    let run = |cmd: &str, args: &[&str]| {
        std::process::Command::new(cmd)
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if run(
        "pw-cli",
        &[
            "create-node",
            "adapter",
            "{ factory.name=support.null-audio-sink node.name=sdl-hotplugged \
             node.description=Hotplugged media.class=Audio/Sink audio.position=[FL FR] \
             object.linger=true }",
        ],
    ) {
        let named = |name: &str| {
            playback_devices()
                .unwrap()
                .into_iter()
                .find(|&d| audio_device_name(d).is_ok_and(|n| n == name))
        };
        // The default we set dropped, then the node destroyed (so the
        // session manager chooses the default again), even when an
        // assertion fails, so a rerun starts clean.
        struct Hotplugged;
        impl Drop for Hotplugged {
            fn drop(&mut self) {
                let _ = std::process::Command::new("pw-metadata")
                    .args(["-d", "0", "default.audio.sink"])
                    .output();
                let _ = std::process::Command::new("pw-cli")
                    .args(["destroy", "sdl-hotplugged"])
                    .output();
            }
        }
        let hotplugged = Hotplugged;
        assert!(wait_for(&|| named("Hotplugged").is_some()), "added");
        let is_default =
            || audio_device_name(AUDIO_DEVICE_DEFAULT_PLAYBACK).is_ok_and(|n| n == "Hotplugged");
        // A session manager (WirePlumber) that is still reacting to the new
        // node may write its own choice over ours, so set it again until it
        // sticks.
        let mut changed = false;
        for _ in 0..10 {
            if !run(
                "pw-metadata",
                &[
                    "0",
                    "default.audio.sink",
                    "{ \"name\": \"sdl-hotplugged\" }",
                ],
            ) {
                changed = true; // (no pw-metadata: nothing to check)
                break;
            }
            let start = Instant::now();
            while !is_default() && start.elapsed() < Duration::from_secs(1) {
                crate::events::pump();
                std::thread::sleep(Duration::from_millis(10));
            }
            changed = is_default();
            if changed {
                break;
            }
        }
        assert!(changed, "the new default");
        drop(hotplugged);
        assert!(wait_for(&|| named("Hotplugged").is_none()), "removed");
    } else {
        crate::test_support::skip("pipewire", "no pw-cli for the hotplug check");
    }

    crate::init::quit_subsystem(InitFlags::AUDIO);
}
