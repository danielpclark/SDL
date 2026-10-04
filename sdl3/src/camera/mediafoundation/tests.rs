// Tests for the Media Foundation camera driver. The expected values come
// from upstream's C (the format table, GetMediaTypeColorspace() and
// MediaTypeToSDLFmt() over fake media types, and the SDK's GUIDs and
// constants), built with mingw-w64 and run under Wine.

use std::cell::{Cell, RefCell};
use std::mem::{offset_of, size_of};

use super::*;
use crate::camera::{
    camera_name, camera_supported_formats, cameras, current_camera_driver, Camera,
};
use crate::core::windows::is_wine;
use crate::init::{self, InitFlags};

/// `MF_E_ATTRIBUTENOTFOUND`.
const MF_E_ATTRIBUTENOTFOUND: HRESULT = 0xc00d_36e6_u32 as HRESULT;

fn guid_string(g: &GUID) -> String {
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        g.data1,
        g.data2,
        g.data3,
        g.data4[0],
        g.data4[1],
        g.data4[2],
        g.data4[3],
        g.data4[4],
        g.data4[5],
        g.data4[6],
        g.data4[7]
    )
}

#[test]
fn format_table_matches_the_c_driver() {
    // "map %d 0x%08x 0x%08x <guid>" of fmtmappings[].
    const EXPECTED: [(u32, u32, &str); 14] = [
        (
            0x15130f02,
            0x120005a0,
            "00000018-0000-0010-8000-00aa00389b71",
        ),
        (
            0x15151002,
            0x120005a0,
            "00000017-0000-0010-8000-00aa00389b71",
        ),
        (
            0x17101803,
            0x120005a0,
            "00000014-0000-0010-8000-00aa00389b71",
        ),
        (
            0x16161804,
            0x120005a0,
            "00000016-0000-0010-8000-00aa00389b71",
        ),
        (
            0x16362004,
            0x120005a0,
            "00000015-0000-0010-8000-00aa00389b71",
        ),
        (
            0x16372004,
            0x120005a0,
            "0000001f-0000-0010-8000-00aa00389b71",
        ),
        (
            0x32315659,
            0x21100421,
            "32315659-0000-0010-8000-00aa00389b71",
        ),
        (
            0x56555949,
            0x21100421,
            "56555949-0000-0010-8000-00aa00389b71",
        ),
        (
            0x32595559,
            0x21100421,
            "32595559-0000-0010-8000-00aa00389b71",
        ),
        (
            0x59565955,
            0x21100421,
            "59565955-0000-0010-8000-00aa00389b71",
        ),
        (
            0x55595659,
            0x21100421,
            "55595659-0000-0010-8000-00aa00389b71",
        ),
        (
            0x3231564e,
            0x21100421,
            "3231564e-0000-0010-8000-00aa00389b71",
        ),
        (
            0x3132564e,
            0x21100421,
            "3132564e-0000-0010-8000-00aa00389b71",
        ),
        (
            0x47504a4d,
            0x120005a0,
            "47504a4d-0000-0010-8000-00aa00389b71",
        ),
    ];
    for (m, (format, colorspace, guid)) in FMTMAPPINGS.iter().zip(EXPECTED) {
        assert_eq!(m.format.0, format, "{guid}");
        assert_eq!(m.colorspace.0, colorspace, "{guid}");
        assert_eq!(guid_string(&m.guid), guid);
        // and back.
        assert_eq!(
            sdl_fmt_to_mf_vid_fmt_guid(m.format).map(guid_string),
            Some(guid.into())
        );
    }
    assert!(sdl_fmt_to_mf_vid_fmt_guid(PixelFormat::RGBA8888).is_none());
    assert!(sdl_fmt_to_mf_vid_fmt_guid(PixelFormat::UNKNOWN).is_none());
}

