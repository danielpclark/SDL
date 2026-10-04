// Rust translation of src/joystick/hidapi/SDL_report_descriptor.c and
// SDL_report_descriptor.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This is a very simple (and non-compliant!) report descriptor parser
//! used to quickly parse Xbox Bluetooth reports: the input fields of a
//! descriptor, with their usages and bit positions.
//!
//! (Upstream's `DEBUG_DESCRIPTOR` trace is left out.)

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use crate::error::{Error, Result};

/// The item types (`ItemType`).
const DESCRIPTOR_ITEM_TYPE_MAIN: u8 = 0;
const DESCRIPTOR_ITEM_TYPE_GLOBAL: u8 = 1;
const DESCRIPTOR_ITEM_TYPE_LOCAL: u8 = 2;

/// The main item tags (`MainTag`).
const MAIN_TAG_INPUT: u8 = 0x8;
const MAIN_TAG_COLLECTION: u8 = 0xa;
const MAIN_TAG_END_COLLECTION: u8 = 0xc;

/// The global item tags (`GlobalTag`) that are used.
const GLOBAL_TAG_USAGE_PAGE: u8 = 0x0;
const GLOBAL_TAG_REPORT_SIZE: u8 = 0x7;
const GLOBAL_TAG_REPORT_ID: u8 = 0x8;
const GLOBAL_TAG_REPORT_COUNT: u8 = 0x9;

/// The local item tags (`LocalTag`) that are used.
const LOCAL_TAG_USAGE: u8 = 0x0;
const LOCAL_TAG_USAGE_MINIMUM: u8 = 0x1;
const LOCAL_TAG_USAGE_MAXIMUM: u8 = 0x2;

/// `MAKE_USAGE(page, usage)`: a full usage, the page in the high 16 bits.
pub(crate) const fn make_usage(usage_page: u16, usage: u16) -> u32 {
    ((usage_page as u32) << 16) | usage as u32
}

/// An input field of a report. Translation of `DescriptorInputField`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct DescriptorInputField {
    pub(crate) report_id: u8,
    pub(crate) usage: u32,
    pub(crate) bit_offset: i32,
    pub(crate) bit_size: i32,
}

/// The input fields of a descriptor. Translation of `SDL_ReportDescriptor`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub(crate) struct ReportDescriptor {
    pub(crate) fields: Vec<DescriptorInputField>,
}

/// Translation of `DescriptorGlobalState`.
#[derive(Default)]
struct DescriptorGlobalState {
    usage_page: u32,
    report_size: u32,
    report_count: u32,
    report_id: u32,
}

/// Translation of `DescriptorLocalState`.
#[derive(Default)]
struct DescriptorLocalState {
    usage_minimum: u32,
    usage_maximum: u32,
    usages: Vec<u32>,
}

/// Translation of `DescriptorContext`.
#[derive(Default)]
struct DescriptorContext {
    collection_depth: i32,
    global: DescriptorGlobalState,
    local: DescriptorLocalState,
    field_offset: i32,
    fields: Vec<DescriptorInputField>,
}

/// Translation of `ReadValue()`: little endian, at most 4 bytes.
fn read_value(data: &[u8]) -> u32 {
    let mut value = 0u32;
    for (i, &byte) in data.iter().enumerate() {
        // FIXME (upstream): more than four bytes shift past the width of
        // the value (undefined in C); here the extra bytes are ignored.
        if i < 4 {
            value |= u32::from(byte) << (8 * i);
        }
    }
    value
}

impl DescriptorContext {
    /// Translation of `ResetLocalState()`.
    fn reset_local_state(&mut self) {
        self.local.usage_minimum = 0;
        self.local.usage_maximum = 0;
        self.local.usages.clear();
    }

    /// Translation of `AddUsage()`.
    fn add_usage(&mut self, mut usage: u32) {
        if usage <= 0xFFFF {
            usage |= self.global.usage_page << 16;
        }
        self.local.usages.push(usage);
    }

    /// Translation of `AddInputField()`.
    fn add_input_field(&mut self, usage: u32, bit_size: i32) {
        self.fields.push(DescriptorInputField {
            report_id: self.global.report_id as u8,
            usage,
            bit_offset: self.field_offset,
            bit_size,
        });
    }

    /// Translation of `AddInputFields()`.
    fn add_input_fields(&mut self) {
        let mut usage = 0u32;

        if self.global.report_count == 0 || self.global.report_size == 0 {
            return;
        }

        if self.local.usages.is_empty()
            && self.local.usage_minimum > 0
            && self.local.usage_maximum >= self.local.usage_minimum
        {
            for usage in self.local.usage_minimum..=self.local.usage_maximum {
                self.add_usage(usage);
            }
        }

        let mut usage_index = 0;
        for _ in 0..self.global.report_count {
            if usage_index < self.local.usages.len() {
                usage = self.local.usages[usage_index];
                if usage_index < self.local.usages.len() - 1 {
                    usage_index += 1;
                }
            }

            let size = self.global.report_size as i32;
            if usage > 0 {
                self.add_input_field(usage, size);
            }
            self.field_offset = self.field_offset.wrapping_add(size);
        }
    }

