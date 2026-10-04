// Audio drivers, one module per upstream driver directory. The platform
// drivers are compiled for their platform only, as upstream's
// SDL_AUDIO_DRIVER_* configuration does; the system libraries they use are
// loaded at run time. (CoreAudio, AAudio and the others are still to come.)

#[cfg(target_os = "linux")]
pub(crate) mod alsa;
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

#[cfg(test)]
pub(crate) mod tests;
