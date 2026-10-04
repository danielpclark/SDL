// Rust translation of the static inline SPA helpers SDL_pipewire.c uses,
// from PipeWire 1.2's spa/pod/builder.h, spa/pod/iter.h, spa/pod/parser.h,
// spa/param/audio/raw-utils.h, spa/utils/json.h and spa/utils/dict.h.
// Copyright © 2018-2020 Wim Taymans (MIT); this is a translated version.

// These are header-only in C, so a C program compiles them in; here they
// work on byte slices instead of raw pod pointers. A pod is its 8-byte
// header (`size` of the body, `type`) followed by the body, padded to 8
// bytes inside containers.

use std::ffi::{c_char, CStr};

// enum spa_type
pub(super) const SPA_TYPE_NONE: u32 = 1;
pub(super) const SPA_TYPE_ID: u32 = 3;
pub(super) const SPA_TYPE_INT: u32 = 4;
pub(super) const SPA_TYPE_ARRAY: u32 = 13;
pub(super) const SPA_TYPE_OBJECT: u32 = 15;
pub(super) const SPA_TYPE_CHOICE: u32 = 19;
pub(super) const SPA_TYPE_OBJECT_FORMAT: u32 = 0x40003;

// enum spa_choice_type
pub(super) const SPA_CHOICE_NONE: u32 = 0;
pub(super) const SPA_CHOICE_RANGE: u32 = 1;

// enum spa_format
pub(super) const SPA_FORMAT_MEDIA_TYPE: u32 = 1;
pub(super) const SPA_FORMAT_MEDIA_SUBTYPE: u32 = 2;
pub(super) const SPA_FORMAT_AUDIO_FORMAT: u32 = 0x10001;
pub(super) const SPA_FORMAT_AUDIO_RATE: u32 = 0x10003;
pub(super) const SPA_FORMAT_AUDIO_CHANNELS: u32 = 0x10004;
pub(super) const SPA_FORMAT_AUDIO_POSITION: u32 = 0x10005;

pub(super) const SPA_MEDIA_TYPE_AUDIO: u32 = 1;
pub(super) const SPA_MEDIA_SUBTYPE_RAW: u32 = 1;

pub(super) const SPA_AUDIO_FORMAT_UNKNOWN: u32 = 0;
pub(super) const SPA_AUDIO_MAX_CHANNELS: usize = 64;
pub(super) const SPA_AUDIO_FLAG_UNPOSITIONED: u32 = 1 << 0;

const EINVAL: i32 = 22;
const ENOSPC: i32 = 28;
const EPIPE: i32 = 32;
const EPROTO: i32 = 71;
const ESRCH: i32 = 3;

/// `SPA_ROUND_UP_N(num, 8)`.
fn round_up_8(num: u64) -> u64 {
    (num + 7) & !7
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4)
        .map(|b| u32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
}

/// `struct spa_audio_info_raw`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SpaAudioInfoRaw {
    pub(super) format: u32,
    pub(super) flags: u32,
    pub(super) rate: u32,
    pub(super) channels: u32,
    pub(super) position: [u32; SPA_AUDIO_MAX_CHANNELS],
}

impl Default for SpaAudioInfoRaw {
    fn default() -> Self {
        SpaAudioInfoRaw {
            format: SPA_AUDIO_FORMAT_UNKNOWN,
            flags: 0,
            rate: 0,
            channels: 0,
            position: [0; SPA_AUDIO_MAX_CHANNELS],
        }
    }
}

// --- spa/pod/builder.h ---

/// `struct spa_pod_frame` (of a builder): the container's header so far
/// and where it starts.
struct BuilderFrame {
    pod_size: u32,
    pod_type: u32,
    offset: u32,
    flags: u32,
}

const SPA_POD_BUILDER_FLAG_BODY: u32 = 1 << 0;
const SPA_POD_BUILDER_FLAG_FIRST: u32 = 1 << 1;

/// `struct spa_pod_builder` over a fixed buffer (no overflow callbacks).
/// The C frames are a linked list on the caller's stack; here a stack.
pub(super) struct SpaPodBuilder<'a> {
    data: &'a mut [u8],
    offset: u32,
    flags: u32,
    frames: Vec<BuilderFrame>,
}