#[test]
fn guids_and_constants_match_the_sdk() {
    // "sdk <name> <guid>": the SDK's own GUIDs, which SDL keeps copies of.
    assert_eq!(
        guid_string(&MFVIDEOFORMAT_NV12),
        "3231564e-0000-0010-8000-00aa00389b71"
    );
    assert_eq!(
        guid_string(&MFVIDEOFORMAT_RGB32),
        "00000016-0000-0010-8000-00aa00389b71"
    );
    assert_eq!(
        guid_string(&MF_MT_SUBTYPE),
        "f7e34c9a-42e8-4714-b74b-cb29d72c35e5"
    );
    assert_eq!(
        guid_string(&MF_MT_FRAME_SIZE),
        "1652c33d-d6b2-4012-b834-72030849a37d"
    );
    assert_eq!(
        guid_string(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK),
        "58f0aad8-22bf-4f8a-bb3d-d2c4978c6e2f"
    );
    assert_eq!(
        guid_string(&IID_IMF2DBUFFER2),
        "33ae5ea6-4316-436f-8ddd-d73d22f829ec"
    );
    // The other GUIDs, as upstream spells them out.
    assert_eq!(
        guid_string(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME),
        "60d0e559-52f8-4fa2-bbce-acdb34a8ec01"
    );
    assert_eq!(
        guid_string(&MFMEDIATYPE_VIDEO),
        "73646976-0000-0010-8000-00aa00389b71"
    );
    assert_eq!(
        guid_string(&IID_IMFMEDIASOURCE),
        "279a808d-aec7-40c8-9c6b-a6b492c78a66"
    );

    // "chroma MPEG2=5 MPEG1=1 DV_PAL=6"
    assert_eq!(MF_VIDEO_CHROMA_SUBSAMPLING_MPEG2, 5);
    assert_eq!(MF_VIDEO_CHROMA_SUBSAMPLING_MPEG1, 1);
    assert_eq!(MF_VIDEO_CHROMA_SUBSAMPLING_DV_PAL, 6);
    // "MF_VERSION=0x20070 MFSTARTUP_LITE=1 first_video=0xfffffffc"
    assert_eq!(MF_VERSION, 0x20070);
    assert_eq!(MFSTARTUP_LITE, 1);
    assert_eq!(MF_SOURCE_READER_FIRST_VIDEO_STREAM, 0xfffffffc);
    // "readerf ERROR=1 EOS=2 lock_read=1"
    assert_eq!(MF_SOURCE_READERF_ERROR, 1);
    assert_eq!(MF_SOURCE_READERF_ENDOFSTREAM, 2);
    assert_eq!(MF2DBUFFER_LOCKFLAGS_READ, 1);
}

#[test]
fn vtable_layouts_match_the_headers() {
    // Slot numbers (counting IUnknown's three) from mfobjects.h, mfidl.h
    // and mfreadwrite.h.
    const P: usize = size_of::<usize>();
    assert_eq!(offset_of!(IMFAttributesVtbl, get_uint32), 7 * P);
    assert_eq!(offset_of!(IMFAttributesVtbl, get_uint64), 8 * P);
    assert_eq!(offset_of!(IMFAttributesVtbl, get_guid), 10 * P);
    assert_eq!(offset_of!(IMFAttributesVtbl, get_allocated_string), 13 * P);
    assert_eq!(offset_of!(IMFAttributesVtbl, set_uint32), 21 * P);
    assert_eq!(offset_of!(IMFAttributesVtbl, set_uint64), 22 * P);
    assert_eq!(offset_of!(IMFAttributesVtbl, set_guid), 24 * P);
    assert_eq!(offset_of!(IMFAttributesVtbl, set_string), 25 * P);
    assert_eq!(size_of::<IMFAttributesVtbl>(), 33 * P);

    assert_eq!(offset_of!(IMFActivateVtbl, activate_object), 33 * P);
    assert_eq!(offset_of!(IMFActivateVtbl, shutdown_object), 34 * P);
    assert_eq!(size_of::<IMFActivateVtbl>(), 36 * P);

    assert_eq!(
        offset_of!(IMFMediaSourceVtbl, create_presentation_descriptor),
        8 * P
    );
    assert_eq!(offset_of!(IMFMediaSourceVtbl, shutdown), 12 * P);
    assert_eq!(size_of::<IMFMediaSourceVtbl>(), 13 * P);

    assert_eq!(
        offset_of!(IMFPresentationDescriptorVtbl, get_stream_descriptor_count),
        33 * P
    );
    assert_eq!(
        offset_of!(
            IMFPresentationDescriptorVtbl,
            get_stream_descriptor_by_index
        ),
        34 * P
    );
    assert_eq!(
        offset_of!(IMFStreamDescriptorVtbl, get_media_type_handler),
        34 * P
    );
    assert_eq!(
        offset_of!(IMFMediaTypeHandlerVtbl, get_media_type_count),
        4 * P
    );
    assert_eq!(
        offset_of!(IMFMediaTypeHandlerVtbl, get_media_type_by_index),
        5 * P
    );
    assert_eq!(
        offset_of!(IMFSourceReaderVtbl, set_current_media_type),
        7 * P
    );
    assert_eq!(offset_of!(IMFSourceReaderVtbl, read_sample), 9 * P);
    assert_eq!(offset_of!(IMFSampleVtbl, get_sample_time), 35 * P);
    assert_eq!(
        offset_of!(IMFSampleVtbl, convert_to_contiguous_buffer),
        41 * P
    );
    assert_eq!(offset_of!(IMFMediaBufferVtbl, lock), 3 * P);
    assert_eq!(offset_of!(IMFMediaBufferVtbl, unlock), 4 * P);
    assert_eq!(offset_of!(IMF2DBufferVtbl, lock_2d), 3 * P);
    assert_eq!(offset_of!(IMF2DBufferVtbl, unlock_2d), 4 * P);
    assert_eq!(size_of::<IMF2DBufferVtbl>(), 10 * P);
    assert_eq!(offset_of!(IMF2DBuffer2Vtbl, lock_2d_size), 10 * P);
}

