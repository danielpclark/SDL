// Rust translation of the report handling of src/hidapi/ios/hid.m from
// Simple DirectMedia Layer.
// Copyright (C) 2021 Valve Corporation
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What the iOS and tvOS Bluetooth LE backend ([`super::ios`]) does with
//! the bytes of a Steam Controller, apart from CoreBluetooth: the input
//! reports' ring buffer, the controllers' GATT characteristics, and how
//! feature and input reports go in and out of a characteristic's value.
//! (Pure code, so the tests run it everywhere.)

/// `VALVE_USB_VID`
pub(crate) const VALVE_USB_VID: u16 = 0x28DE;
/// `D0G_BLE2_PID`: the Steam Controller.
pub(crate) const D0G_BLE2_PID: u16 = 0x1106;
/// `TRITON_BLE_PID`: the Steam Controller (2026).
pub(crate) const TRITON_BLE_PID: u16 = 0x1303;

/// `VALVE_SERVICE`: the controllers' GATT service.
pub(crate) const VALVE_SERVICE: &str = "100F6C32-1735-4313-B402-38567131E5F3";

/// `VALVE_INPUT_CHAR_0x1106` (read/notifications)
pub(crate) const VALVE_INPUT_CHAR_0X1106: &str = "100F6C33-1735-4313-B402-38567131E5F3";
/// `VALVE_INPUT_CHAR_0x1303_0x45` (read/notifications)
pub(crate) const VALVE_INPUT_CHAR_0X1303_0X45: &str = "100F6C7A-1735-4313-B402-38567131E5F3";
/// `VALVE_INPUT_CHAR_0x1303_0x47` (read/notifications)
pub(crate) const VALVE_INPUT_CHAR_0X1303_0X47: &str = "100F6C7C-1735-4313-B402-38567131E5F3";

/// `VALVE_REPORT_CHAR` (read/write): the feature reports'.
pub(crate) const VALVE_REPORT_CHAR: &str = "100F6C34-1735-4313-B402-38567131E5F3";

// (`REPORT_SEGMENT_DATA_FLAG`, `REPORT_SEGMENT_LAST_FLAG`, the
// `bluetoothSegment` and `hidFeatureReport` structures and
// `GetBluetoothSegmentSize()` are only declared upstream: the Steam driver
// frames the segments, and the backend sends the bytes after a feature
// report's ID, `&report->segment`, as `feature_report_segment` does.)

/// Whether two UUID strings name the same UUID (`-[CBUUID isEqual:]`, for
/// the 128-bit UUIDs here: `UUIDString` is upper case, the constants too,
/// but a UUID made from a lower case string is the same one).
pub(crate) fn uuid_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// What an input characteristic says about its controller: its product
/// ID and the ID and length of its input reports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct InputCharacteristic {
    pub(crate) pid: u16,
    pub(crate) report_id: u8,
    pub(crate) report_length: i32,
}

/// The input characteristic a UUID is, if it is one (the first branches of
/// `-peripheral:didDiscoverCharacteristicsForService:error:`).
pub(crate) fn input_characteristic(uuid: &str) -> Option<InputCharacteristic> {
    let (pid, report_id, report_length) = if uuid_eq(uuid, VALVE_INPUT_CHAR_0X1106) {
        (D0G_BLE2_PID, 0x03, 19)
    } else if uuid_eq(uuid, VALVE_INPUT_CHAR_0X1303_0X45) {
        (TRITON_BLE_PID, 0x45, 45)
    } else if uuid_eq(uuid, VALVE_INPUT_CHAR_0X1303_0X47) {
        (TRITON_BLE_PID, 0x47, 45)
    } else {
        return None;
    };
    Some(InputCharacteristic {
        pid,
        report_id,
        report_length,
    })
}

/// The output report a characteristic of the service carries, by its
/// `UUIDString`: `sscanf(UUIDString, "100F6C%x", &report_id) == 1 &&
/// report_id > 0x35`, then less 0x35, and an output report from 0x80.
pub(crate) fn output_report_id(uuid_string: &str) -> Option<i32> {
    let report_id = scan_hex(uuid_string.strip_prefix("100F6C")?)?;
    if report_id > 0x35 {
        let report_id = report_id - 0x35;
        //NSLog( @"Found characteristic for output report 0x%.2x", report_id );

        if report_id >= 0x80 {
            // An output report
            return Some(report_id);
        }
    }
    None
}

/// `sscanf`'s `%x` into an `int`: spaces, a sign, `0x`, then hex digits
/// (at least one), wrapping as the conversion to `unsigned` does.
fn scan_hex(s: &str) -> Option<i32> {
    let s = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (negative, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let digits = |s: &str| s.bytes().take_while(u8::is_ascii_hexdigit).count();
    let s = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(rest) if digits(rest) > 0 => rest,
        _ => s,
    };
    let n = digits(s);
    if n == 0 {
        return None;
    }
    let mut value: u32 = 0;
    for b in s[..n].bytes() {
        value = value
            .wrapping_mul(16)
            .wrapping_add((b as char).to_digit(16).unwrap_or(0));
    }
    let value = value as i32;
    Some(if negative {
        value.wrapping_neg()
    } else {
        value
    })
}

/// `RingBuffer_nElem`
const RING_BUFFER_N_ELEM: usize = 256;