impl<'a> SpaPodBuilder<'a> {
    /// `SPA_POD_BUILDER_INIT(buffer, size)`.
    pub(super) fn new(data: &'a mut [u8]) -> SpaPodBuilder<'a> {
        SpaPodBuilder {
            data,
            offset: 0,
            flags: 0,
            frames: Vec::new(),
        }
    }

    fn size(&self) -> u32 {
        self.data.len() as u32
    }

    /// `spa_pod_builder_raw()`.
    fn raw(&mut self, data: &[u8]) -> i32 {
        let mut res = 0;
        let offset = self.offset;
        let size = data.len() as u32;

        if offset as u64 + size as u64 > self.size() as u64 {
            res = -ENOSPC;
            // (no overflow callbacks are set)
        }
        if res == 0 {
            self.data[offset as usize..(offset + size) as usize].copy_from_slice(data);
        }

        self.offset += size;

        for f in &mut self.frames {
            f.pod_size += size;
        }

        res
    }

    /// `spa_pod_builder_pad()`.
    fn pad(&mut self, size: u32) -> i32 {
        let zeroes = [0u8; 8];
        let size = (round_up_8(size as u64) - size as u64) as usize;
        if size != 0 {
            self.raw(&zeroes[..size])
        } else {
            0
        }
    }

    /// `spa_pod_builder_raw_padded()`.
    fn raw_padded(&mut self, data: &[u8]) -> i32 {
        let mut res = self.raw(data);
        let r = self.pad(data.len() as u32);
        if r < 0 {
            res = r;
        }
        res
    }

    /// `spa_pod_builder_push()`.
    fn push(&mut self, pod_size: u32, pod_type: u32, offset: u32) {
        self.frames.push(BuilderFrame {
            pod_size,
            pod_type,
            offset,
            flags: self.flags,
        });

        if pod_type == SPA_TYPE_ARRAY || pod_type == SPA_TYPE_CHOICE {
            self.flags = SPA_POD_BUILDER_FLAG_FIRST | SPA_POD_BUILDER_FLAG_BODY;
        }
    }

    /// `spa_pod_builder_pop()`: the finished pod's bytes, or `None` if it
    /// didn't fit.
    fn pop(&mut self) -> Option<std::ops::Range<usize>> {
        if self.flags & SPA_POD_BUILDER_FLAG_FIRST != 0 {
            let p = pod_header(0, SPA_TYPE_NONE);
            self.raw(&p);
        }
        let frame = self.frames.pop()?;
        // (spa_pod_builder_frame())
        let end = frame.offset as u64 + 8 + frame.pod_size as u64;
        let pod = if end <= self.size() as u64 {
            let o = frame.offset as usize;
            self.data[o..o + 8].copy_from_slice(&pod_header(frame.pod_size, frame.pod_type));
            Some(o..end as usize)
        } else {
            None
        };

        self.flags = frame.flags;
        self.pad(self.offset);
        pod
    }

    /// `spa_pod_builder_primitive()` for a pod given as its bytes.
    fn primitive(&mut self, p: &[u8]) -> i32 {
        let data = if self.flags == SPA_POD_BUILDER_FLAG_BODY {
            &p[8..]
        } else {
            self.flags &= !SPA_POD_BUILDER_FLAG_FIRST;
            p
        };
        let size = data.len() as u32;
        let mut res = self.raw(data);
        if self.flags != SPA_POD_BUILDER_FLAG_BODY {
            let r = self.pad(size);
            if r < 0 {
                res = r;
            }
        }
        res
    }

    /// `spa_pod_builder_id()`.
    fn id(&mut self, val: u32) -> i32 {
        let mut p = [0u8; 16]; // SPA_POD_INIT_Id(val)
        p[..8].copy_from_slice(&pod_header(4, SPA_TYPE_ID));
        p[8..12].copy_from_slice(&val.to_ne_bytes());
        self.primitive(&p[..12]) // (SPA_POD_SIZE(): the padding isn't part of the pod)
    }

    /// `spa_pod_builder_int()`.
    fn int(&mut self, val: i32) -> i32 {
        let mut p = [0u8; 16]; // SPA_POD_INIT_Int(val)
        p[..8].copy_from_slice(&pod_header(4, SPA_TYPE_INT));
        p[8..12].copy_from_slice(&val.to_ne_bytes());
        self.primitive(&p[..12]) // (SPA_POD_SIZE(): the padding isn't part of the pod)
    }

    /// `spa_pod_builder_array()`.
    fn array(&mut self, child_size: u32, child_type: u32, elems: &[u8]) -> i32 {
        let n_elems = elems.len() as u32 / child_size.max(1);
        let mut p = [0u8; 16];
        p[..8].copy_from_slice(&pod_header(8 + n_elems * child_size, SPA_TYPE_ARRAY));
        p[8..].copy_from_slice(&pod_header(child_size, child_type));
        let mut res = self.raw(&p);
        let r = self.raw_padded(elems);
        if r < 0 {
            res = r;
        }
        res
    }

    /// `spa_pod_builder_push_object()`.
    fn push_object(&mut self, type_: u32, id: u32) -> i32 {
        let mut p = [0u8; 16]; // SPA_POD_INIT_Object(sizeof(struct spa_pod_object_body), type, id)
        p[..8].copy_from_slice(&pod_header(8, SPA_TYPE_OBJECT));
        p[8..12].copy_from_slice(&type_.to_ne_bytes());
        p[12..].copy_from_slice(&id.to_ne_bytes());
        let offset = self.offset;
        let res = self.raw(&p);
        self.push(8, SPA_TYPE_OBJECT, offset);
        res
    }

    /// `spa_pod_builder_prop()`.
    fn prop(&mut self, key: u32, flags: u32) -> i32 {
        let mut p = [0u8; 8];
        p[..4].copy_from_slice(&key.to_ne_bytes());
        p[4..].copy_from_slice(&flags.to_ne_bytes());
        self.raw(&p)
    }

    /// The bytes of a pod `pop()` returned.
    pub(super) fn bytes(&self, range: std::ops::Range<usize>) -> &[u8] {
        &self.data[range]
    }
}

/// A `struct spa_pod` header.
fn pod_header(size: u32, type_: u32) -> [u8; 8] {
    let mut h = [0u8; 8];
    h[..4].copy_from_slice(&size.to_ne_bytes());
    h[4..].copy_from_slice(&type_.to_ne_bytes());
    h
}

/// Translation of `spa_format_audio_raw_build()`; returns the range of
/// the built pod in the builder's buffer, or `None` if it didn't fit.
pub(super) fn spa_format_audio_raw_build(
    builder: &mut SpaPodBuilder<'_>,
    id: u32,
    info: &SpaAudioInfoRaw,
) -> Option<std::ops::Range<usize>> {
    builder.push_object(SPA_TYPE_OBJECT_FORMAT, id);
    // spa_pod_builder_add(builder, key, SPA_POD_Id(..), ..., 0): a prop, then its value
    builder.prop(SPA_FORMAT_MEDIA_TYPE, 0);
    builder.id(SPA_MEDIA_TYPE_AUDIO);
    builder.prop(SPA_FORMAT_MEDIA_SUBTYPE, 0);
    builder.id(SPA_MEDIA_SUBTYPE_RAW);
    if info.format != SPA_AUDIO_FORMAT_UNKNOWN {
        builder.prop(SPA_FORMAT_AUDIO_FORMAT, 0);
        builder.id(info.format);
    }
    if info.rate != 0 {
        builder.prop(SPA_FORMAT_AUDIO_RATE, 0);
        builder.int(info.rate as i32);
    }
    if info.channels != 0 {
        builder.prop(SPA_FORMAT_AUDIO_CHANNELS, 0);
        builder.int(info.channels as i32);
        if info.flags & SPA_AUDIO_FLAG_UNPOSITIONED == 0 {
            builder.prop(SPA_FORMAT_AUDIO_POSITION, 0);
            let n = (info.channels as usize).min(SPA_AUDIO_MAX_CHANNELS);
            let elems: Vec<u8> = info.position[..n]
                .iter()
                .flat_map(|p| p.to_ne_bytes())
                .collect();
            builder.array(4, SPA_TYPE_ID, &elems);
        }
    }
    builder.pop()
}

// --- spa/pod/iter.h ---

/// A pod's bytes (header and body), as libpipewire passes it.
#[derive(Clone, Copy, Debug)]
pub(super) struct Pod<'a>(&'a [u8]);