/// A media type with a few attributes (`MF_E_ATTRIBUTENOTFOUND` for the
/// rest), as the C reference fakes them.
#[derive(Default)]
struct FakeMediaType {
    guids: Vec<(GUID, GUID)>,
    uint32s: RefCell<Vec<(GUID, u32)>>,
    uint64s: Vec<(GUID, u64)>,
}

impl FakeMediaType {
    fn with_subtype(subtype: GUID) -> FakeMediaType {
        FakeMediaType {
            guids: vec![(MF_MT_SUBTYPE, subtype)],
            ..FakeMediaType::default()
        }
    }

    fn uint32(self, key: GUID, value: Option<u32>) -> FakeMediaType {
        if let Some(value) = value {
            self.uint32s.borrow_mut().push((key, value));
        }
        self
    }

    fn uint64(mut self, key: GUID, value: u64) -> FakeMediaType {
        self.uint64s.push((key, value));
        self
    }

    fn guid(mut self, key: GUID, value: GUID) -> FakeMediaType {
        self.guids.push((key, value));
        self
    }
}

fn lookup<T: Copy>(list: &[(GUID, T)], key: &GUID) -> std::result::Result<T, HRESULT> {
    list.iter()
        .find(|(k, _)| same_guid(k, key))
        .map(|&(_, v)| v)
        .ok_or(MF_E_ATTRIBUTENOTFOUND)
}

impl MediaTypeAttributes for FakeMediaType {
    fn get_uint32(&self, key: &GUID) -> std::result::Result<u32, HRESULT> {
        lookup(&self.uint32s.borrow(), key)
    }
    fn get_uint64(&self, key: &GUID) -> std::result::Result<u64, HRESULT> {
        lookup(&self.uint64s, key)
    }
    fn get_guid(&self, key: &GUID) -> std::result::Result<GUID, HRESULT> {
        lookup(&self.guids, key)
    }
    fn set_uint32(&self, key: &GUID, value: u32) -> std::result::Result<(), HRESULT> {
        let mut list = self.uint32s.borrow_mut();
        list.retain(|(k, _)| !same_guid(k, key));
        list.push((*key, value));
        Ok(())
    }
}

