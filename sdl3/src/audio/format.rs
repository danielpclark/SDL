// Rust translation of the format definitions of include/SDL3/SDL_audio.h and the
// format helpers of src/audio/SDL_audio.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

/// Audio format. Translation of `SDL_AudioFormat`.
///
/// The bits of the value encode the sample type:
///
/// ```text
/// ++-----------------------sample is signed if set
/// ||
/// ||       ++-----------sample is bigendian if set
/// ||       ||
/// ||       ||          ++---sample is float if set
/// ||       ||          ||
/// ||       ||          || +=--sample bit size--------+
/// ||       ||          || ||                         ||
/// 15 14 13 12 11 10 09 08 07 06 05 04 03 02 01 00
/// ```
///
/// There are helper methods that can read these bits ([`bitsize`](Self::bitsize),
/// [`is_float`](Self::is_float), ...). Although the bitfields could in
/// theory describe many formats, only the constants below are supported.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
pub struct AudioFormat(pub u32);

/// Mask of bits in an [`AudioFormat`] that contains the format bit size.
/// Translation of `SDL_AUDIO_MASK_BITSIZE`.
pub const AUDIO_MASK_BITSIZE: u32 = 0xFF;
/// Mask of bits in an [`AudioFormat`] that contain the floating point flag.
/// Translation of `SDL_AUDIO_MASK_FLOAT`.
pub const AUDIO_MASK_FLOAT: u32 = 1 << 8;
/// Mask of bits in an [`AudioFormat`] that contain the bigendian flag.
/// Translation of `SDL_AUDIO_MASK_BIG_ENDIAN`.
pub const AUDIO_MASK_BIG_ENDIAN: u32 = 1 << 12;
/// Mask of bits in an [`AudioFormat`] that contain the signed data flag.
/// Translation of `SDL_AUDIO_MASK_SIGNED`.
pub const AUDIO_MASK_SIGNED: u32 = 1 << 15;

/// Define an [`AudioFormat`] value. Translation of `SDL_DEFINE_AUDIO_FORMAT()`.
pub const fn define_audio_format(
    signed: bool,
    bigendian: bool,
    float: bool,
    size: u32,
) -> AudioFormat {
    AudioFormat(
        ((signed as u32) << 15)
            | ((bigendian as u32) << 12)
            | ((float as u32) << 8)
            | (size & AUDIO_MASK_BITSIZE),
    )
}

impl AudioFormat {
    /// Unspecified audio format
    pub const UNKNOWN: AudioFormat = AudioFormat(0x0000);
    /// Unsigned 8-bit samples
    pub const U8: AudioFormat = AudioFormat(0x0008);
    /// Signed 8-bit samples
    pub const S8: AudioFormat = AudioFormat(0x8008);
    /// Signed 16-bit samples
    pub const S16LE: AudioFormat = AudioFormat(0x8010);
    /// As above, but big-endian byte order
    pub const S16BE: AudioFormat = AudioFormat(0x9010);
    /// 32-bit integer samples
    pub const S32LE: AudioFormat = AudioFormat(0x8020);
    /// As above, but big-endian byte order
    pub const S32BE: AudioFormat = AudioFormat(0x9020);
    /// 32-bit floating point samples
    pub const F32LE: AudioFormat = AudioFormat(0x8120);
    /// As above, but big-endian byte order
    pub const F32BE: AudioFormat = AudioFormat(0x9120);

    /// Signed 16-bit samples in native byte order.
    #[cfg(target_endian = "little")]
    pub const S16: AudioFormat = AudioFormat::S16LE;
    /// 32-bit integer samples in native byte order.
    #[cfg(target_endian = "little")]
    pub const S32: AudioFormat = AudioFormat::S32LE;
    /// 32-bit floating point samples in native byte order.
    #[cfg(target_endian = "little")]
    pub const F32: AudioFormat = AudioFormat::F32LE;
    /// Signed 16-bit samples in native byte order.
    #[cfg(target_endian = "big")]
    pub const S16: AudioFormat = AudioFormat::S16BE;
    /// 32-bit integer samples in native byte order.
    #[cfg(target_endian = "big")]
    pub const S32: AudioFormat = AudioFormat::S32BE;
    /// 32-bit floating point samples in native byte order.
    #[cfg(target_endian = "big")]
    pub const F32: AudioFormat = AudioFormat::F32BE;

    /// Retrieve the size, in bits, of the format (8 for `S8`, 16 for `S16LE`...).
    /// Translation of `SDL_AUDIO_BITSIZE()`.
    pub const fn bitsize(self) -> u32 {
        self.0 & AUDIO_MASK_BITSIZE
    }