impl<'a> Pod<'a> {
    /// The pod at `bytes`, if its header and body fit.
    #[cfg(test)]
    pub(super) fn new(bytes: &'a [u8]) -> Option<Pod<'a>> {
        let size = read_u32(bytes, 0)? as usize;
        bytes.get(..8 + size).map(Pod)
    }

    /// The pod at `p` (`const struct spa_pod *`).
    ///
    /// # Safety
    ///
    /// `p` must point to a valid pod whose body is `size` bytes long.
    pub(super) unsafe fn from_ptr(p: *const u8) -> Option<Pod<'a>> {
        if p.is_null() {
            return None;
        }
        // SAFETY: the header is readable (the caller's contract).
        let size = unsafe { p.cast::<u32>().read_unaligned() } as usize;
        // SAFETY: the header and body are readable (the caller's contract).
        Some(Pod(unsafe { std::slice::from_raw_parts(p, 8 + size) }))
    }

    /// `SPA_POD_BODY_SIZE()`.
    pub(super) fn body_size(&self) -> u32 {
        read_u32(self.0, 0).unwrap_or(0)
    }

    /// `SPA_POD_TYPE()`.
    pub(super) fn type_(&self) -> u32 {
        read_u32(self.0, 4).unwrap_or(0)
    }

    /// `SPA_POD_BODY()`.
    pub(super) fn body(&self) -> &'a [u8] {
        &self.0[8..]
    }
}

/// `spa_pod_is_object()`.
fn spa_pod_is_object(pod: Pod<'_>) -> bool {
    pod.type_() == SPA_TYPE_OBJECT && pod.body_size() >= 8
}

/// `spa_pod_is_choice()`.
fn spa_pod_is_choice(pod: Pod<'_>) -> bool {
    pod.type_() == SPA_TYPE_CHOICE && pod.body_size() >= 16
}

/// `spa_pod_is_id()`.
fn spa_pod_is_id(pod: Pod<'_>) -> bool {
    pod.type_() == SPA_TYPE_ID && pod.body_size() >= 4
}

/// `spa_pod_is_int()`.
fn spa_pod_is_int(pod: Pod<'_>) -> bool {
    pod.type_() == SPA_TYPE_INT && pod.body_size() >= 4
}

/// `spa_pod_is_array()`.
fn spa_pod_is_array(pod: Pod<'_>) -> bool {
    pod.type_() == SPA_TYPE_ARRAY && pod.body_size() >= 8
}

/// `spa_pod_get_int()`.
pub(super) fn spa_pod_get_int(pod: Pod<'_>) -> Result<i32, i32> {
    if !spa_pod_is_int(pod) {
        return Err(-EINVAL);
    }
    Ok(read_u32(pod.0, 8).unwrap_or(0) as i32)
}

/// A property inside an object (`struct spa_pod_prop`): its key and value.
#[derive(Clone, Copy, Debug)]
pub(super) struct PodProp<'a> {
    pub(super) key: u32,
    pub(super) value: Pod<'a>,
    /// Offset of the property from the object's start.
    offset: usize,
}