#[test]
fn media_type_formats_match_the_c_driver() {
    // run(tag, subtype, range, primaries, transfer, matrix, chroma) of the
    // C reference (None: the attribute isn't set), and what
    // MediaTypeToSDLFmt() gave.
    #[allow(clippy::type_complexity)]
    let cases: [(&str, GUID, [Option<u32>; 5], u32, u32); 17] = [
        (
            "nv12-none",
            MFVIDEOFORMAT_NV12,
            [None; 5],
            0x3231564e,
            0x21100421,
        ),
        (
            "nv12-full",
            MFVIDEOFORMAT_NV12,
            [Some(1), Some(9), Some(15), Some(4), Some(5)],
            0x3231564e,
            0x22102609,
        ),
        (
            "yuy2-601",
            MFVIDEOFORMAT_YUY2,
            [Some(2), Some(5), Some(5), Some(2), Some(1)],
            0x32595559,
            0x21201826,
        ),
        (
            "yv12-odd",
            MFVIDEOFORMAT_YV12,
            [Some(7), Some(11), Some(19), Some(12), Some(6)],
            0x32315659,
            0x2130322e,
        ),
        (
            "iyuv-misc",
            MFVIDEOFORMAT_IYUV,
            [Some(0), Some(10), Some(18), Some(6), Some(99)],
            0x56555949,
            0x21102980,
        ),
        (
            "uyvy-a",
            MFVIDEOFORMAT_UYVY,
            [None, Some(3), Some(1), Some(7), None],
            0x59565955,
            0x21101104,
        ),
        (
            "yvyu-b",
            MFVIDEOFORMAT_YVYU,
            [None, Some(4), Some(4), Some(8), None],
            0x55595659,
            0x21101488,
        ),
        (
            "nv21-c",
            MFVIDEOFORMAT_NV21,
            [None, Some(6), Some(6), Some(9), None],
            0x3132564e,
            0x21101ceb,
        ),
        (
            "nv12-d",
            MFVIDEOFORMAT_NV12,
            [None, Some(7), Some(7), Some(10), None],
            0x3231564e,
            0x211059ac,
        ),
        (
            "nv12-e",
            MFVIDEOFORMAT_NV12,
            [None, Some(2), Some(8), Some(11), None],
            0x3231564e,
            0x211004ad,
        ),
        (
            "nv12-f",
            MFVIDEOFORMAT_NV12,
            [None, Some(100), Some(9), Some(3), None],
            0x3231564e,
            0x21100527,
        ),
        (
            "nv12-g",
            MFVIDEOFORMAT_NV12,
            [None, None, Some(16), Some(1), None],
            0x3231564e,
            0x21100641,
        ),
        (
            "nv12-h",
            MFVIDEOFORMAT_NV12,
            [None, None, Some(77), Some(77), None],
            0x3231564e,
            0x21100421,
        ),
        (
            "rgb32-full",
            MFVIDEOFORMAT_RGB32,
            [Some(2), Some(9), None, None, None],
            0x16161804,
            0x120005a0,
        ),
        (
            "mjpg",
            MFVIDEOFORMAT_MJPG,
            [Some(2), None, None, None, None],
            0x47504a4d,
            0x120005a0,
        ),
        (
            "argb",
            MFVIDEOFORMAT_ARGB32,
            [None; 5],
            0x16362004,
            0x120005a0,
        ),
        ("h264", mediatype_guid(fcc(b"H264")), [None; 5], 0, 0),
    ];
    let keys = [
        MF_MT_VIDEO_NOMINAL_RANGE,
        MF_MT_VIDEO_PRIMARIES,
        MF_MT_TRANSFER_FUNCTION,
        MF_MT_YUV_MATRIX,
        MF_MT_VIDEO_CHROMA_SITING,
    ];
    for (tag, subtype, values, format, colorspace) in cases {
        let mut mt = FakeMediaType::with_subtype(subtype);
        for (key, value) in keys.iter().zip(values) {
            mt = mt.uint32(*key, value);
        }
        let (f, c) = media_type_to_sdl_fmt(&mt);
        assert_eq!((f.0, c.0), (format, colorspace), "{tag}");
    }

    // Without a subtype, the format is unknown.
    assert_eq!(
        media_type_to_sdl_fmt(&FakeMediaType::default()),
        (PixelFormat::UNKNOWN, Colorspace::UNKNOWN)
    );
}

