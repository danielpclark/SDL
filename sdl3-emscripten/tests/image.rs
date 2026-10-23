// Tests of the preloaded images (src/image.rs).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Under node, which decodes no image: the canvases emscripten's image
//! plugin would keep in `Browser.preloadedImages` are stood in for (what
//! emscripten reads of one: its size and `getImageData()`). This program is
//! linked with `-sEXPORTED_FUNCTIONS=_main` (build.rs), as an application
//! with its own export list is, and `emscripten_get_preloaded_image_data_from_FILE()`
//! still finds the `fileno()` it calls.

#![cfg(target_os = "emscripten")]

use sdl3_emscripten::em_asm;
use sdl3_emscripten::image::PreloadedImage;
use std::fs::File;
use std::io::Read;
use std::os::fd::AsRawFd;

/// Stand-ins for `/standin.png` (3×2, the bytes 0, 1, 2, ...) and the
/// empty `/tmp/empty.png`.
fn standins() {
    // SAFETY: plain JavaScript.
    unsafe {
        em_asm!(
            r#"
            function standin(w, h) {
                var data = new Uint8ClampedArray(w * h * 4);
                for (var i = 0; i < data.length; i++) data[i] = i;
                return { width: w, height: h, getContext: (type) => ({
                    getImageData: (x, y, cw, ch) => ({ width: cw, height: ch, data: data }),
                }) };
            }
            Browser.preloadedImages['/standin.png'] = standin(3, 2);
            Browser.preloadedImages['/tmp/empty.png'] = standin(0, 0);
        "#
        )
    };
}

fn rgba(image: &PreloadedImage) -> (i32, i32, Vec<u8>) {
    (image.width(), image.height(), image.pixels().to_vec())
}

#[test]
fn of_path() {
    standins();
    let expected = (3, 2, (0..24).collect::<Vec<u8>>());
    let image = PreloadedImage::of_path(c"/standin.png").unwrap();
    assert_eq!(rgba(&image), expected);
    assert_eq!(
        format!("{image:?}"),
        "PreloadedImage { width: 3, height: 2, .. }"
    );
    // resolved against the current directory, `/`
    assert_eq!(
        rgba(&PreloadedImage::of_path(c"standin.png").unwrap()),
        expected
    );
    assert_eq!(
        rgba(&PreloadedImage::of_path(c"/tmp/../standin.png").unwrap()),
        expected
    );
    assert_eq!(
        rgba(&PreloadedImage::of_path(c"/tmp/empty.png").unwrap()),
        (0, 0, vec![])
    );
    assert!(PreloadedImage::of_path(c"/elsewhere.png").is_none());
    assert!(PreloadedImage::of_path(c"").is_none());
}

#[test]
fn of_fd() {
    standins();
    std::fs::write("/standin.png", b"not an image").unwrap();
    let mut file = File::open("/standin.png").unwrap();
    let image = PreloadedImage::of_fd(file.as_raw_fd()).unwrap();
    assert_eq!(rgba(&image), (3, 2, (0..24).collect::<Vec<u8>>()));
    // The descriptor is still the file's, unread.
    let mut contents = Vec::new();
    file.read_to_end(&mut contents).unwrap();
    assert_eq!(contents, b"not an image");

    // A file that wasn't preloaded, and descriptors that aren't open.
    std::fs::write("/tmp/plain.png", b"").unwrap();
    let plain = File::open("/tmp/plain.png").unwrap();
    assert!(PreloadedImage::of_fd(plain.as_raw_fd()).is_none());
    assert!(PreloadedImage::of_fd(-1).is_none());
    assert!(PreloadedImage::of_fd(4000).is_none());
    drop(plain);
    drop(file);
    std::fs::remove_file("/standin.png").unwrap();
    std::fs::remove_file("/tmp/plain.png").unwrap();
}