/// The property at `offset` in `obj`, if it's inside the object's body
/// (`spa_pod_prop_is_inside()`).
fn prop_at(obj: Pod<'_>, offset: usize) -> Option<PodProp<'_>> {
    let end = 8 + obj.body_size() as usize; // the body is [8, 8 + size)
    if offset + 16 > end || !offset.is_multiple_of(4) {
        return None;
    }
    let key = read_u32(obj.0, offset)?;
    let value_size = read_u32(obj.0, offset + 8)? as usize;
    if end - (offset + 16) < value_size {
        return None;
    }
    Some(PodProp {
        key,
        value: Pod(&obj.0[offset + 8..offset + 16 + value_size]),
        offset,
    })
}

/// `spa_pod_prop_next()`.
fn prop_next(prop: &PodProp<'_>) -> usize {
    prop.offset + round_up_8(16 + prop.value.body_size() as u64) as usize
}

/// `spa_pod_object_find_prop()`.
fn spa_pod_object_find_prop<'a>(
    pod: Pod<'a>,
    start: Option<&PodProp<'a>>,
    key: u32,
) -> Option<PodProp<'a>> {
    let first = 16; // spa_pod_prop_first(&pod->body)
    let start = start.map_or(first, prop_next);
    let mut res = start;
    while let Some(p) = prop_at(pod, res) {
        if p.key == key {
            return Some(p);
        }
        res = prop_next(&p);
    }
    let mut res = first;
    while res != start {
        let Some(p) = prop_at(pod, res) else { break };
        if p.key == key {
            return Some(p);
        }
        res = prop_next(&p);
    }
    None
}

/// `spa_pod_find_prop()`.
pub(super) fn spa_pod_find_prop<'a>(
    pod: Pod<'a>,
    start: Option<&PodProp<'a>>,
    key: u32,
) -> Option<PodProp<'a>> {
    if !spa_pod_is_object(pod) {
        return None;
    }
    spa_pod_object_find_prop(pod, start, key)
}