    /// Retrieve the size, in bytes, of the format. Translation of `SDL_AUDIO_BYTESIZE()`.
    pub const fn bytesize(self) -> u32 {
        self.bitsize() / 8
    }

    /// Whether the format represents floating point data. Translation of `SDL_AUDIO_ISFLOAT()`.
    pub const fn is_float(self) -> bool {
        self.0 & AUDIO_MASK_FLOAT != 0
    }

    /// Whether the format represents bigendian data. Translation of `SDL_AUDIO_ISBIGENDIAN()`.
    pub const fn is_big_endian(self) -> bool {
        self.0 & AUDIO_MASK_BIG_ENDIAN != 0
    }

    /// Whether the format represents littleendian data. Translation of `SDL_AUDIO_ISLITTLEENDIAN()`.
    pub const fn is_little_endian(self) -> bool {
        !self.is_big_endian()
    }

    /// Whether the format represents signed data. Translation of `SDL_AUDIO_ISSIGNED()`.
    pub const fn is_signed(self) -> bool {
        self.0 & AUDIO_MASK_SIGNED != 0
    }

    /// Whether the format represents integer data. Translation of `SDL_AUDIO_ISINT()`.
    pub const fn is_int(self) -> bool {
        !self.is_float()
    }

    /// Whether the format represents unsigned data. Translation of `SDL_AUDIO_ISUNSIGNED()`.
    pub const fn is_unsigned(self) -> bool {
        !self.is_signed()
    }

    /// The human readable name of the format ("SDL_AUDIO_S16LE", ...;
    /// "SDL_AUDIO_UNKNOWN" if not recognized). Translation of `SDL_GetAudioFormatName()`.
    pub const fn name(self) -> &'static str {
        match self {
            AudioFormat::U8 => "SDL_AUDIO_U8",
            AudioFormat::S8 => "SDL_AUDIO_S8",
            AudioFormat::S16LE => "SDL_AUDIO_S16LE",
            AudioFormat::S16BE => "SDL_AUDIO_S16BE",
            AudioFormat::S32LE => "SDL_AUDIO_S32LE",
            AudioFormat::S32BE => "SDL_AUDIO_S32BE",
            AudioFormat::F32LE => "SDL_AUDIO_F32LE",
            AudioFormat::F32BE => "SDL_AUDIO_F32BE",
            _ => "SDL_AUDIO_UNKNOWN",
        }
    }

    /// The name without the "SDL_AUDIO_" prefix. Translation of `GetShortAudioFormatName()`.
    pub(crate) fn short_name(self) -> &'static str {
        &self.name()[10..]
    }

    /// The byte value that represents silence in this format: 0x80 for
    /// `U8`, 0 for everything else. Translation of `SDL_GetSilenceValueForFormat()`.
    pub const fn silence_value(self) -> u8 {
        if self.0 == AudioFormat::U8.0 {
            0x80
        } else {
            0x00
        }
    }

    /// Translation of `SDL_IsSupportedAudioFormat()`.
    pub(crate) const fn is_supported(self) -> bool {
        matches!(
            self,
            AudioFormat::U8
                | AudioFormat::S8
                | AudioFormat::S16LE
                | AudioFormat::S16BE
                | AudioFormat::S32LE
                | AudioFormat::S32BE
                | AudioFormat::F32LE
                | AudioFormat::F32BE
        )
    }

    /// Translation of `ParseAudioFormatString()`.
    pub(crate) fn parse(string: Option<&str>) -> AudioFormat {
        match string {
            Some("U8") => AudioFormat::U8,
            Some("S8") => AudioFormat::S8,
            Some("S16LE") => AudioFormat::S16LE,
            Some("S16BE") => AudioFormat::S16BE,
            Some("S16") => AudioFormat::S16,
            Some("S32LE") => AudioFormat::S32LE,
            Some("S32BE") => AudioFormat::S32BE,
            Some("S32") => AudioFormat::S32,
            Some("F32LE") => AudioFormat::F32LE,
            Some("F32BE") => AudioFormat::F32BE,
            Some("F32") => AudioFormat::F32,
            _ => AudioFormat::UNKNOWN,
        }
    }

    /// Formats ordered most similar to `self` to least (empty if `self` is
    /// not supported). Translation of `SDL_ClosestAudioFormats()`.
    pub fn closest_formats(self) -> &'static [AudioFormat] {
        FORMAT_LIST
            .iter()
            .find(|list| list[0] == self)
            .map_or(&[][..], |list| &list[..])
    }
}