#[test]
fn frame_sizes_and_rates() {
    assert_eq!(pack_attribute_pair(640, 480), 0x0000_0280_0000_01e0);
    assert_eq!(unpack_attribute_pair(0x0000_0280_0000_01e0), (640, 480));
    assert_eq!(pack_attribute_pair(30000, 1001), (30000 << 32) | 1001);
    // (int to UINT64, as upstream: a negative value sign-extends)
    assert_eq!(pack_attribute_pair(0, -1), u64::MAX);

    let video = |w: u32, h: u32, num: i32, den: i32| {
        FakeMediaType::with_subtype(MFVIDEOFORMAT_YUY2)
            .guid(MF_MT_MAJOR_TYPE, MFMEDIATYPE_VIDEO)
            .uint64(MF_MT_FRAME_SIZE, pack_attribute_pair(w as i32, h as i32))
            .uint64(MF_MT_FRAME_RATE, pack_attribute_pair(num, den))
    };
    assert_eq!(
        media_type_spec(&video(1280, 720, 30000, 1001)),
        Some(CameraSpec {
            format: PixelFormat::YUY2,
            colorspace: Colorspace::BT709_LIMITED,
            width: 1280,
            height: 720,
            framerate_numerator: 30000,
            framerate_denominator: 1001,
        })
    );
    // No size, no frame rate, or a zero in either: skipped.
    assert_eq!(media_type_spec(&video(0, 720, 30, 1)), None);
    assert_eq!(media_type_spec(&video(1280, 0, 30, 1)), None);
    assert_eq!(media_type_spec(&video(1280, 720, 0, 1)), None);
    assert_eq!(media_type_spec(&video(1280, 720, 30, 0)), None);
    let no_rate = FakeMediaType::with_subtype(MFVIDEOFORMAT_YUY2)
        .guid(MF_MT_MAJOR_TYPE, MFMEDIATYPE_VIDEO)
        .uint64(MF_MT_FRAME_SIZE, pack_attribute_pair(640, 480));
    assert_eq!(media_type_spec(&no_rate), None);
    let no_size = FakeMediaType::with_subtype(MFVIDEOFORMAT_YUY2)
        .guid(MF_MT_MAJOR_TYPE, MFMEDIATYPE_VIDEO)
        .uint64(MF_MT_FRAME_RATE, pack_attribute_pair(30, 1));
    assert_eq!(media_type_spec(&no_size), None);
    // Not video, or a format SDL doesn't handle: skipped.
    let audio = FakeMediaType::with_subtype(MFVIDEOFORMAT_YUY2)
        .guid(
            MF_MT_MAJOR_TYPE,
            GUID::from_u128(0x73647561_0000_0010_8000_00aa00389b71),
        )
        .uint64(MF_MT_FRAME_SIZE, pack_attribute_pair(640, 480))
        .uint64(MF_MT_FRAME_RATE, pack_attribute_pair(30, 1));
    assert_eq!(media_type_spec(&audio), None);
    let h264 = FakeMediaType::with_subtype(mediatype_guid(fcc(b"H264")))
        .guid(MF_MT_MAJOR_TYPE, MFMEDIATYPE_VIDEO)
        .uint64(MF_MT_FRAME_SIZE, pack_attribute_pair(640, 480))
        .uint64(MF_MT_FRAME_RATE, pack_attribute_pair(30, 1));
    assert_eq!(media_type_spec(&h264), None);
}

#[test]
fn default_stride() {
    // Set on the media type: used as is (a LONG stored as a UINT32).
    let mt = FakeMediaType::with_subtype(MFVIDEOFORMAT_RGB32)
        .uint32(MF_MT_DEFAULT_STRIDE, Some(-2560i32 as u32));
    let called = Cell::new(false);
    let stride = get_default_stride(&mt, |_, _| {
        called.set(true);
        Ok(0)
    });
    assert_eq!(stride, Ok(-2560));
    assert!(!called.get());

    // Not set: computed from the subtype's FourCC and the width, and kept.
    let mt = FakeMediaType::with_subtype(MFVIDEOFORMAT_NV12)
        .uint64(MF_MT_FRAME_SIZE, pack_attribute_pair(640, 480));
    let stride = get_default_stride(&mt, |format, width| {
        assert_eq!((format, width), (fcc(b"NV12"), 640));
        Ok(640)
    });
    assert_eq!(stride, Ok(640));
    assert_eq!(mt.get_uint32(&MF_MT_DEFAULT_STRIDE), Ok(640));

    // A failure along the way is the result.
    let mt = FakeMediaType::default().uint64(MF_MT_FRAME_SIZE, pack_attribute_pair(640, 480));
    assert_eq!(
        get_default_stride(&mt, |_, _| Ok(1)),
        Err(MF_E_ATTRIBUTENOTFOUND)
    );
    let mt = FakeMediaType::with_subtype(MFVIDEOFORMAT_NV12);
    assert_eq!(
        get_default_stride(&mt, |_, _| Ok(1)),
        Err(MF_E_ATTRIBUTENOTFOUND)
    );
    let mt = FakeMediaType::with_subtype(MFVIDEOFORMAT_NV12)
        .uint64(MF_MT_FRAME_SIZE, pack_attribute_pair(640, 480));
    assert_eq!(
        get_default_stride(&mt, |_, _| Err(E_POINTER)),
        Err(E_POINTER)
    );
    assert!(mt.get_uint32(&MF_MT_DEFAULT_STRIDE).is_err());
}