/// `SPA_POD_CHOICE_CHILD()`: the child pod header and the values after it.
fn choice_child(pod: Pod<'_>) -> Pod<'_> {
    // spa_pod_choice_body: type, flags, then the child pod; its values
    // fill the rest of the choice body.
    Pod(&pod.0[16..])
}

/// `SPA_POD_CHOICE_N_VALUES()`.
fn choice_n_values(pod: Pod<'_>) -> u32 {
    let child_size = read_u32(pod.0, 16).unwrap_or(0);
    // (0 when the child size is 0)
    pod.body_size()
        .saturating_sub(16)
        .checked_div(child_size)
        .unwrap_or(0)
}

/// `spa_pod_get_values()`: the values pod, their count and the choice type.
/// For a choice, the returned pod's "body" is all the values.
pub(super) fn spa_pod_get_values(pod: Pod<'_>) -> (Pod<'_>, u32, u32) {
    if pod.type_() == SPA_TYPE_CHOICE && pod.0.len() >= 24 {
        let mut n_vals = choice_n_values(pod);
        let choice = read_u32(pod.0, 8).unwrap_or(0); // SPA_POD_CHOICE_TYPE()
        if choice == SPA_CHOICE_NONE {
            n_vals = n_vals.min(1);
        }
        (choice_child(pod), n_vals, choice)
    } else {
        (pod, 1, SPA_CHOICE_NONE)
    }
}

// --- spa/pod/parser.h, spa/param/audio/raw-utils.h ---

/// `spa_pod_parser_can_collect()` for the types SDL collects.
fn spa_pod_parser_can_collect(pod: Option<Pod<'_>>, type_: u8) -> bool {
    let Some(mut pod) = pod else {
        return false;
    };

    if pod.type_() == SPA_TYPE_CHOICE {
        if !spa_pod_is_choice(pod) {
            return false;
        }
        if read_u32(pod.0, 8) != Some(SPA_CHOICE_NONE) {
            return false;
        }
        pod = choice_child(pod);
    }

    match type_ {
        b'P' => true,
        b'I' => spa_pod_is_id(pod),
        b'i' => spa_pod_is_int(pod),
        _ => false,
    }
}

/// `spa_pod_copy_array()`.
fn spa_pod_copy_array(pod: Pod<'_>, type_: u32, values: &mut [u32]) -> u32 {
    // (spa_pod_get_array())
    if !spa_pod_is_array(pod) || values.is_empty() {
        return 0;
    }
    let child_size = read_u32(pod.0, 8).unwrap_or(0); // SPA_POD_ARRAY_VALUE_SIZE()
    let child_type = read_u32(pod.0, 12).unwrap_or(0); // SPA_POD_ARRAY_VALUE_TYPE()
    if child_type != type_ {
        return 0;
    }
    // SPA_POD_ARRAY_N_VALUES() (0 when the child size is 0)
    let n_values = (pod.body_size() - 8).checked_div(child_size).unwrap_or(0);
    let n_values = (n_values as usize).min(values.len());
    // memcpy(values, v, SPA_POD_ARRAY_VALUE_SIZE(pod) * n_values), bounded
    // by the destination (only 4-byte ids are ever copied here).
    let src = &pod.0[16..];
    let nbytes = (child_size as usize * n_values)
        .min(values.len() * 4)
        .min(src.len());
    for (i, part) in src[..nbytes].chunks(4).enumerate() {
        let mut b = values[i].to_ne_bytes();
        b[..part.len()].copy_from_slice(part);
        values[i] = u32::from_ne_bytes(b);
    }
    n_values as u32
}

/// Translation of `spa_format_audio_raw_parse()`, i.e.
/// `spa_pod_parse_object(format, SPA_TYPE_OBJECT_Format, NULL,
/// SPA_FORMAT_AUDIO_format, SPA_POD_OPT_Id(..), SPA_FORMAT_AUDIO_rate,
/// SPA_POD_OPT_Int(..), SPA_FORMAT_AUDIO_channels, SPA_POD_OPT_Int(..),
/// SPA_FORMAT_AUDIO_position, SPA_POD_OPT_Pod(..))`. Returns the number of
/// fields collected, or a negative errno.
pub(super) fn spa_format_audio_raw_parse(format: Pod<'_>, info: &mut SpaAudioInfoRaw) -> i32 {
    let mut position = None;

    info.flags = 0;
    let res = (|| {
        // spa_pod_parser_push_object(): spa_pod_parser_current() wants the
        // body, rounded up to 8 bytes, inside the pod.
        if round_up_8(format.body_size() as u64) > format.body_size() as u64 {
            return -EPIPE;
        }
        if !spa_pod_is_object(format) {
            return -EINVAL;
        }
        if read_u32(format.0, 8) != Some(SPA_TYPE_OBJECT_FORMAT) {
            return -EPROTO;
        }
        // spa_pod_parser_getv()
        let mut count = 0;
        let mut prop: Option<PodProp<'_>> = None;
        for (key, type_) in [
            (SPA_FORMAT_AUDIO_FORMAT, b'I'),
            (SPA_FORMAT_AUDIO_RATE, b'i'),
            (SPA_FORMAT_AUDIO_CHANNELS, b'i'),
            (SPA_FORMAT_AUDIO_POSITION, b'P'),
        ] {
            prop = spa_pod_object_find_prop(format, prop.as_ref(), key);
            let pod = prop.map(|p| p.value);
            // (every field is optional: "?" + type)
            if spa_pod_parser_can_collect(pod, type_) {
                let mut pod = pod.expect("collectable pods exist");
                if pod.type_() == SPA_TYPE_CHOICE {
                    pod = choice_child(pod);
                }
                let value = read_u32(pod.0, 8).unwrap_or(0);
                match key {
                    SPA_FORMAT_AUDIO_FORMAT => info.format = value,
                    SPA_FORMAT_AUDIO_RATE => info.rate = value,
                    SPA_FORMAT_AUDIO_CHANNELS => info.channels = value,
                    _ => position = (pod.type_() != SPA_TYPE_NONE).then_some(pod),
                }
                count += 1;
            }
        }
        let _ = ESRCH; // (only for missing, non-optional fields)
        count
    })();
    if position.is_none_or(|p| spa_pod_copy_array(p, SPA_TYPE_ID, &mut info.position) == 0) {
        info.flags |= SPA_AUDIO_FLAG_UNPOSITIONED;
    }
    res
}

// --- spa/utils/dict.h ---

/// `struct spa_dict_item`.
#[repr(C)]
pub(super) struct SpaDictItem {
    pub(super) key: *const c_char,
    pub(super) value: *const c_char,
}

/// `struct spa_dict`.
#[repr(C)]
pub(super) struct SpaDict {
    pub(super) flags: u32,
    pub(super) n_items: u32,
    pub(super) items: *const SpaDictItem,
}

const SPA_DICT_FLAG_SORTED: u32 = 1 << 0;

/// Translation of `spa_dict_lookup()` (and `spa_dict_lookup_item()`).
///
/// # Safety
///
/// `dict` must be NULL or a valid dictionary whose keys and values are C strings.
pub(super) unsafe fn spa_dict_lookup<'a>(dict: *const SpaDict, key: &str) -> Option<&'a CStr> {
    if dict.is_null() {
        // FIXME (upstream): SDL passes the props of an info struct without
        // checking them for NULL; a NULL dict has nothing in it here.
        return None;
    }
    // SAFETY: a valid dictionary (the caller's contract).
    let dict = unsafe { &*dict };
    if dict.items.is_null() || dict.n_items == 0 {
        return None;
    }
    // SAFETY: `items` holds `n_items` items.
    let items = unsafe { std::slice::from_raw_parts(dict.items, dict.n_items as usize) };
    // SAFETY: the keys are C strings.
    let key_of = |item: &SpaDictItem| unsafe { CStr::from_ptr(item.key) }.to_bytes();
    let found = if dict.flags & SPA_DICT_FLAG_SORTED != 0 {
        // (bsearch with strcmp)
        items
            .binary_search_by(|item| key_of(item).cmp(key.as_bytes()))
            .ok()
            .map(|i| &items[i])
    } else {
        items.iter().find(|item| key_of(item) == key.as_bytes())
    };
    let item = found?;
    if item.value.is_null() {
        return None;
    }
    // SAFETY: the values are C strings that live as long as the dictionary.
    Some(unsafe { CStr::from_ptr(item.value) })
}

// --- spa/utils/json.h ---

/// `struct spa_json`: a relaxed JSON tokenizer. Upstream's iterator keeps
/// a pointer to its parent to hand back the position after a container;
/// SDL never continues with the parent, so only whether there is one is
/// kept.
#[derive(Clone, Debug)]
pub(super) struct SpaJson<'a> {
    data: &'a [u8],
    cur: usize,
    end: usize,
    has_parent: bool,
    state: u32,
    depth: u32,
}

