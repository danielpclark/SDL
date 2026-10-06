// placeholder
use crate::internal::{AudioData, Decoder};
use sdl3::audio::AudioSpec;
use sdl3::error::{Error, Result};
use sdl3::io::IoStream;
use sdl3::properties::Properties;
use std::sync::Arc;
fn init_audio(
    _io: Option<&mut IoStream<'_>>,
    _spec: &mut AudioSpec,
    _props: &Properties,
    _d: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    Err(Error::unsupported())
}
pub(crate) static DECODER: Decoder = Decoder {
    name: "STBVORBIS",
    init: None,
    init_audio,
    has_jump_to_order: false,
    quit: None,
};
