// Audio drivers. Platform drivers (PipeWire, PulseAudio, ALSA, WASAPI,
// CoreAudio, ...) arrive with the platform layer.

pub(crate) mod disk;
pub(crate) mod dummy;

/// `io_delay` for a device: `(sample_frames * 1000) / freq` milliseconds,
/// scaled by a timescale hint (`SDL_atof`, rounded with `SDL_round`).
pub(super) fn io_delay(sample_frames: i32, freq: i32, timescale_hint: &str) -> u32 {
    let mut io_delay = ((sample_frames as i64 * 1000) / freq.max(1) as i64) as u32;

    if let Some(hint) = crate::hints::get(timescale_hint) {
        let scale = crate::stdlib::atof(&hint);
        if scale >= 0.0 {
            io_delay = crate::stdlib::math::round(io_delay as f64 * scale) as u32;
        }
    }
    io_delay
}