const SPA_JSON_ERROR_FLAG: u32 = 0x100;

// the tokenizer's states and flags (an anonymous enum in spa_json_next())
const NONE: u32 = 0;
const STRUCT: u32 = 1;
const BARE: u32 = 2;
const STRING: u32 = 3;
const UTF8: u32 = 4;
const ESC: u32 = 5;
const COMMENT: u32 = 6;
const ARRAY_FLAG: u32 = 0x10; // in array context
const PREV_ARRAY_FLAG: u32 = 0x20; // depth=0 array context flag
const KEY_FLAG: u32 = 0x40; // inside object key
const SUB_FLAG: u32 = 0x80; // not at top-level
const FLAGS: u32 = 0xff0;
const ERROR_INVALID_ARRAY_SEPARATOR: u32 = SPA_JSON_ERROR_FLAG + 1;
const ERROR_EXPECTED_OBJECT_KEY: u32 = SPA_JSON_ERROR_FLAG + 2;
const ERROR_EXPECTED_OBJECT_VALUE: u32 = SPA_JSON_ERROR_FLAG + 3;
const ERROR_TOO_DEEP_NESTING: u32 = SPA_JSON_ERROR_FLAG + 4;
const ERROR_EXPECTED_ARRAY_CLOSE: u32 = SPA_JSON_ERROR_FLAG + 5;
const ERROR_EXPECTED_OBJECT_CLOSE: u32 = SPA_JSON_ERROR_FLAG + 6;
const ERROR_MISMATCHED_BRACKET: u32 = SPA_JSON_ERROR_FLAG + 7;
const ERROR_ESCAPE_NOT_ALLOWED: u32 = SPA_JSON_ERROR_FLAG + 8;
const ERROR_CHARACTERS_NOT_ALLOWED: u32 = SPA_JSON_ERROR_FLAG + 9;
const ERROR_INVALID_ESCAPE: u32 = SPA_JSON_ERROR_FLAG + 10;
const ERROR_INVALID_STATE: u32 = SPA_JSON_ERROR_FLAG + 11;
const ERROR_UNFINISHED_STRING: u32 = SPA_JSON_ERROR_FLAG + 12;

/// `SPA_FLAG_UPDATE(field, flag, val)`.
fn flag_update(field: &mut u32, flag: u32, val: bool) {
    if val {
        *field |= flag;
    } else {
        *field &= !flag;
    }
}

impl<'a> SpaJson<'a> {
    /// `spa_json_init()`.
    pub(super) fn new(data: &'a [u8]) -> SpaJson<'a> {
        SpaJson {
            data,
            cur: 0,
            end: data.len(),
            has_parent: false,
            state: 0,
            depth: 0,
        }
    }

    /// `spa_json_enter()`.
    fn enter(&self) -> SpaJson<'a> {
        SpaJson {
            data: self.data,
            cur: self.cur,
            end: self.end,
            has_parent: true,
            state: self.state & 0xff0,
            depth: 0,
        }
    }