    /// Translation of `ParseMainItem()`.
    fn parse_main_item(&mut self, tag: u8, data: &[u8]) {
        match tag {
            MAIN_TAG_INPUT => {
                let _flags = read_value(data);
                self.add_input_fields();
            }
            MAIN_TAG_COLLECTION => {
                // FIXME (upstream): the collection type is read (only for
                // its debug trace) even when the item has no data, one byte
                // past the item, possibly past the descriptor; it isn't read
                // here.
                self.collection_depth += 1;
            }
            MAIN_TAG_END_COLLECTION if self.collection_depth > 0 => {
                self.collection_depth -= 1;
            }
            // (Output, Feature and unknown tags only have a debug trace)
            _ => {}
        }

        self.reset_local_state();
    }

    /// Translation of `ParseGlobalItem()`.
    fn parse_global_item(&mut self, tag: u8, data: &[u8]) {
        match tag {
            GLOBAL_TAG_USAGE_PAGE => self.global.usage_page = read_value(data),
            GLOBAL_TAG_REPORT_SIZE => self.global.report_size = read_value(data),
            GLOBAL_TAG_REPORT_ID => {
                self.global.report_id = read_value(data);
                self.field_offset = 0;
            }
            GLOBAL_TAG_REPORT_COUNT => self.global.report_count = read_value(data),
            // (the other tags only have a debug trace)
            _ => {}
        }
    }

    /// Translation of `ParseLocalItem()`.
    fn parse_local_item(&mut self, tag: u8, data: &[u8]) {
        match tag {
            LOCAL_TAG_USAGE => {
                let value = read_value(data);
                self.add_usage(value);
            }
            LOCAL_TAG_USAGE_MINIMUM => self.local.usage_minimum = read_value(data),
            LOCAL_TAG_USAGE_MAXIMUM => self.local.usage_maximum = read_value(data),
            // (the other tags only have a debug trace)
            _ => {}
        }
    }

    /// Translation of `ParseDescriptor()`.
    fn parse_descriptor(&mut self, descriptor: &[u8]) -> Result<()> {
        const SIZES: [usize; 4] = [0, 1, 2, 4];

        let mut here = 0;
        while here < descriptor.len() {
            let data = descriptor[here];
            here += 1;
            let size = SIZES[usize::from(data & 0x3)];
            let item_type = (data >> 2) & 0x3;
            let tag = data >> 4;

            if here + size > descriptor.len() {
                return Err(Error::new("Invalid descriptor"));
            }

            let item = &descriptor[here..here + size];
            match item_type {
                DESCRIPTOR_ITEM_TYPE_MAIN => self.parse_main_item(tag, item),
                DESCRIPTOR_ITEM_TYPE_GLOBAL => self.parse_global_item(tag, item),
                DESCRIPTOR_ITEM_TYPE_LOCAL => self.parse_local_item(tag, item),
                _ => {
                    // Long items are currently unsupported
                    return Err(Error::unsupported());
                }
            }

            here += size;
        }
        Ok(())
    }
}

/// Parse a report descriptor. Translation of `SDL_ParseReportDescriptor()`.
pub(crate) fn parse_report_descriptor(descriptor: &[u8]) -> Result<ReportDescriptor> {
    let mut ctx = DescriptorContext::default();
    ctx.parse_descriptor(descriptor)?;
    Ok(ReportDescriptor { fields: ctx.fields })
}

impl ReportDescriptor {
    /// Whether a field has a usage. Translation of `SDL_DescriptorHasUsage()`.
    pub(crate) fn has_usage(&self, usage_page: u16, usage: u16) -> bool {
        let full_usage = make_usage(usage_page, usage);
        self.fields.iter().any(|field| field.usage == full_usage)
    }
}

/// Read a field from a report. `data` is the buffer holding the report,
/// which is `size` bytes long. Translation of `SDL_ReadReportData()`.
pub(crate) fn read_report_data(
    data: &[u8],
    size: usize,
    bit_offset: i32,
    bit_size: i32,
) -> Result<u32> {
    let offset = (bit_offset / 8) as usize;
    if offset >= size {
        return Err(Error::new("Out of bounds reading report data"));
    }

    // FIXME (upstream): the bytes of the field aren't checked against the
    // report size; they are read from the buffer (past its end they read as
    // 0 here, where C would read past it).
    let num_bytes = ((bit_size + 7) / 8).max(0) as usize;
    let bytes: Vec<u8> = (offset..offset + num_bytes)
        .map(|i| data.get(i).copied().unwrap_or(0))
        .collect();
    let mut value = read_value(&bytes);

    let shift = bit_offset % 8;
    if shift > 0 {
        value >>= shift;
    }

    match bit_size {
        1 => value &= 0x1,
        4 => value &= 0xf,
        10 => value &= 0x3ff,
        15 => value &= 0x7fff,
        _ => {
            crate::sdl_assert!((bit_size % 8) == 0);
        }
    }
    Ok(value)
}

#[cfg(test)]
mod tests;