#[test]
fn frame_copies() {
    let buffer: Vec<u8> = (0..12).collect();

    // Top-down rows: buflen bytes from the top row, with the pitch.
    assert_eq!(copy_frame(&buffer, 0, 4, 3, 12), Some((buffer.clone(), 4)));
    // (MJPG: the pitch is the length of the data)
    assert_eq!(
        copy_frame(&buffer, 0, 7, 3, 7),
        Some((buffer[..7].to_vec(), 7))
    );
    // A copy that would read past the buffer stops at its end.
    assert_eq!(
        copy_frame(&buffer, 4, 4, 3, 12),
        Some((buffer[4..].to_vec(), 4))
    );
    assert_eq!(copy_frame(&buffer, 13, 4, 3, 12), None);

    // Bottom-up rows (the top row is last in memory): copied top-down,
    // with the positive pitch.
    let expected: Vec<u8> = [8, 9, 10, 11, 4, 5, 6, 7, 0, 1, 2, 3].into();
    assert_eq!(copy_frame(&buffer, 8, -4, 3, 12), Some((expected, 4)));
    // (rows outside the buffer: no frame)
    assert_eq!(copy_frame(&buffer, 4, -4, 3, 12), None);
    assert_eq!(copy_frame(&buffer, 10, -4, 3, 12), None);

    // Timestamps are in 100-nanosecond units.
    assert_eq!(sample_time_to_ns(0), 0);
    assert_eq!(sample_time_to_ns(333_667), 33_366_700);
}

/// The libraries, started up, or `None` (with a note) where they're
/// missing (Windows Server without the Media Foundation feature); Wine
/// always has them.
fn started_lib() -> Option<MfLib> {
    match MfLib::load() {
        Ok(lib) => {
            // SAFETY: MFStartup takes a version and flags; balanced by the
            // caller's MFShutdown.
            let ret = unsafe { (lib.startup)(MF_VERSION, MFSTARTUP_LITE) };
            assert_eq!(check_hr(ret), Ok(()));
            Some(lib)
        }
        Err(e) => {
            assert!(!is_wine(), "{}", e.message());
            crate::test_support::skip(
                "mediafoundation",
                format_args!("Media Foundation isn't available here: {}", e.message()),
            );
            None
        }
    }
}