    /// `spa_json_next()`: the next token's start and length; the length is
    /// -1 on parse error, 0 on end of input.
    pub(super) fn next(&mut self) -> (usize, i32) {
        let mut utf8_remain = 0;
        let mut array_stack = [0u64; 8]; // array context flags of depths 1...512

        let mut value = self.cur;

        if self.state & SPA_JSON_ERROR_FLAG != 0 {
            return (value, -1);
        }

        macro_rules! error {
            ($reason:expr) => {{
                self.state = $reason;
                return (value, -1);
            }};
        }

        while self.cur < self.end {
            let cur = self.data[self.cur];
            let mut again = true;
            while again {
                again = false;
                let mut flag = self.state & FLAGS;
                match self.state & !FLAGS {
                    NONE => {
                        flag &= !(KEY_FLAG | PREV_ARRAY_FLAG);
                        self.state = STRUCT | flag;
                        self.depth = 0;
                        again = true;
                    }
                    STRUCT => match cur {
                        b'\0' | b'\t' | b' ' | b'\r' | b'\n' | b',' => {}
                        b':' | b'=' => {
                            if flag & ARRAY_FLAG != 0 {
                                error!(ERROR_INVALID_ARRAY_SEPARATOR);
                            }
                            if flag & KEY_FLAG == 0 {
                                error!(ERROR_EXPECTED_OBJECT_KEY);
                            }
                            self.state |= SUB_FLAG;
                        }
                        b'#' => self.state = COMMENT | flag,
                        b'"' => {
                            if flag & KEY_FLAG != 0 {
                                flag |= SUB_FLAG;
                            }
                            if flag & ARRAY_FLAG == 0 {
                                let k = flag & KEY_FLAG == 0;
                                flag_update(&mut flag, KEY_FLAG, k);
                            }
                            value = self.cur;
                            self.state = STRING | flag;
                        }
                        b'[' | b'{' => {
                            if flag & ARRAY_FLAG == 0 {
                                // At top-level we may be either in object context
                                // or in single-item context, and then we need to
                                // accept array/object here.
                                if (self.state & SUB_FLAG) != 0 && (flag & KEY_FLAG) == 0 {
                                    error!(ERROR_EXPECTED_OBJECT_KEY);
                                }
                                flag &= !KEY_FLAG;
                            }
                            self.state = STRUCT | SUB_FLAG | flag;
                            flag_update(&mut self.state, ARRAY_FLAG, cur == b'[');

                            // We need to remember previous array state across calls
                            // for depth=0, so store that in state. Others bits go to
                            // temporary stack.
                            if self.depth == 0 {
                                flag_update(
                                    &mut self.state,
                                    PREV_ARRAY_FLAG,
                                    flag & ARRAY_FLAG != 0,
                                );
                            } else if (((self.depth - 1) >> 6) as usize) < array_stack.len() {
                                let mask = 1u64 << ((self.depth - 1) & 0x3f);
                                let slot = &mut array_stack[((self.depth - 1) >> 6) as usize];
                                if flag & ARRAY_FLAG != 0 {
                                    *slot |= mask;
                                } else {
                                    *slot &= !mask;
                                }
                            } else {
                                // too deep
                                error!(ERROR_TOO_DEEP_NESTING);
                            }

                            value = self.cur;
                            self.depth += 1;
                            if self.depth <= 1 {
                                self.cur += 1;
                                return (value, 1);
                            }
                        }
                        b'}' | b']' => {
                            if (flag & ARRAY_FLAG) != 0 && cur != b']' {
                                error!(ERROR_EXPECTED_ARRAY_CLOSE);
                            }
                            if (flag & ARRAY_FLAG) == 0 && cur != b'}' {
                                error!(ERROR_EXPECTED_OBJECT_CLOSE);
                            }
                            if flag & KEY_FLAG != 0 {
                                // incomplete key-value pair
                                error!(ERROR_EXPECTED_OBJECT_VALUE);
                            }
                            self.state = STRUCT | SUB_FLAG | flag;
                            if self.depth == 0 {
                                if !self.has_parent {
                                    error!(ERROR_MISMATCHED_BRACKET);
                                }
                                // (iter->parent->cur = iter->cur)
                                return (value, 0);
                            }
                            self.depth -= 1;
                            if self.depth == 0 {
                                let prev = flag & PREV_ARRAY_FLAG != 0;
                                flag_update(&mut self.state, ARRAY_FLAG, prev);
                            } else if (((self.depth - 1) >> 6) as usize) < array_stack.len() {
                                let mask = 1u64 << ((self.depth - 1) & 0x3f);
                                let set = array_stack[((self.depth - 1) >> 6) as usize] & mask != 0;
                                flag_update(&mut self.state, ARRAY_FLAG, set);
                            } else {
                                // too deep
                                error!(ERROR_TOO_DEEP_NESTING);
                            }
                        }
                        b'\\' => {
                            // disallow bare escape
                            error!(ERROR_ESCAPE_NOT_ALLOWED);
                        }
                        _ => {
                            // allow bare ascii
                            if !(32..=126).contains(&cur) {
                                error!(ERROR_CHARACTERS_NOT_ALLOWED);
                            }
                            if flag & KEY_FLAG != 0 {
                                flag |= SUB_FLAG;
                            }
                            if flag & ARRAY_FLAG == 0 {
                                let k = flag & KEY_FLAG == 0;
                                flag_update(&mut flag, KEY_FLAG, k);
                            }
                            value = self.cur;
                            self.state = BARE | flag;
                        }
                    },
                    BARE => match cur {
                        b'\0' | b'\t' | b' ' | b'\r' | b'\n' | b'"' | b'#' | b':' | b',' | b'='
                        | b']' | b'}' => {
                            self.state = STRUCT | flag;
                            if self.depth > 0 {
                                again = true;
                            } else {
                                return (value, (self.cur - value) as i32);
                            }
                        }
                        b'\\' => {
                            // disallow bare escape
                            error!(ERROR_ESCAPE_NOT_ALLOWED);
                        }
                        _ => {
                            // allow bare ascii
                            if !(32..=126).contains(&cur) {
                                error!(ERROR_CHARACTERS_NOT_ALLOWED);
                            }
                        }
                    },
                    STRING => match cur {
                        b'\\' => self.state = ESC | flag,
                        b'"' => {
                            self.state = STRUCT | flag;
                            if self.depth == 0 {
                                self.cur += 1;
                                return (value, (self.cur - value) as i32);
                            }
                        }
                        240..=247 => {
                            utf8_remain += 3;
                            self.state = UTF8 | flag;
                        }
                        224..=239 => {
                            utf8_remain += 2;
                            self.state = UTF8 | flag;
                        }
                        192..=223 => {
                            utf8_remain += 1;
                            self.state = UTF8 | flag;
                        }
                        _ => {
                            if !(32..=127).contains(&cur) {
                                error!(ERROR_CHARACTERS_NOT_ALLOWED);
                            }
                        }
                    },
                    UTF8 => match cur {
                        128..=191 => {
                            utf8_remain -= 1;
                            if utf8_remain == 0 {
                                self.state = STRING | flag;
                            }
                        }
                        _ => error!(ERROR_CHARACTERS_NOT_ALLOWED),
                    },
                    ESC => match cur {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' | b'u' => {
                            self.state = STRING | flag;
                        }
                        _ => error!(ERROR_INVALID_ESCAPE),
                    },
                    COMMENT => {
                        if cur == b'\n' || cur == b'\r' {
                            self.state = STRUCT | flag;
                        }
                    }
                    _ => error!(ERROR_INVALID_STATE),
                }
            }
            self.cur += 1;
        }
        if self.depth != 0 || self.has_parent {
            error!(ERROR_MISMATCHED_BRACKET);
        }