/// The input reports received and not read yet, the oldest dropped when
/// it is full. Translation of `RingBuffer` (its `accessLock` is the mutex
/// it is kept in).
#[derive(Debug)]
pub(crate) struct RingBuffer {
    /// `_first`: the oldest report's offset, `None` (-1) when empty.
    first: Option<usize>,
    /// `_last`: where the next report goes.
    last: usize,
    /// `_cbElem`: the size of a report.
    cb_elem: usize,
    /// `_data`
    data: Vec<u8>,
}

impl RingBuffer {
    /// Translation of `RingBuffer_alloc()`.
    pub(crate) fn new(cb_elem: usize) -> RingBuffer {
        RingBuffer {
            first: None,
            last: 0,
            cb_elem,
            data: vec![0; RING_BUFFER_N_ELEM * cb_elem],
        }
    }

    /// The size of a report (`_cbElem`).
    pub(crate) fn elem_size(&self) -> usize {
        self.cb_elem
    }

    /// `offset % (RingBuffer_nElem * _cbElem)`. Note (upstream): with no
    /// report length (a controller whose input characteristic wasn't
    /// found yet) that divides by zero; this wraps to 0, as AArch64's
    /// division does.
    fn wrap(&self, offset: usize) -> usize {
        offset.checked_rem(self.data.len()).unwrap_or(0)
    }

    /// Add a report (`_cbElem` bytes of `src`); false when the buffer was
    /// full and lost its oldest report. Translation of `RingBuffer_write()`.
    pub(crate) fn write(&mut self, src: &[u8]) -> bool {
        // FIXME (upstream): a report shorter than `_cbElem` (which is only
        // logged) is read past its end; here the rest is zeros.
        let n = src.len().min(self.cb_elem);
        let last = self.last;
        self.data[last..last + n].copy_from_slice(&src[..n]);
        self.data[last + n..last + self.cb_elem].fill(0);
        if self.first.is_none() {
            self.first = Some(last);
        }
        self.last = self.wrap(last + self.cb_elem);
        if Some(self.last) == self.first {
            self.first = Some(self.wrap(self.last + self.cb_elem));
            return false;
        }
        true
    }

    /// Take the oldest report into `dst` (`_cbElem` bytes, as many as fit
    /// here); false when there is none. Translation of `RingBuffer_read()`.
    pub(crate) fn read(&mut self, dst: &mut [u8]) -> bool {
        let Some(first) = self.first else {
            return false;
        };
        let n = dst.len().min(self.cb_elem);
        dst[..n].copy_from_slice(&self.data[first..first + n]);
        let first = self.wrap(first + self.cb_elem);
        self.first = (first != self.last).then_some(first);
        true
    }
}

/// An input report into `dst`: the report ID, then the oldest report of
/// the buffer; its length (`_cbElem + 1`), 0 without one. Translation of
/// `-[HIDBLEDevice read_input_report:]`.
pub(crate) fn read_input_report(
    input_reports: Option<&mut RingBuffer>,
    report_id: u8,
    dst: &mut [u8],
) -> usize {
    let Some(input_reports) = input_reports else {
        return 0;
    };
    // FIXME (upstream): the report is copied whatever the size of the
    // caller's buffer; here only what fits is (with the full length
    // returned, as upstream's).
    let mut report = vec![0; input_reports.elem_size()];
    if input_reports.read(&mut report) {
        if let Some((id, rest)) = dst.split_first_mut() {
            *id = report_id;
            let n = rest.len().min(report.len());
            rest[..n].copy_from_slice(&report[..n]);
        }
        return input_reports.elem_size() + 1;
    }
    0
}

/// The bytes `-[HIDBLEDevice send_feature_report:length:]` writes to the
/// report characteristic: the report after its ID (`&report->segment`),
/// at most 64 bytes of it.
pub(crate) fn feature_report_segment(data: &[u8]) -> Vec<u8> {
    let length = data.len().min(64);
    // FIXME (upstream): `length` bytes are written from the byte after the
    // report ID, so (for a report of up to 64 bytes) the last one is past
    // the caller's report; here it is 0.
    let mut segment = vec![0; length];
    let available = data.len().saturating_sub(1).min(length);
    if available > 0 {
        segment[..available].copy_from_slice(&data[1..1 + available]);
    }
    segment
}

/// Copy a feature report (the report characteristic's value) into
/// `buffer`, which holds the report ID asked for: the whole value when it
/// starts with that ID, else after the ID ("leave the report in the
/// buffer"). The number of bytes the buffer then holds (`amount`, 0 for
/// an empty value). From `-[HIDBLEDevice get_feature_report:into:length:]`.
pub(crate) fn copy_feature_report(report: &[u8], buffer: &mut [u8]) -> usize {
    let length = buffer.len();
    let mut amount = 0;
    if !report.is_empty() && length > 0 {
        if report[0] == buffer[0] {
            amount = length.min(report.len());
            buffer[..amount].copy_from_slice(&report[..amount]);
        } else {
            // Leave the report in the buffer
            amount = (length - 1).min(report.len());
            buffer[1..1 + amount].copy_from_slice(&report[..amount]);
            amount += 1;
        }
    }
    amount
}

/// What `hid_get_feature_report()` returns for what the device copied
/// (`written`, -1 for an error) into a buffer of `length` bytes: the whole
/// length when one byte less was (`written == length-1 ? length :
/// written`, in `size_t`, so -1 for an error, and a 0-byte buffer's 0).
pub(crate) fn feature_report_result(written: isize, length: usize) -> isize {
    if written as usize == length.wrapping_sub(1) {
        length as isize
    } else {
        written
    }
}