impl std::fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(target_endian = "little")]
mod native {
    use super::AudioFormat;
    pub const NATIVE_F32: AudioFormat = AudioFormat::F32LE;
    pub const SWAPPED_F32: AudioFormat = AudioFormat::F32BE;
    pub const NATIVE_S16: AudioFormat = AudioFormat::S16LE;
    pub const SWAPPED_S16: AudioFormat = AudioFormat::S16BE;
    pub const NATIVE_S32: AudioFormat = AudioFormat::S32LE;
    pub const SWAPPED_S32: AudioFormat = AudioFormat::S32BE;
}
#[cfg(target_endian = "big")]
mod native {
    use super::AudioFormat;
    pub const NATIVE_F32: AudioFormat = AudioFormat::F32BE;
    pub const SWAPPED_F32: AudioFormat = AudioFormat::F32LE;
    pub const NATIVE_S16: AudioFormat = AudioFormat::S16BE;
    pub const SWAPPED_S16: AudioFormat = AudioFormat::S16LE;
    pub const NATIVE_S32: AudioFormat = AudioFormat::S32BE;
    pub const SWAPPED_S32: AudioFormat = AudioFormat::S32LE;
}
use native::*;

const NUM_FORMATS: usize = 8;
const U8: AudioFormat = AudioFormat::U8;
const S8: AudioFormat = AudioFormat::S8;

/// always favor Float32 in native byte order, since we're probably going to
/// convert to that for processing anyhow. Translation of `format_list`
/// (without the terminating `SDL_AUDIO_UNKNOWN`).
static FORMAT_LIST: [[AudioFormat; NUM_FORMATS]; NUM_FORMATS] = [
    [
        U8,
        NATIVE_F32,
        SWAPPED_F32,
        S8,
        NATIVE_S16,
        SWAPPED_S16,
        NATIVE_S32,
        SWAPPED_S32,
    ],
    [
        S8,
        NATIVE_F32,
        SWAPPED_F32,
        U8,
        NATIVE_S16,
        SWAPPED_S16,
        NATIVE_S32,
        SWAPPED_S32,
    ],
    [
        NATIVE_S16,
        NATIVE_F32,
        SWAPPED_F32,
        SWAPPED_S16,
        NATIVE_S32,
        SWAPPED_S32,
        U8,
        S8,
    ],
    [
        SWAPPED_S16,
        NATIVE_F32,
        SWAPPED_F32,
        NATIVE_S16,
        SWAPPED_S32,
        NATIVE_S32,
        U8,
        S8,
    ],
    [
        NATIVE_S32,
        NATIVE_F32,
        SWAPPED_F32,
        SWAPPED_S32,
        NATIVE_S16,
        SWAPPED_S16,
        U8,
        S8,
    ],
    [
        SWAPPED_S32,
        NATIVE_F32,
        SWAPPED_F32,
        NATIVE_S32,
        SWAPPED_S16,
        NATIVE_S16,
        U8,
        S8,
    ],
    [
        NATIVE_F32,
        SWAPPED_F32,
        NATIVE_S32,
        SWAPPED_S32,
        NATIVE_S16,
        SWAPPED_S16,
        U8,
        S8,
    ],
    [
        SWAPPED_F32,
        NATIVE_F32,
        SWAPPED_S32,
        NATIVE_S32,
        SWAPPED_S16,
        NATIVE_S16,
        U8,
        S8,
    ],
];

/// Format specifier for audio data. Translation of `SDL_AudioSpec`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct AudioSpec {
    /// Audio data format
    pub format: AudioFormat,
    /// Number of channels: 1 mono, 2 stereo, etc
    pub channels: i32,
    /// sample rate: sample frames per second
    pub freq: i32,
}

impl AudioSpec {
    /// A spec from its fields.
    pub const fn new(format: AudioFormat, channels: i32, freq: i32) -> AudioSpec {
        AudioSpec {
            format,
            channels,
            freq,
        }
    }

    /// The size, in bytes, of one sample frame (all channels).
    /// Translation of `SDL_AUDIO_FRAMESIZE()`.
    pub const fn frame_size(&self) -> usize {
        self.format.bytesize() as usize
            * if self.channels > 0 {
                self.channels as usize
            } else {
                0
            }
    }
}

/// The largest number of channels SDL channel maps handle.
/// Translation of `SDL_MAX_CHANNELMAP_CHANNELS`.
///
/// !!! FIXME: if SDL ever supports more channels, clean this out and make those parts dynamic.
pub(crate) const MAX_CHANNELMAP_CHANNELS: usize = 8;

/// Translation of `SDL_IsSupportedChannelCount()`.
pub(crate) const fn is_supported_channel_count(channels: i32) -> bool {
    channels >= 1 && channels <= 8
}