#[test]
fn media_types_from_media_foundation() {
    let _l = crate::test_support::test_lock();
    let Some(lib) = started_lib() else {
        return;
    };

    // The format mapping over a real IMFMediaType.
    let mt = lib.create_media_type().unwrap();
    mt.set_guid(&MF_MT_MAJOR_TYPE, &MFMEDIATYPE_VIDEO).unwrap();
    mt.set_guid(&MF_MT_SUBTYPE, &MFVIDEOFORMAT_NV12).unwrap();
    mt.set_uint64(&MF_MT_FRAME_SIZE, pack_attribute_pair(640, 480))
        .unwrap();
    mt.set_uint64(&MF_MT_FRAME_RATE, pack_attribute_pair(30000, 1001))
        .unwrap();
    mt.set_uint32(&MF_MT_VIDEO_NOMINAL_RANGE, MF_NOMINAL_RANGE_0_255)
        .unwrap();
    assert_eq!(
        mt.get_uint32(&MF_MT_YUV_MATRIX),
        Err(MF_E_ATTRIBUTENOTFOUND)
    );
    let spec = media_type_spec(&mt).unwrap();
    assert_eq!(spec.format, PixelFormat::NV12);
    assert_eq!(spec.colorspace, Colorspace::BT709_FULL);
    assert_eq!((spec.width, spec.height), (640, 480));
    assert_eq!(
        (spec.framerate_numerator, spec.framerate_denominator),
        (30000, 1001)
    );

    // The default stride from MFGetStrideForBitmapInfoHeader(), kept in
    // the media type.
    let stride = get_default_stride(&mt, |format, width| {
        lib.get_stride_for_bitmap_info_header(format, width)
    })
    .unwrap();
    assert_eq!(stride, 640);
    assert_eq!(mt.get_uint32(&MF_MT_DEFAULT_STRIDE), Ok(640));
    let rgb32 = lib.get_stride_for_bitmap_info_header(22, 640);
    assert_eq!(rgb32.map(i32::unsigned_abs), Ok(2560));

    // Strings, as the device's name and symlink are read.
    let attrs = lib.create_attributes(1).unwrap();
    let name = "Caméra \u{2713}";
    attrs
        .set_string(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &utf8_to_wide(name))
        .unwrap();
    assert_eq!(
        attrs.get_allocated_string(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME),
        Ok(name.to_owned())
    );
    assert_eq!(
        attrs.get_allocated_string(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK),
        Err(MF_E_ATTRIBUTENOTFOUND)
    );

    drop((mt, attrs));
    // SAFETY: balances started_lib()'s MFStartup().
    unsafe { (lib.shutdown)() };
}

#[test]
fn driver_starts_without_cameras() {
    let _l = crate::test_support::test_lock();
    crate::hints::set(crate::hints::CAMERA_DRIVER, "mediafoundation").unwrap();
    let r = init::init_subsystem(InitFlags::CAMERA);
    crate::hints::reset(crate::hints::CAMERA_DRIVER);
    if let Err(e) = r {
        assert!(!is_wine(), "{}", e.message());
        crate::test_support::skip(
            "mediafoundation",
            format_args!("Media Foundation isn't available here: {}", e.message()),
        );
        return;
    }
    assert_eq!(current_camera_driver(), Some("mediafoundation"));

    // (there may be none: Wine and CI machines have no camera)
    let ids = cameras().unwrap();
    eprintln!("note: Media Foundation cameras here: {}", ids.len());
    for &id in &ids {
        let specs = camera_supported_formats(id).unwrap();
        eprintln!(
            "note:   {} ({} specs)",
            camera_name(id).unwrap(),
            specs.len()
        );
        assert!(!specs.is_empty());
        for spec in specs {
            assert!(sdl_fmt_to_mf_vid_fmt_guid(spec.format).is_some());
            assert!(spec.width > 0 && spec.height > 0);
            assert!(spec.framerate_numerator > 0 && spec.framerate_denominator > 0);
        }
    }

    // A device that isn't there fails to open, and stays closed.
    let device = add_camera(
        "No Such Camera",
        CameraPosition::Unknown,
        &[CameraSpec {
            format: PixelFormat::NV12,
            colorspace: Colorspace::BT709_LIMITED,
            width: 640,
            height: 480,
            framerate_numerator: 30,
            framerate_denominator: 1,
        }],
        Box::new(MfDeviceHandle {
            symlink: r"\\?\sdl#no-such-camera".to_owned(),
        }),
    )
    .unwrap();
    // (Wine's MFCreateDeviceSource() is an unimplemented stub, which
    // aborts the process when called.)
    if !is_wine() {
        for _ in 0..2 {
            let err = Camera::open(device.instance_id, None).unwrap_err();
            eprintln!("note: opening a missing camera: {}", err.message());
            assert!(err.message().contains(" failed"), "{}", err.message());
        }
    }

    init::quit_subsystem(InitFlags::CAMERA);
    assert_eq!(current_camera_driver(), None);
}

/// Hardware: grab frames from the first Media Foundation camera (a webcam).
#[test]
#[ignore = "hardware: needs a camera"]
fn hardware_capture_from_the_first_camera() {
    let _l = crate::test_support::test_lock();
    crate::camera::tests::hardware_capture("mediafoundation", "mediafoundation");
}