        match self.state & !FLAGS {
            STRING | UTF8 | ESC => {
                // string/escape not closed
                error!(ERROR_UNFINISHED_STRING);
            }
            COMMENT => {
                // trailing comment
                return (value, 0);
            }
            _ => {}
        }

        if (self.state & SUB_FLAG) != 0 && (self.state & KEY_FLAG) != 0 {
            // incomplete key-value pair
            error!(ERROR_EXPECTED_OBJECT_VALUE);
        }

        if (self.state & !FLAGS) != STRUCT {
            self.state = STRUCT | (self.state & FLAGS);
            return (value, (self.cur - value) as i32);
        }
        (value, 0)
    }

    /// `spa_json_enter_container()`.
    fn enter_container(&mut self, type_: u8) -> Result<SpaJson<'a>, i32> {
        let (value, len) = self.next();
        if len <= 0 {
            return Err(len);
        }
        if self.data[value] != type_ {
            return Err(-1);
        }
        Ok(self.enter())
    }

    /// `spa_json_enter_object()`: the sub-iterator, or the (<= 0) result.
    pub(super) fn enter_object(&mut self) -> Result<SpaJson<'a>, i32> {
        self.enter_container(b'{')
    }

    /// `spa_json_enter_array()`: the sub-iterator, or the (<= 0) result.
    pub(super) fn enter_array(&mut self) -> Result<SpaJson<'a>, i32> {
        self.enter_container(b'[')
    }

    /// `spa_json_get_string()` into a buffer of `maxlen` bytes: the string
    /// (without the terminating NUL), or the (<= 0) result.
    pub(super) fn get_string(&mut self, maxlen: usize) -> Result<Vec<u8>, i32> {
        let (value, len) = self.next();
        if len <= 0 {
            return Err(len);
        }
        let val = &self.data[value..value + len as usize];
        spa_json_parse_stringn(val, maxlen).ok_or(-1)
    }
}

/// `spa_json_is_string()`.
fn spa_json_is_string(val: &[u8]) -> bool {
    val.len() > 1 && val[0] == b'"'
}

/// `spa_json_parse_hex()`.
fn spa_json_parse_hex(p: &[u8], num: usize) -> Option<u32> {
    let mut res = 0u32;
    for i in 0..num {
        let v = *p.get(i)?;
        let v = match v {
            b'0'..=b'9' => v - b'0',
            b'a'..=b'f' => v - b'a' + 10,
            b'A'..=b'F' => v - b'A' + 10,
            _ => return None,
        };
        res = (res << 4) | v as u32;
    }
    Some(res)
}

/// `spa_json_parse_stringn()`: the unescaped string (or `None` for -1,
/// when it doesn't fit in `maxlen` bytes with its NUL).
fn spa_json_parse_stringn(val: &[u8], maxlen: usize) -> Option<Vec<u8>> {
    let len = val.len();
    if maxlen <= len {
        return None;
    }
    let mut result = Vec::with_capacity(len);
    if !spa_json_is_string(val) {
        result.extend_from_slice(val);
    } else {
        let mut p = 1;
        while p < len {
            if val[p] == b'\\' {
                p += 1;
                let c = val.get(p).copied().unwrap_or(0);
                if c == b'n' {
                    result.push(b'\n');
                } else if c == b'r' {
                    result.push(b'\r');
                } else if c == b'b' {
                    result.push(0x08);
                } else if c == b't' {
                    result.push(b'\t');
                } else if c == b'f' {
                    result.push(0x0c);
                } else if c == b'u' {
                    let prefix = [0u8, 0xc0, 0xe0, 0xf0];
                    let enc = [0x80u32, 0x800, 0x10000];
                    let Some(mut cp) = (len - p >= 5)
                        .then(|| spa_json_parse_hex(&val[p + 1..], 4))
                        .flatten()
                    else {
                        result.push(c);
                        p += 1;
                        continue;
                    };
                    p += 4;

                    if (0xd800..=0xdbff).contains(&cp) {
                        let v = (len - p >= 7 && val[p + 1] == b'\\' && val[p + 2] == b'u')
                            .then(|| spa_json_parse_hex(&val[p + 3..], 4))
                            .flatten()
                            .filter(|v| (0xdc00..=0xdfff).contains(v));
                        let Some(v) = v else {
                            p += 1;
                            continue;
                        };
                        p += 6;
                        cp = 0x010000 + (((cp & 0x3ff) << 10) | (v & 0x3ff));
                    } else if (0xdc00..=0xdfff).contains(&cp) {
                        p += 1;
                        continue;
                    }

                    let mut idx = 0;
                    while idx < 3 {
                        if cp < enc[idx] {
                            break;
                        }
                        idx += 1;
                    }
                    let mut tail = [0u8; 3];
                    for n in (1..=idx).rev() {
                        tail[n - 1] = ((cp | 0x80) & 0xbf) as u8;
                        cp >>= 6;
                    }
                    result.push(((cp | prefix[idx] as u32) & 0xff) as u8);
                    result.extend_from_slice(&tail[..idx]);
                } else {
                    result.push(c);
                }
            } else if val[p] == b'"' {
                break;
            } else {
                result.push(val[p]);
            }
            p += 1;
        }
    }
    Some(result)
}