/// Whether a channel map would index outside of `channels` (or below -1).
/// Translation of `SDL_ChannelMapIsBogus()`.
pub(crate) fn channel_map_is_bogus(chmap: Option<&[i32]>, channels: i32) -> bool {
    chmap.is_some_and(|map| {
        map.iter()
            .take(channels.max(0) as usize)
            .any(|&mapping| mapping < -1 || mapping >= channels)
    })
}

/// Whether a channel map is the identity. Translation of `SDL_ChannelMapIsDefault()`.
pub(crate) fn channel_map_is_default(chmap: Option<&[i32]>, channels: i32) -> bool {
    chmap.is_none_or(|map| {
        map.iter()
            .take(channels.max(0) as usize)
            .enumerate()
            .all(|(i, &m)| m == i as i32)
    })
}

/// See if two channel maps match; `None` is the default layout.
/// Translation of `SDL_AudioChannelMapsEqual()`.
pub(crate) fn audio_channel_maps_equal(
    channels: i32,
    a: Option<&[i32]>,
    b: Option<&[i32]>,
) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            let n = channels.max(0) as usize;
            a[..n] == b[..n]
        }
        _ => false,
    }
}

/// Compare two specs (and channel maps), return true if they match exactly.
/// Translation of `SDL_AudioSpecsEqual()`.
pub(crate) fn audio_specs_equal(
    a: &AudioSpec,
    b: &AudioSpec,
    map_a: Option<&[i32]>,
    map_b: Option<&[i32]>,
) -> bool {
    if a.format != b.format
        || a.channels != b.channels
        || a.freq != b.freq
        || map_a.is_some() != map_b.is_some()
    {
        return false;
    }
    match (map_a, map_b) {
        (Some(ma), Some(mb)) => {
            let n = a.channels.max(0) as usize;
            ma[..n] == mb[..n]
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bits() {
        assert_eq!(
            define_audio_format(true, false, false, 16),
            AudioFormat::S16LE
        );
        assert_eq!(
            define_audio_format(true, true, true, 32),
            AudioFormat::F32BE
        );
        assert_eq!(define_audio_format(false, false, false, 8), AudioFormat::U8);
        assert_eq!(AudioFormat::S32BE.bitsize(), 32);
        assert_eq!(AudioFormat::S16LE.bytesize(), 2);
        assert!(AudioFormat::F32LE.is_float() && AudioFormat::F32LE.is_signed());
        assert!(AudioFormat::U8.is_unsigned() && AudioFormat::U8.is_int());
        assert!(AudioFormat::S16BE.is_big_endian() && AudioFormat::S16LE.is_little_endian());
        assert_eq!(AudioFormat::S16BE.name(), "SDL_AUDIO_S16BE");
        assert_eq!(AudioFormat(0x1234).name(), "SDL_AUDIO_UNKNOWN");
        assert_eq!(
            AudioFormat::F32.short_name(),
            if cfg!(target_endian = "little") {
                "F32LE"
            } else {
                "F32BE"
            }
        );
        assert_eq!(AudioFormat::U8.silence_value(), 0x80);
        assert_eq!(AudioFormat::S8.silence_value(), 0);
        assert_eq!(AudioFormat::parse(Some("S16")), AudioFormat::S16);
        assert_eq!(AudioFormat::parse(Some("s16")), AudioFormat::UNKNOWN);
        assert_eq!(AudioSpec::new(AudioFormat::S16LE, 2, 44100).frame_size(), 4);
    }

    #[test]
    fn closest_formats() {
        let c = AudioFormat::S16.closest_formats();
        assert_eq!(c.len(), 8);
        assert_eq!(c[0], AudioFormat::S16);
        assert_eq!(c[1], AudioFormat::F32);
        assert!(AudioFormat(7).closest_formats().is_empty());
    }

    #[test]
    fn channel_maps() {
        assert!(channel_map_is_default(None, 2));
        assert!(channel_map_is_default(Some(&[0, 1]), 2));
        assert!(!channel_map_is_default(Some(&[1, 0]), 2));
        assert!(!channel_map_is_bogus(Some(&[-1, 1]), 2));
        assert!(channel_map_is_bogus(Some(&[0, 2]), 2));
        assert!(channel_map_is_bogus(Some(&[-2, 0]), 2));
        assert!(audio_channel_maps_equal(2, None, None));
        assert!(!audio_channel_maps_equal(2, None, Some(&[0, 1])));
        let s = AudioSpec::new(AudioFormat::S16, 2, 48000);
        assert!(audio_specs_equal(&s, &s, None, None));
        assert!(!audio_specs_equal(&s, &s, Some(&[0, 1]), None));
        assert!(audio_specs_equal(
            &s,
            &s,
            Some(&[1, 0, 9]),
            Some(&[1, 0, 7])
        ));
    }
}
