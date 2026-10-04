// Rust translation of src/hidapi/windows/hidapi_descriptor_reconstruct.c and
// hidapi_descriptor_reconstruct.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.
//
// HIDAPI - Multi-Platform library for communication with HID devices.
// libusb/hidapi Team. Copyright 2022, All Rights Reserved. At the
// discretion of the user of this library, this software may be licensed
// under the terms of the GNU General Public License v3, a BSD-Style
// license, or the original HIDAPI license as outlined in the LICENSE.txt,
// LICENSE-gpl3.txt, LICENSE-bsd.txt, and LICENSE-orig.txt files located at
// the root of the source distribution. These files may also be found in
// the public source code repository located at:
// https://github.com/libusb/hidapi .

//! Windows doesn't give out HID report descriptors; this rebuilds one from
//! the "preparsed data" the HID class driver keeps for a device
//! (`HidD_GetPreparsedData()`), whose undocumented layout is declared here.
//!
//! The algorithm is platform independent and works on the preparsed data
//! as bytes, so it is tested everywhere. Upstream's linked list of main
//! items is a vector of nodes linked by index.

/// `NUM_OF_HIDP_REPORT_TYPES`
const NUM_OF_HIDP_REPORT_TYPES: usize = 3;

/// The report descriptor items. Translation of `rd_items`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
enum RdItems {
    MainInput = 0x80,              // 1000 00 nn
    MainOutput = 0x90,             // 1001 00 nn
    MainFeature = 0xB0,            // 1011 00 nn
    MainCollection = 0xA0,         // 1010 00 nn
    MainCollectionEnd = 0xC0,      // 1100 00 nn
    GlobalUsagePage = 0x04,        // 0000 01 nn
    GlobalLogicalMinimum = 0x14,   // 0001 01 nn
    GlobalLogicalMaximum = 0x24,   // 0010 01 nn
    GlobalPhysicalMinimum = 0x34,  // 0011 01 nn
    GlobalPhysicalMaximum = 0x44,  // 0100 01 nn
    GlobalUnitExponent = 0x54,     // 0101 01 nn
    GlobalUnit = 0x64,             // 0110 01 nn
    GlobalReportSize = 0x74,       // 0111 01 nn
    GlobalReportId = 0x84,         // 1000 01 nn
    GlobalReportCount = 0x94,      // 1001 01 nn
    LocalUsage = 0x08,             // 0000 10 nn
    LocalUsageMinimum = 0x18,      // 0001 10 nn
    LocalUsageMaximum = 0x28,      // 0010 10 nn
    LocalDesignatorIndex = 0x38,   // 0011 10 nn
    LocalDesignatorMinimum = 0x48, // 0100 10 nn
    LocalDesignatorMaximum = 0x58, // 0101 10 nn
    LocalString = 0x78,            // 0111 10 nn
    LocalStringMinimum = 0x88,     // 1000 10 nn
    LocalStringMaximum = 0x98,     // 1001 10 nn
    LocalDelimiter = 0xA8,         // 1010 10 nn
}

/// The kinds of main item. Translation of `rd_main_items` (the first
/// three are the `HIDP_REPORT_TYPE`s).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RdMainItems {
    Input = 0,
    Output = 1,
    Feature = 2,
    Collection,
    CollectionEnd,
    DelimiterOpen,
    DelimiterUsage,
    DelimiterClose,
}

impl RdMainItems {
    /// The main item of a report type index.
    fn from_report_type(rt_idx: usize) -> RdMainItems {
        match rt_idx {
            0 => RdMainItems::Input,
            1 => RdMainItems::Output,
            _ => RdMainItems::Feature,
        }
    }

    /// The report type index of an Input, Output or Feature item.
    fn report_type(self) -> Option<usize> {
        match self {
            RdMainItems::Input => Some(0),
            RdMainItems::Output => Some(1),
            RdMainItems::Feature => Some(2),
            _ => None,
        }
    }
}

/// Translation of `rd_bit_range`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct RdBitRange {
    first_bit: i32,
    last_bit: i32,
}

/// Translation of `rd_node_type`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RdNodeType {
    Cap,
    Padding,
    Collection,
}

/// Translation of `struct rd_main_item_node`.
#[derive(Clone, Copy, Debug)]
struct RdMainItemNode {
    /// Position of first bit in report (counting from 0)
    first_bit: i32,
    /// Position of last bit in report (counting from 0)
    last_bit: i32,
    /// Information if caps index refers to the array of button caps, value
    /// caps, or if the node is just a padding element to fill unused bit
    /// positions. The node can also be a collection node without any bits
    /// in the report.
    type_of_node: RdNodeType,
    /// Index in the array of caps
    caps_index: i32,
    /// Index in the array of link collections
    collection_index: i32,
    /// Input, Output, Feature, Collection or Collection End
    main_item_type: RdMainItems,
    report_id: u8,
    next: Option<usize>,
}

/// Translation of `hid_pp_caps_info`.
#[derive(Clone, Copy, Debug)]
struct HidPpCapsInfo {
    first_cap: u16,
    last_cap: u16,
}

/// Translation of `hid_pp_link_collection_node` (16 bytes).
#[derive(Clone, Copy, Debug)]
struct HidPpLinkCollectionNode {
    link_usage: u16,
    link_usage_page: u16,
    parent: u16,
    number_of_children: u16,
    next_sibling: u16,
    first_child: u16,
    collection_type: u8,
    is_alias: bool,
}

/// Translation of `hid_pp_cap` (104 bytes; the unions are read as both of
/// their views).
#[derive(Clone, Copy, Debug)]
struct HidPpCap {
    usage_page: u16,
    report_id: u8,
    bit_position: u8,
    /// WIN32 term for this is BitSize
    report_size: u16,
    report_count: u16,
    byte_position: u16,
    bit_field: u32,
    link_collection: u16,
    is_button_cap: bool,
    is_range: bool,
    /// IsAlias is set to TRUE in the first n-1 capability structures added
    /// to the capability array. IsAlias set to FALSE in the nth capability
    /// structure.
    is_alias: bool,
    is_string_range: bool,
    is_designator_range: bool,
    // Range
    usage_min: u16,
    usage_max: u16,
    string_min: u16,
    string_max: u16,
    designator_min: u16,
    designator_max: u16,
    data_index_min: u16,
    data_index_max: u16,
    // NotRange (Usage, StringIndex, DesignatorIndex share the Range fields)
    // Button
    button_logical_min: i32,
    button_logical_max: i32,
    // NotButton
    not_button_logical_min: i32,
    not_button_logical_max: i32,
    physical_min: i32,
    physical_max: i32,
    units: u32,
    units_exp: u32,
}

impl HidPpCap {
    /// `NotRange.Usage`
    fn usage(&self) -> u16 {
        self.usage_min
    }
    /// `NotRange.StringIndex`
    fn string_index(&self) -> u16 {
        self.string_min
    }
    /// `NotRange.DesignatorIndex`
    fn designator_index(&self) -> u16 {
        self.designator_min
    }
}

/// `offsetof(hidp_preparsed_data, caps)`
pub(super) const PP_DATA_CAPS_OFFSET: usize = 44;
/// `sizeof(hid_pp_cap)`
const PP_CAP_SIZE: usize = 104;
/// `sizeof(hid_pp_link_collection_node)`
const PP_LINK_COLLECTION_NODE_SIZE: usize = 16;

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

/// The size of a preparsed data structure, from its header (the caps and
/// the link collection array that follows them), or `None` if `header` is
/// shorter than the header.
pub(super) fn preparsed_data_size(header: &[u8]) -> Option<usize> {
    let first_byte_of_link_collection_array = usize::from(u16_at(header, 40)?);
    let number_link_collection_nodes = usize::from(u16_at(header, 42)?);
    Some(
        PP_DATA_CAPS_OFFSET
            + first_byte_of_link_collection_array
            + number_link_collection_nodes * PP_LINK_COLLECTION_NODE_SIZE,
    )
}

/// The parsed `hidp_preparsed_data`.
struct PreparsedData {
    caps_info: [HidPpCapsInfo; 3],
    caps: Vec<HidPpCap>,
    link_collection_nodes: Vec<HidPpLinkCollectionNode>,
}

impl PreparsedData {
    /// Read the structure from its bytes; `None` if it doesn't fit them.
    fn parse(pp_data: &[u8]) -> Option<PreparsedData> {
        let mut caps_info = [HidPpCapsInfo {
            first_cap: 0,
            last_cap: 0,
        }; 3];
        for (rt_idx, info) in caps_info.iter_mut().enumerate() {
            let base = 16 + rt_idx * 8;
            info.first_cap = u16_at(pp_data, base)?;
            info.last_cap = u16_at(pp_data, base + 4)?;
        }
        let first_byte_of_link_collection_array = usize::from(u16_at(pp_data, 40)?);
        let number_link_collection_nodes = usize::from(u16_at(pp_data, 42)?);

        let number_of_caps = caps_info
            .iter()
            .map(|info| usize::from(info.last_cap))
            .max()
            .unwrap_or(0);
        let mut caps = Vec::with_capacity(number_of_caps);
        for caps_idx in 0..number_of_caps {
            caps.push(Self::parse_cap(
                pp_data,
                PP_DATA_CAPS_OFFSET + caps_idx * PP_CAP_SIZE,
            )?);
        }

        let mut link_collection_nodes = Vec::with_capacity(number_link_collection_nodes);
        for i in 0..number_link_collection_nodes {
            let base = PP_DATA_CAPS_OFFSET
                + first_byte_of_link_collection_array
                + i * PP_LINK_COLLECTION_NODE_SIZE;
            let bits = u32_at(pp_data, base + 12)?;
            link_collection_nodes.push(HidPpLinkCollectionNode {
                link_usage: u16_at(pp_data, base)?,
                link_usage_page: u16_at(pp_data, base + 2)?,
                parent: u16_at(pp_data, base + 4)?,
                number_of_children: u16_at(pp_data, base + 6)?,
                next_sibling: u16_at(pp_data, base + 8)?,
                first_child: u16_at(pp_data, base + 10)?,
                collection_type: bits as u8,
                is_alias: bits & 0x100 != 0,
            });
        }

        Some(PreparsedData {
            caps_info,
            caps,
            link_collection_nodes,
        })
    }

    fn parse_cap(pp_data: &[u8], base: usize) -> Option<HidPpCap> {
        let flags = *pp_data.get(base + 24)?;
        let i32_at = |offset: usize| u32_at(pp_data, base + offset).map(|v| v as i32);
        Some(HidPpCap {
            usage_page: u16_at(pp_data, base)?,
            report_id: *pp_data.get(base + 2)?,
            bit_position: *pp_data.get(base + 3)?,
            report_size: u16_at(pp_data, base + 4)?,
            report_count: u16_at(pp_data, base + 6)?,
            byte_position: u16_at(pp_data, base + 8)?,
            bit_field: u32_at(pp_data, base + 12)?,
            link_collection: u16_at(pp_data, base + 18)?,
            is_button_cap: flags & 0x04 != 0,
            is_range: flags & 0x10 != 0,
            is_alias: flags & 0x20 != 0,
            is_string_range: flags & 0x40 != 0,
            is_designator_range: flags & 0x80 != 0,
            usage_min: u16_at(pp_data, base + 60)?,
            usage_max: u16_at(pp_data, base + 62)?,
            string_min: u16_at(pp_data, base + 64)?,
            string_max: u16_at(pp_data, base + 66)?,
            designator_min: u16_at(pp_data, base + 68)?,
            designator_max: u16_at(pp_data, base + 70)?,
            data_index_min: u16_at(pp_data, base + 72)?,
            data_index_max: u16_at(pp_data, base + 74)?,
            button_logical_min: i32_at(76)?,
            button_logical_max: i32_at(80)?,
            not_button_logical_min: i32_at(80)?,
            not_button_logical_max: i32_at(84)?,
            physical_min: i32_at(88)?,
            physical_max: i32_at(92)?,
            units: u32_at(pp_data, base + 96)?,
            units_exp: u32_at(pp_data, base + 100)?,
        })
    }
}

/// References to report descriptor buffer. Translation of `struct rd_buffer`.
struct RdBuffer<'a> {
    /// The array which stores the reconstructed descriptor
    buf: &'a mut [u8],
    /// Index of the next report byte to write to buf array
    byte_idx: usize,
}

impl RdBuffer<'_> {
    /// Function that appends a byte to encoded report descriptor buffer.
    /// Translation of `rd_append_byte()`.
    fn append_byte(&mut self, byte: u8) {
        if self.byte_idx < self.buf.len() {
            self.buf[self.byte_idx] = byte;
            self.byte_idx += 1;
        }
    }

    /// Writes a short report descriptor item according USB HID spec 1.11
    /// chapter 6.2.2.2. `data` size depends on `rd_item`: 0, 1, 2 or 4
    /// bytes. Returns `Err` for data out of range. Translation of
    /// `rd_write_short_item()`.
    fn write_short_item(&mut self, rd_item: RdItems, data: i64) -> Result<(), ()> {
        let item = rd_item as u8;
        if item & 0x03 != 0 {
            // Invalid input data, last to bits are reserved for data size
            return Err(());
        }

        if rd_item == RdItems::MainCollectionEnd {
            // Item without data (1Byte prefix only)
            self.append_byte(item);
        } else if matches!(
            rd_item,
            RdItems::GlobalLogicalMinimum
                | RdItems::GlobalLogicalMaximum
                | RdItems::GlobalPhysicalMinimum
                | RdItems::GlobalPhysicalMaximum
        ) {
            // Item with signed integer data
            if (-128..=127).contains(&data) {
                // 1Byte prefix + 1Byte data
                self.append_byte(item + 0x01);
                self.append_byte(data as u8);
            } else if (-32768..=32767).contains(&data) {
                // 1Byte prefix + 2Byte data
                self.append_byte(item + 0x02);
                for byte in &(data as i16).to_le_bytes() {
                    self.append_byte(*byte);
                }
            } else if (-2147483648..=2147483647).contains(&data) {
                // 1Byte prefix + 4Byte data
                self.append_byte(item + 0x03);
                for byte in &(data as i32).to_le_bytes() {
                    self.append_byte(*byte);
                }
            } else {
                // Data out of 32 bit signed integer range
                return Err(());
            }
        } else {
            // Item with unsigned integer data
            if (0..=0xFF).contains(&data) {
                // 1Byte prefix + 1Byte data
                self.append_byte(item + 0x01);
                self.append_byte(data as u8);
            } else if (0..=0xFFFF).contains(&data) {
                // 1Byte prefix + 2Byte data
                self.append_byte(item + 0x02);
                for byte in &(data as u16).to_le_bytes() {
                    self.append_byte(*byte);
                }
            } else if (0..=0xFFFFFFFF).contains(&data) {
                // 1Byte prefix + 4Byte data
                self.append_byte(item + 0x03);
                for byte in &(data as u32).to_le_bytes() {
                    self.append_byte(*byte);
                }
            } else {
                // Data out of 32 bit unsigned integer range
                return Err(());
            }
        }
        Ok(())
    }

    /// [`RdBuffer::write_short_item`], ignoring the result as upstream does.
    fn write(&mut self, rd_item: RdItems, data: i64) {
        let _ = self.write_short_item(rd_item, data);
    }
}

/// The main item list: upstream's linked list, with nodes in an arena.
#[derive(Default)]
struct MainItemList {
    nodes: Vec<RdMainItemNode>,
}

impl MainItemList {
    fn node(&self, index: usize) -> &RdMainItemNode {
        &self.nodes[index]
    }

    /// Translation of `rd_append_main_item_node()`: appends a node at the
    /// end of the list starting at `*list` (which becomes the node if the
    /// list is empty).
    #[allow(clippy::too_many_arguments)]
    fn append(
        &mut self,
        first_bit: i32,
        last_bit: i32,
        type_of_node: RdNodeType,
        caps_index: i32,
        collection_index: i32,
        main_item_type: RdMainItems,
        report_id: u8,
        list: &mut Option<usize>,
    ) -> usize {
        let new_list_node = self.nodes.len();
        self.nodes.push(RdMainItemNode {
            first_bit,
            last_bit,
            type_of_node,
            caps_index,
            collection_index,
            main_item_type,
            report_id,
            next: None, // NULL marks last node in the list
        });

        // Determine last node in the list
        match *list {
            None => *list = Some(new_list_node),
            Some(mut last) => {
                while let Some(next) = self.nodes[last].next {
                    last = next;
                }
                self.nodes[last].next = Some(new_list_node);
            }
        }
        new_list_node
    }

    /// Translation of `rd_insert_main_item_node()`: inserts a node after
    /// the node `list`.
    #[allow(clippy::too_many_arguments)]
    fn insert(
        &mut self,
        first_bit: i32,
        last_bit: i32,
        type_of_node: RdNodeType,
        caps_index: i32,
        collection_index: i32,
        main_item_type: RdMainItems,
        report_id: u8,
        list: usize,
    ) -> usize {
        // Insert item after the main item node referenced by list
        let next_item = self.nodes[list].next.take();
        let new_node = self.append(
            first_bit,
            last_bit,
            type_of_node,
            caps_index,
            collection_index,
            main_item_type,
            report_id,
            &mut Some(list),
        );
        self.nodes[new_node].next = next_item;
        new_node
    }

    /// Translation of `rd_search_main_item_list_for_bit_position()`:
    /// determine first INPUT/OUTPUT/FEATURE main item, where the last bit
    /// position is equal or greater than the search bit position; returns
    /// the node before it.
    fn search_for_bit_position(
        &self,
        search_bit: i32,
        main_item_type: RdMainItems,
        report_id: u8,
        mut list: usize,
    ) -> usize {
        // FIXME (upstream): the search dereferences the next node without
        // checking for the end of the list; here it stops at the last node.
        while let Some(next) = self.nodes[list].next {
            let next = &self.nodes[next];
            if next.main_item_type == RdMainItems::Collection
                || next.main_item_type == RdMainItems::CollectionEnd
                || (next.last_bit >= search_bit
                    && next.report_id == report_id
                    && next.main_item_type == main_item_type)
            {
                break;
            }
            list = self.nodes[list].next.expect("checked above");
        }
        list
    }
}

/// `coll_bit_range[COLLECTION_INDEX][REPORT_ID][INPUT/OUTPUT/FEATURE]`
struct CollBitRanges {
    ranges: Vec<RdBitRange>,
}

impl CollBitRanges {
    fn new(number_of_collections: usize) -> CollBitRanges {
        CollBitRanges {
            ranges: vec![
                RdBitRange {
                    first_bit: -1,
                    last_bit: -1,
                };
                number_of_collections * 256 * NUM_OF_HIDP_REPORT_TYPES
            ],
        }
    }

    fn index(collection: usize, report_id: usize, rt_idx: usize) -> usize {
        (collection * 256 + report_id) * NUM_OF_HIDP_REPORT_TYPES + rt_idx
    }

    fn get(&self, collection: usize, report_id: usize, rt_idx: usize) -> RdBitRange {
        self.ranges[Self::index(collection, report_id, rt_idx)]
    }

    fn get_mut(&mut self, collection: usize, report_id: usize, rt_idx: usize) -> &mut RdBitRange {
        &mut self.ranges[Self::index(collection, report_id, rt_idx)]
    }
}

/// Rebuild the report descriptor of a device from its preparsed data,
/// into `buf`; returns the length written (the descriptor is cut short if
/// `buf` is too small). Fails if the preparsed data isn't valid.
/// Translation of `hid_winapi_descriptor_reconstruct_pp_data()`.
pub(super) fn reconstruct_pp_data(pp_data: &[u8], buf: &mut [u8]) -> Result<usize, ()> {
    // Check if MagicKey is correct, to ensure that pp_data points to an valid preparse data structure
    if pp_data.get(..8) != Some(b"HidP KDR".as_slice()) {
        return Err(());
    }

    let mut pp_data = PreparsedData::parse(pp_data).ok_or(())?;
    reconstruct(&mut pp_data, buf).ok_or(())
}

/// The algorithm of [`reconstruct_pp_data`]; `None` where upstream would
/// index out of bounds (malformed preparsed data).
#[allow(clippy::needless_range_loop)] // (the tables are indexed as upstream does)
fn reconstruct(pp_data: &mut PreparsedData, buf: &mut [u8]) -> Option<usize> {
    let mut rpt_desc = RdBuffer { buf, byte_idx: 0 };

    let link_collection_nodes = pp_data.link_collection_nodes.clone();
    let number_link_collection_nodes = link_collection_nodes.len();
    let link = |idx: usize| link_collection_nodes.get(idx);

    // Validate the indices upstream follows without checking
    for node in &link_collection_nodes {
        for idx in [node.parent, node.next_sibling, node.first_child] {
            if usize::from(idx) >= number_link_collection_nodes {
                return None;
            }
        }
    }
    for cap in &pp_data.caps {
        if usize::from(cap.link_collection) >= number_link_collection_nodes {
            return None;
        }
    }
    if number_link_collection_nodes == 0 {
        return None;
    }

    // ****************************************************************************************************************************
    // Create lookup tables for the bit range of each report per collection (position of first bit and last bit in each collection)
    // coll_bit_range[COLLECTION_INDEX][REPORT_ID][INPUT/OUTPUT/FEATURE]
    // ****************************************************************************************************************************

    // Allocate memory and initialize lookup table
    let mut coll_bit_range = CollBitRanges::new(number_link_collection_nodes);

    // Fill the lookup table where caps exist
    for rt_idx in 0..NUM_OF_HIDP_REPORT_TYPES {
        let info = pp_data.caps_info[rt_idx];
        for caps_idx in info.first_cap..info.last_cap {
            let cap = pp_data.caps[usize::from(caps_idx)];
            let first_bit = (i32::from(cap.byte_position) - 1) * 8 + i32::from(cap.bit_position);
            let last_bit = first_bit + i32::from(cap.report_size) * i32::from(cap.report_count) - 1;
            let range = coll_bit_range.get_mut(
                usize::from(cap.link_collection),
                usize::from(cap.report_id),
                rt_idx,
            );
            if range.first_bit == -1 || range.first_bit > first_bit {
                range.first_bit = first_bit;
            }
            if range.last_bit < last_bit {
                range.last_bit = last_bit;
            }
        }
    }

    // *************************************************************************
    // -Determine hierarchy levels of each collections and store it in:
    //  coll_levels[COLLECTION_INDEX]
    // -Determine number of direct childs of each collections and store it in:
    //  coll_number_of_direct_childs[COLLECTION_INDEX]
    // *************************************************************************
    let mut max_coll_level = 0;
    let mut coll_levels = vec![-1i32; number_link_collection_nodes];
    let mut coll_number_of_direct_childs = vec![0i32; number_link_collection_nodes];

    {
        let mut actual_coll_level = 0;
        let mut collection_node_idx = 0usize;
        while actual_coll_level >= 0 {
            coll_levels[collection_node_idx] = actual_coll_level;
            let node = link(collection_node_idx)?;
            if node.number_of_children > 0 && coll_levels[usize::from(node.first_child)] == -1 {
                actual_coll_level += 1;
                coll_levels[collection_node_idx] = actual_coll_level;
                if max_coll_level < actual_coll_level {
                    max_coll_level = actual_coll_level;
                }
                coll_number_of_direct_childs[collection_node_idx] += 1;
                collection_node_idx = usize::from(node.first_child);
            } else if node.next_sibling != 0 {
                coll_number_of_direct_childs[usize::from(node.parent)] += 1;
                collection_node_idx = usize::from(node.next_sibling);
            } else {
                actual_coll_level -= 1;
                if actual_coll_level >= 0 {
                    collection_node_idx = usize::from(node.parent);
                }
            }
        }
    }

    // *********************************************************************************
    // Propagate the bit range of each report from the child collections to their parent
    // and store the merged result for the parent
    // *********************************************************************************
    for actual_coll_level in (0..max_coll_level).rev() {
        for collection_node_idx in 0..number_link_collection_nodes {
            if coll_levels[collection_node_idx] == actual_coll_level {
                let mut child_idx = usize::from(link(collection_node_idx)?.first_child);
                while child_idx != 0 {
                    for reportid_idx in 0..256 {
                        for rt_idx in 0..NUM_OF_HIDP_REPORT_TYPES {
                            // Merge bit range from childs
                            let child = coll_bit_range.get(child_idx, reportid_idx, rt_idx);
                            let parent =
                                coll_bit_range.get_mut(collection_node_idx, reportid_idx, rt_idx);
                            if child.first_bit != -1 && parent.first_bit > child.first_bit {
                                parent.first_bit = child.first_bit;
                            }
                            if parent.last_bit < child.last_bit {
                                parent.last_bit = child.last_bit;
                            }
                            // FIXME (upstream): the next sibling is taken
                            // inside the report ID and report type loops,
                            // so each child only merges one (report ID,
                            // report type) range, and once the siblings run
                            // out the root collection (0) is "merged" in.
                            child_idx = usize::from(link(child_idx)?.next_sibling);
                        }
                    }
                }
            }
        }
    }

    // **************************************************************************************************
    // Determine child collection order of the whole hierarchy, based on previously determined bit ranges
    // and store it this index coll_child_order[COLLECTION_INDEX][DIRECT_CHILD_INDEX]
    // **************************************************************************************************
    let mut coll_child_order: Vec<Vec<u16>> = vec![Vec::new(); number_link_collection_nodes];
    {
        let mut coll_parsed_flag = vec![false; number_link_collection_nodes];
        let mut actual_coll_level = 0;
        let mut collection_node_idx = 0usize;
        while actual_coll_level >= 0 {
            let node = *link(collection_node_idx)?;
            if coll_number_of_direct_childs[collection_node_idx] != 0
                && !coll_parsed_flag[usize::from(node.first_child)]
            {
                coll_parsed_flag[usize::from(node.first_child)] = true;
                let number_of_childs = coll_number_of_direct_childs[collection_node_idx] as usize;
                let mut order = vec![0u16; number_of_childs];

                {
                    // Create list of child collection indices
                    // sorted reverse to the order returned to HidP_GetLinkCollectionNodeschild
                    // which seems to match the original order, as long as no bit position needs to be considered
                    let mut child_idx = node.first_child;
                    let mut child_count = number_of_childs.checked_sub(1)?;
                    order[child_count] = child_idx;
                    while link(usize::from(child_idx))?.next_sibling != 0 {
                        child_count = child_count.checked_sub(1)?;
                        child_idx = link(usize::from(child_idx))?.next_sibling;
                        order[child_count] = child_idx;
                    }
                }

                if number_of_childs > 1 {
                    // Sort child collections indices by bit positions
                    for rt_idx in 0..NUM_OF_HIDP_REPORT_TYPES {
                        for reportid_idx in 0..256 {
                            for child_idx in 1..number_of_childs {
                                // since the coll_bit_range array is not sorted, we need to reference the collection index in
                                // our sorted coll_child_order array, and look up the corresponding bit ranges for comparing values to sort
                                let prev_coll_idx = usize::from(order[child_idx - 1]);
                                let cur_coll_idx = usize::from(order[child_idx]);
                                let prev = coll_bit_range.get(prev_coll_idx, reportid_idx, rt_idx);
                                let cur = coll_bit_range.get(cur_coll_idx, reportid_idx, rt_idx);
                                if prev.first_bit != -1
                                    && cur.first_bit != -1
                                    && prev.first_bit > cur.first_bit
                                {
                                    // Swap position indices of the two compared child collections
                                    order.swap(child_idx - 1, child_idx);
                                }
                            }
                        }
                    }
                }
                coll_child_order[collection_node_idx] = order;
                actual_coll_level += 1;
                collection_node_idx = usize::from(node.first_child);
            } else if node.next_sibling != 0 {
                collection_node_idx = usize::from(node.next_sibling);
            } else {
                actual_coll_level -= 1;
                if actual_coll_level >= 0 {
                    collection_node_idx = usize::from(node.parent);
                }
            }
        }
    }
    let child_order = |collection: usize, child: usize| -> Option<usize> {
        coll_child_order
            .get(collection)?
            .get(child)
            .map(|&c| usize::from(c))
    };

    // ***************************************************************************************
    // Create sorted main_item_list containing all the Collection and CollectionEnd main items
    // ***************************************************************************************
    let mut list = MainItemList::default();
    let mut main_item_list: Option<usize> = None; // List root
                                                  // Lookup table to find the Collection items in the list by index
    let mut coll_begin_lookup: Vec<Option<usize>> = vec![None; number_link_collection_nodes];
    let mut coll_end_lookup: Vec<Option<usize>> = vec![None; number_link_collection_nodes];
    {
        let mut coll_last_written_child = vec![-1i32; number_link_collection_nodes];

        let mut actual_coll_level = 0;
        let mut collection_node_idx = 0usize;
        let mut first_delimiter_node: Option<usize> = None;
        let mut delimiter_close_node: Option<usize> = None;
        coll_begin_lookup[0] = Some(list.append(
            0,
            0,
            RdNodeType::Collection,
            0,
            collection_node_idx as i32,
            RdMainItems::Collection,
            0,
            &mut main_item_list,
        ));
        while actual_coll_level >= 0 {
            let number_of_childs = coll_number_of_direct_childs[collection_node_idx];
            if number_of_childs != 0 && coll_last_written_child[collection_node_idx] == -1 {
                // Collection has child collections, but none is written to the list yet

                let first = child_order(collection_node_idx, 0)?;
                coll_last_written_child[collection_node_idx] = first as i32;
                collection_node_idx = first;

                // In a HID Report Descriptor, the first usage declared is the most preferred usage for the control.
                // While the order in the WIN32 capabiliy strutures is the opposite:
                // Here the preferred usage is the last aliased usage in the sequence.

                if link(collection_node_idx)?.is_alias && first_delimiter_node.is_none() {
                    // Alliased Collection (First node in link_collection_nodes -> Last entry in report descriptor output)
                    first_delimiter_node = main_item_list;
                    coll_begin_lookup[collection_node_idx] = Some(list.append(
                        0,
                        0,
                        RdNodeType::Collection,
                        0,
                        collection_node_idx as i32,
                        RdMainItems::DelimiterUsage,
                        0,
                        &mut main_item_list,
                    ));
                    coll_begin_lookup[collection_node_idx] = Some(list.append(
                        0,
                        0,
                        RdNodeType::Collection,
                        0,
                        collection_node_idx as i32,
                        RdMainItems::DelimiterClose,
                        0,
                        &mut main_item_list,
                    ));
                    delimiter_close_node = main_item_list;
                } else {
                    // Normal not aliased collection
                    coll_begin_lookup[collection_node_idx] = Some(list.append(
                        0,
                        0,
                        RdNodeType::Collection,
                        0,
                        collection_node_idx as i32,
                        RdMainItems::Collection,
                        0,
                        &mut main_item_list,
                    ));
                    actual_coll_level += 1;
                }
            } else if number_of_childs > 1
                && coll_last_written_child[collection_node_idx]
                    != child_order(collection_node_idx, number_of_childs as usize - 1)? as i32
            {
                // Collection has child collections, and this is not the first child

                let mut next_child = 1;
                while coll_last_written_child[collection_node_idx]
                    != child_order(collection_node_idx, next_child - 1)? as i32
                {
                    next_child += 1;
                }
                let next = child_order(collection_node_idx, next_child)?;
                coll_last_written_child[collection_node_idx] = next as i32;
                collection_node_idx = next;

                let is_alias = link(collection_node_idx)?.is_alias;
                if is_alias && first_delimiter_node.is_none() {
                    // Alliased Collection (First node in link_collection_nodes -> Last entry in report descriptor output)
                    first_delimiter_node = main_item_list;
                    coll_begin_lookup[collection_node_idx] = Some(list.append(
                        0,
                        0,
                        RdNodeType::Collection,
                        0,
                        collection_node_idx as i32,
                        RdMainItems::DelimiterUsage,
                        0,
                        &mut main_item_list,
                    ));
                    coll_begin_lookup[collection_node_idx] = Some(list.append(
                        0,
                        0,
                        RdNodeType::Collection,
                        0,
                        collection_node_idx as i32,
                        RdMainItems::DelimiterClose,
                        0,
                        &mut main_item_list,
                    ));
                    delimiter_close_node = main_item_list;
                } else if is_alias {
                    let first = first_delimiter_node?;
                    coll_begin_lookup[collection_node_idx] = Some(list.insert(
                        0,
                        0,
                        RdNodeType::Collection,
                        0,
                        collection_node_idx as i32,
                        RdMainItems::DelimiterUsage,
                        0,
                        first,
                    ));
                } else if let Some(first) = first_delimiter_node {
                    coll_begin_lookup[collection_node_idx] = Some(list.insert(
                        0,
                        0,
                        RdNodeType::Collection,
                        0,
                        collection_node_idx as i32,
                        RdMainItems::DelimiterUsage,
                        0,
                        first,
                    ));
                    coll_begin_lookup[collection_node_idx] = Some(list.insert(
                        0,
                        0,
                        RdNodeType::Collection,
                        0,
                        collection_node_idx as i32,
                        RdMainItems::DelimiterOpen,
                        0,
                        first,
                    ));
                    first_delimiter_node = None;
                    main_item_list = delimiter_close_node;
                    delimiter_close_node = None; // Last entry of alias has .IsAlias == FALSE
                }
                if !is_alias {
                    coll_begin_lookup[collection_node_idx] = Some(list.append(
                        0,
                        0,
                        RdNodeType::Collection,
                        0,
                        collection_node_idx as i32,
                        RdMainItems::Collection,
                        0,
                        &mut main_item_list,
                    ));
                    actual_coll_level += 1;
                }
            } else {
                actual_coll_level -= 1;
                coll_end_lookup[collection_node_idx] = Some(list.append(
                    0,
                    0,
                    RdNodeType::Collection,
                    0,
                    collection_node_idx as i32,
                    RdMainItems::CollectionEnd,
                    0,
                    &mut main_item_list,
                ));
                collection_node_idx = usize::from(link(collection_node_idx)?.parent);
            }
        }
    }

    // ****************************************************************
    // Inserted Input/Output/Feature main items into the main_item_list
    // in order of reconstructed bit positions
    // ****************************************************************
    for rt_idx in 0..NUM_OF_HIDP_REPORT_TYPES {
        // Add all value caps to node list
        let mut first_delimiter_node: Option<usize> = None;
        let mut delimiter_close_node: Option<usize> = None;
        let main_item_type = RdMainItems::from_report_type(rt_idx);
        let info = pp_data.caps_info[rt_idx];
        for caps_idx in info.first_cap..info.last_cap {
            let cap = pp_data.caps[usize::from(caps_idx)];
            let link_collection = usize::from(cap.link_collection);
            let mut coll_begin = coll_begin_lookup[link_collection]?;
            let first_bit = (i32::from(cap.byte_position) - 1) * 8 + i32::from(cap.bit_position);
            let last_bit = first_bit + i32::from(cap.report_size) * i32::from(cap.report_count) - 1;

            for child_idx in 0..coll_number_of_direct_childs[link_collection] as usize {
                // Determine in which section before/between/after child collection the item should be inserted
                let child = child_order(link_collection, child_idx)?;
                if first_bit
                    < coll_bit_range
                        .get(child, usize::from(cap.report_id), rt_idx)
                        .first_bit
                {
                    // Note, that the default value for undefined coll_bit_range is -1, which can't be greater than the bit position
                    break;
                }
                coll_begin = coll_end_lookup[child]?;
            }
            let mut list_node =
                list.search_for_bit_position(first_bit, main_item_type, cap.report_id, coll_begin);

            // In a HID Report Descriptor, the first usage declared is the most preferred usage for the control.
            // While the order in the WIN32 capabiliy strutures is the opposite:
            // Here the preferred usage is the last aliased usage in the sequence.

            let caps_index = i32::from(caps_idx);
            let collection_index = i32::from(cap.link_collection);
            if cap.is_alias && first_delimiter_node.is_none() {
                // Alliased Usage (First node in pp_data->caps -> Last entry in report descriptor output)
                first_delimiter_node = Some(list_node);
                list.insert(
                    first_bit,
                    last_bit,
                    RdNodeType::Cap,
                    caps_index,
                    collection_index,
                    RdMainItems::DelimiterUsage,
                    cap.report_id,
                    list_node,
                );
                list.insert(
                    first_bit,
                    last_bit,
                    RdNodeType::Cap,
                    caps_index,
                    collection_index,
                    RdMainItems::DelimiterClose,
                    cap.report_id,
                    list_node,
                );
                delimiter_close_node = Some(list_node);
            } else if cap.is_alias {
                list.insert(
                    first_bit,
                    last_bit,
                    RdNodeType::Cap,
                    caps_index,
                    collection_index,
                    RdMainItems::DelimiterUsage,
                    cap.report_id,
                    list_node,
                );
            } else if first_delimiter_node.is_some() {
                // Alliased Collection (Last node in pp_data->caps -> First entry in report descriptor output)
                list.insert(
                    first_bit,
                    last_bit,
                    RdNodeType::Cap,
                    caps_index,
                    collection_index,
                    RdMainItems::DelimiterUsage,
                    cap.report_id,
                    list_node,
                );
                list.insert(
                    first_bit,
                    last_bit,
                    RdNodeType::Cap,
                    caps_index,
                    collection_index,
                    RdMainItems::DelimiterOpen,
                    cap.report_id,
                    list_node,
                );
                first_delimiter_node = None;
                list_node = delimiter_close_node?;
                delimiter_close_node = None; // Last entry of alias has .IsAlias == FALSE
            }
            if !cap.is_alias {
                list.insert(
                    first_bit,
                    last_bit,
                    RdNodeType::Cap,
                    caps_index,
                    collection_index,
                    main_item_type,
                    cap.report_id,
                    list_node,
                );
            }
        }
    }

    // ***********************************************************
    // Add const main items for padding to main_item_list
    // -To fill all bit gaps
    // -At each report end for 8bit padding
    //  Note that information about the padding at the report end,
    //  is not stored in the preparsed data, but in practice all
    //  report descriptors seem to have it, as assumed here.
    // ***********************************************************
    {
        let mut last_bit_position = [[-1i32; 256]; NUM_OF_HIDP_REPORT_TYPES];
        let mut last_report_item_lookup = [[None::<usize>; 256]; NUM_OF_HIDP_REPORT_TYPES];

        let mut node_idx = main_item_list?; // List root;

        while let Some(next) = list.node(node_idx).next {
            let node = *list.node(node_idx);
            if let Some(rt) = node.main_item_type.report_type() {
                // INPUT, OUTPUT or FEATURE
                let report_id = usize::from(node.report_id);
                if node.first_bit != -1 {
                    if let Some(last_item) = last_report_item_lookup[rt][report_id] {
                        if last_bit_position[rt][report_id] + 1 != node.first_bit
                            && list.node(last_item).first_bit != node.first_bit
                        // Happens in case of IsMultipleItemsForArray for multiple dedicated usages for a multi-button array
                        {
                            let list_node = list.search_for_bit_position(
                                last_bit_position[rt][report_id],
                                node.main_item_type,
                                node.report_id,
                                last_item,
                            );
                            list.insert(
                                last_bit_position[rt][report_id] + 1,
                                node.first_bit - 1,
                                RdNodeType::Padding,
                                -1,
                                0,
                                node.main_item_type,
                                node.report_id,
                                list_node,
                            );
                        }
                    }
                    last_bit_position[rt][report_id] = node.last_bit;
                    last_report_item_lookup[rt][report_id] = Some(node_idx);
                }
            }
            node_idx = next;
        }
        // Add 8 bit padding at each report end
        for rt_idx in 0..NUM_OF_HIDP_REPORT_TYPES {
            for reportid_idx in 0..256 {
                if last_bit_position[rt_idx][reportid_idx] != -1 {
                    let padding = 8 - ((last_bit_position[rt_idx][reportid_idx] + 1) % 8);
                    if padding < 8 {
                        // Insert padding item after item referenced in last_report_item_lookup
                        list.insert(
                            last_bit_position[rt_idx][reportid_idx] + 1,
                            last_bit_position[rt_idx][reportid_idx] + padding,
                            RdNodeType::Padding,
                            -1,
                            0,
                            RdMainItems::from_report_type(rt_idx),
                            reportid_idx as u8,
                            last_report_item_lookup[rt_idx][reportid_idx]?,
                        );
                    }
                }
            }
        }
    }

    // ***********************************
    // Encode the report descriptor output
    // ***********************************
    let mut last_report_id: u8 = 0;
    let mut last_usage_page: u16 = 0;
    let mut last_physical_min: i32 = 0; // If both, Physical Minimum and Physical Maximum are 0, the logical limits should be taken as physical limits according USB HID spec 1.11 chapter 6.2.2.7
    let mut last_physical_max: i32 = 0;
    let mut last_unit_exponent: u32 = 0; // If Unit Exponent is Undefined it should be considered as 0 according USB HID spec 1.11 chapter 6.2.2.7
    let mut last_unit: u32 = 0; // If the first nibble is 7, or second nibble of Unit is 0, the unit is None according USB HID spec 1.11 chapter 6.2.2.7
    let mut inhibit_write_of_usage = false; // Needed in case of delimited usage print, before the normal collection or cap
    let mut report_count: i32 = 0;
    let caps = &mut pp_data.caps;
    let mut cursor = main_item_list;
    while let Some(node_idx) = cursor {
        let node = *list.node(node_idx);
        let rt_idx = node.main_item_type;
        let caps_idx = node.caps_index;
        let collection = |node: &RdMainItemNode| link(node.collection_index as usize);
        if node.main_item_type == RdMainItems::Collection {
            let coll = collection(&node)?;
            if last_usage_page != coll.link_usage_page {
                // Write "Usage Page" at the begin of a collection - except it refers the same table as wrote last
                rpt_desc.write(RdItems::GlobalUsagePage, i64::from(coll.link_usage_page));
                last_usage_page = coll.link_usage_page;
            }
            if inhibit_write_of_usage {
                // Inhibit only once after DELIMITER statement
                inhibit_write_of_usage = false;
            } else {
                // Write "Usage" of collection
                rpt_desc.write(RdItems::LocalUsage, i64::from(coll.link_usage));
            }
            // Write begin of "Collection"
            rpt_desc.write(RdItems::MainCollection, i64::from(coll.collection_type));
        } else if node.main_item_type == RdMainItems::CollectionEnd {
            // Write "End Collection"
            rpt_desc.write(RdItems::MainCollectionEnd, 0);
        } else if node.main_item_type == RdMainItems::DelimiterOpen {
            if node.collection_index != -1 {
                // Write "Usage Page" inside of a collection delmiter section
                let coll = collection(&node)?;
                if last_usage_page != coll.link_usage_page {
                    rpt_desc.write(RdItems::GlobalUsagePage, i64::from(coll.link_usage_page));
                    last_usage_page = coll.link_usage_page;
                }
            } else if node.caps_index != 0 {
                // Write "Usage Page" inside of a main item delmiter section
                let cap = caps.get(caps_idx as usize)?;
                if cap.usage_page != last_usage_page {
                    rpt_desc.write(RdItems::GlobalUsagePage, i64::from(cap.usage_page));
                    last_usage_page = cap.usage_page;
                }
            }
            // Write "Delimiter Open"
            rpt_desc.write(RdItems::LocalDelimiter, 1); // 1 = open set of aliased usages
        } else if node.main_item_type == RdMainItems::DelimiterUsage {
            if node.collection_index != -1 {
                // Write aliased collection "Usage"
                rpt_desc.write(
                    RdItems::LocalUsage,
                    i64::from(collection(&node)?.link_usage),
                );
            }
            // (upstream: "}  if", not "else if")
            if node.caps_index != 0 {
                // Write aliased main item range from "Usage Minimum" to "Usage Maximum"
                let cap = caps.get(caps_idx as usize)?;
                if cap.is_range {
                    rpt_desc.write(RdItems::LocalUsageMinimum, i64::from(cap.usage_min));
                    rpt_desc.write(RdItems::LocalUsageMaximum, i64::from(cap.usage_max));
                } else {
                    // Write single aliased main item "Usage"
                    rpt_desc.write(RdItems::LocalUsage, i64::from(cap.usage()));
                }
            }
        } else if node.main_item_type == RdMainItems::DelimiterClose {
            // Write "Delimiter Close"
            rpt_desc.write(RdItems::LocalDelimiter, 0); // 0 = close set of aliased usages
                                                        // Inhibit next usage write
            inhibit_write_of_usage = true;
        } else if node.type_of_node == RdNodeType::Padding {
            // Padding
            // The preparsed data doesn't contain any information about padding. Therefore all undefined gaps
            // in the reports are filled with the same style of constant padding.

            // Write "Report Size" with number of padding bits
            rpt_desc.write(
                RdItems::GlobalReportSize,
                i64::from(node.last_bit - node.first_bit + 1),
            );

            // Write "Report Count" for padding always as 1
            rpt_desc.write(RdItems::GlobalReportCount, 1);

            match rt_idx {
                // Write "Input" main item - We know it's Constant - We can only guess the other bits, but they don't matter in case of const
                RdMainItems::Input => rpt_desc.write(RdItems::MainInput, 0x03), // Const / Abs
                // Write "Output" main item - We know it's Constant - We can only guess the other bits, but they don't matter in case of const
                RdMainItems::Output => rpt_desc.write(RdItems::MainOutput, 0x03), // Const / Abs
                // Write "Feature" main item - We know it's Constant - We can only guess the other bits, but they don't matter in case of const
                RdMainItems::Feature => rpt_desc.write(RdItems::MainFeature, 0x03), // Const / Abs
                _ => {}
            }
            report_count = 0;
        } else {
            let cap = *caps.get(caps_idx as usize)?;
            let next = node.next.map(|next| *list.node(next));
            let next_cap = match next {
                Some(next) if next.type_of_node == RdNodeType::Cap => {
                    Some(*caps.get(next.caps_index as usize)?)
                }
                _ => None,
            };

            if last_report_id != cap.report_id {
                // Write "Report ID" if changed
                rpt_desc.write(RdItems::GlobalReportId, i64::from(cap.report_id));
                last_report_id = cap.report_id;
            }

            // Write "Usage Page" when changed
            if cap.usage_page != last_usage_page {
                rpt_desc.write(RdItems::GlobalUsagePage, i64::from(cap.usage_page));
                last_usage_page = cap.usage_page;
            }

            if cap.is_button_cap {
                // Button
                // (The preparsed data contain different data for 1 bit Button caps, than for parametric Value caps)

                // Write only local report items for each cap, if ReportCount > 1
                if cap.is_range {
                    report_count += i32::from(cap.data_index_max) - i32::from(cap.data_index_min);
                }
            }

            if inhibit_write_of_usage {
                // Inhibit only once after Delimiter - Reset flag
                inhibit_write_of_usage = false;
            } else if cap.is_range {
                // Write range from "Usage Minimum" to "Usage Maximum"
                rpt_desc.write(RdItems::LocalUsageMinimum, i64::from(cap.usage_min));
                rpt_desc.write(RdItems::LocalUsageMaximum, i64::from(cap.usage_max));
            } else {
                // Write single "Usage"
                rpt_desc.write(RdItems::LocalUsage, i64::from(cap.usage()));
            }

            if cap.is_designator_range {
                // Write physical descriptor indices range from "Designator Minimum" to "Designator Maximum"
                rpt_desc.write(
                    RdItems::LocalDesignatorMinimum,
                    i64::from(cap.designator_min),
                );
                rpt_desc.write(
                    RdItems::LocalDesignatorMaximum,
                    i64::from(cap.designator_max),
                );
            } else if cap.designator_index() != 0 {
                // Designator set 0 is a special descriptor set (of the HID Physical Descriptor),
                // that specifies the number of additional descriptor sets.
                // Therefore Designator Index 0 can never be a useful reference for a control and we can inhibit it.
                // Write single "Designator Index"
                rpt_desc.write(
                    RdItems::LocalDesignatorIndex,
                    i64::from(cap.designator_index()),
                );
            }

            if cap.is_string_range {
                // Write range of indices of the USB string descriptor, from "String Minimum" to "String Maximum"
                rpt_desc.write(RdItems::LocalStringMinimum, i64::from(cap.string_min));
                rpt_desc.write(RdItems::LocalStringMaximum, i64::from(cap.string_max));
            } else if cap.string_index() != 0 {
                // String Index 0 is a special entry of the USB string descriptor, that contains a list of supported languages,
                // therefore Designator Index 0 can never be a useful reference for a control and we can inhibit it.
                // Write single "String Index"
                rpt_desc.write(RdItems::LocalString, i64::from(cap.string_index()));
            }

            let main_item = match rt_idx {
                RdMainItems::Input => Some(RdItems::MainInput),
                RdMainItems::Output => Some(RdItems::MainOutput),
                RdMainItems::Feature => Some(RdItems::MainFeature),
                _ => None,
            };

            if cap.is_button_cap {
                let same_as_next = matches!((next, next_cap), (Some(next), Some(next_cap))
                    if next.main_item_type == rt_idx
                        && next_cap.is_button_cap
                        && !cap.is_range // This node in list is no array
                        && !next_cap.is_range // Next node in list is no array
                        && next_cap.usage_page == cap.usage_page
                        && next_cap.report_id == cap.report_id
                        && next_cap.bit_field == cap.bit_field);
                if same_as_next {
                    if next.expect("matched above").first_bit != node.first_bit {
                        // In case of IsMultipleItemsForArray for multiple dedicated usages for a multi-button array, the report count should be incremented

                        // Skip global items until any of them changes, than use ReportCount item to write the count of identical report fields
                        report_count += 1;
                    }
                } else {
                    if cap.button_logical_min == 0 && cap.button_logical_max == 0 {
                        // While a HID report descriptor must always contain LogicalMinimum and LogicalMaximum,
                        // the preparsed data contain both fields set to zero, for the case of simple buttons
                        // Write "Logical Minimum" set to 0 and "Logical Maximum" set to 1
                        rpt_desc.write(RdItems::GlobalLogicalMinimum, 0);
                        rpt_desc.write(RdItems::GlobalLogicalMaximum, 1);
                    } else {
                        // Write logical range from "Logical Minimum" to "Logical Maximum"
                        rpt_desc.write(
                            RdItems::GlobalLogicalMinimum,
                            i64::from(cap.button_logical_min),
                        );
                        rpt_desc.write(
                            RdItems::GlobalLogicalMaximum,
                            i64::from(cap.button_logical_max),
                        );
                    }

                    // Write "Report Size"
                    rpt_desc.write(RdItems::GlobalReportSize, i64::from(cap.report_size));

                    // Write "Report Count"
                    if !cap.is_range {
                        // Variable bit field with one bit per button
                        // In case of multiple usages with the same items, only "Usage" is written per cap, and "Report Count" is incremented
                        rpt_desc.write(
                            RdItems::GlobalReportCount,
                            i64::from(cap.report_count) + i64::from(report_count),
                        );
                    } else {
                        // Button array of "Report Size" x "Report Count
                        rpt_desc.write(RdItems::GlobalReportCount, i64::from(cap.report_count));
                    }

                    // Buttons have only 1 bit and therefore no physical limits/units -> Set to undefined state
                    if last_physical_min != 0 {
                        // Write "Physical Minimum", but only if changed
                        last_physical_min = 0;
                        rpt_desc
                            .write(RdItems::GlobalPhysicalMinimum, i64::from(last_physical_min));
                    }
                    if last_physical_max != 0 {
                        // Write "Physical Maximum", but only if changed
                        last_physical_max = 0;
                        rpt_desc
                            .write(RdItems::GlobalPhysicalMaximum, i64::from(last_physical_max));
                    }
                    if last_unit_exponent != 0 {
                        // Write "Unit Exponent", but only if changed
                        last_unit_exponent = 0;
                        rpt_desc.write(RdItems::GlobalUnitExponent, i64::from(last_unit_exponent));
                    }
                    if last_unit != 0 {
                        // Write "Unit",but only if changed
                        last_unit = 0;
                        rpt_desc.write(RdItems::GlobalUnit, i64::from(last_unit));
                    }

                    // Write "Input"/"Output"/"Feature" main item
                    if let Some(main_item) = main_item {
                        rpt_desc.write(main_item, i64::from(cap.bit_field));
                    }
                    report_count = 0;
                }
            } else {
                let mut cap = cap;
                if (cap.bit_field & 0x02) != 0x02 {
                    // In case of an value array overwrite "Report Count"
                    // (upstream writes this into the preparsed data)
                    cap.report_count = cap
                        .data_index_max
                        .wrapping_sub(cap.data_index_min)
                        .wrapping_add(1);
                    caps[caps_idx as usize].report_count = cap.report_count;
                }
                // (read after that write, as upstream reads it)
                let next_cap = match next {
                    Some(next) if next.type_of_node == RdNodeType::Cap => {
                        Some(*caps.get(next.caps_index as usize)?)
                    }
                    _ => None,
                };

                // Print only local report items for each cap, if ReportCount > 1
                let same_as_next = matches!((next, next_cap), (Some(next), Some(next_cap))
                    if next.main_item_type == rt_idx
                        && !next_cap.is_button_cap
                        && !cap.is_range // This node in list is no array
                        && !next_cap.is_range // Next node in list is no array
                        && next_cap.usage_page == cap.usage_page
                        && next_cap.not_button_logical_min == cap.not_button_logical_min
                        && next_cap.not_button_logical_max == cap.not_button_logical_max
                        && next_cap.physical_min == cap.physical_min
                        && next_cap.physical_max == cap.physical_max
                        && next_cap.units_exp == cap.units_exp
                        && next_cap.units == cap.units
                        && next_cap.report_size == cap.report_size
                        && next_cap.report_id == cap.report_id
                        && next_cap.bit_field == cap.bit_field
                        && next_cap.report_count == 1
                        && cap.report_count == 1);
                if same_as_next {
                    // Skip global items until any of them changes, than use ReportCount item to write the count of identical report fields
                    report_count += 1;
                } else {
                    // Value

                    // Write logical range from "Logical Minimum" to "Logical Maximum"
                    rpt_desc.write(
                        RdItems::GlobalLogicalMinimum,
                        i64::from(cap.not_button_logical_min),
                    );
                    rpt_desc.write(
                        RdItems::GlobalLogicalMaximum,
                        i64::from(cap.not_button_logical_max),
                    );

                    if last_physical_min != cap.physical_min
                        || last_physical_max != cap.physical_max
                    {
                        // Write range from "Physical Minimum" to " Physical Maximum", but only if one of them changed
                        rpt_desc.write(RdItems::GlobalPhysicalMinimum, i64::from(cap.physical_min));
                        last_physical_min = cap.physical_min;
                        rpt_desc.write(RdItems::GlobalPhysicalMaximum, i64::from(cap.physical_max));
                        last_physical_max = cap.physical_max;
                    }

                    if last_unit_exponent != cap.units_exp {
                        // Write "Unit Exponent", but only if changed
                        rpt_desc.write(RdItems::GlobalUnitExponent, i64::from(cap.units_exp));
                        last_unit_exponent = cap.units_exp;
                    }

                    if last_unit != cap.units {
                        // Write physical "Unit", but only if changed
                        rpt_desc.write(RdItems::GlobalUnit, i64::from(cap.units));
                        last_unit = cap.units;
                    }

                    // Write "Report Size"
                    rpt_desc.write(RdItems::GlobalReportSize, i64::from(cap.report_size));

                    // Write "Report Count"
                    rpt_desc.write(
                        RdItems::GlobalReportCount,
                        i64::from(cap.report_count) + i64::from(report_count),
                    );

                    // Write "Input"/"Output"/"Feature" main item
                    if let Some(main_item) = main_item {
                        rpt_desc.write(main_item, i64::from(cap.bit_field));
                    }
                    report_count = 0;
                }
            }
        }

        // Go to next item in main_item_list
        cursor = node.next;
    }

    Some(rpt_desc.byte_idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! case {
        ($name:literal) => {
            (
                $name,
                include_bytes!(concat!("testdata/", $name, ".pp_data")).as_slice(),
                include_bytes!(concat!("testdata/", $name, ".rpt_desc")).as_slice(),
            )
        };
    }

    /// Upstream's `windows/test/data`: the preparsed data of 24 devices
    /// (converted from the text dumps to the binary structure by the loader
    /// of upstream's `hid_report_reconstructor_test.c`, built with MinGW)
    /// and the descriptors upstream's reconstructor gives for them (equal to
    /// the `_expected.rpt_desc` files).
    const CASES: [(&str, &[u8], &[u8]); 24] = [
        case!("045E_02FF_0005_0001"),
        case!("046A_0011_0006_0001"),
        case!("046D_0A37_0001_000C"),
        case!("046D_B010_0001_000C"),
        case!("046D_B010_0001_FF00"),
        case!("046D_B010_0002_0001"),
        case!("046D_B010_0002_FF00"),
        case!("046D_B010_0006_0001"),
        case!("046D_C077_0002_0001"),
        case!("046D_C283_0004_0001"),
        case!("046D_C52F_0001_000C"),
        case!("046D_C52F_0001_FF00"),
        case!("046D_C52F_0002_0001"),
        case!("046D_C52F_0002_FF00"),
        case!("046D_C534_0001_000C"),
        case!("046D_C534_0001_FF00"),
        case!("046D_C534_0002_0001"),
        case!("046D_C534_0002_FF00"),
        case!("046D_C534_0006_0001"),
        case!("046D_C534_0080_0001"),
        case!("047F_C056_0001_000C"),
        case!("047F_C056_0003_FFA0"),
        case!("047F_C056_0005_000B"),
        case!("17CC_1130_0000_FF01"),
    ];

    #[test]
    fn reconstructs_upstream_test_data() {
        for (name, pp_data, expected) in CASES {
            assert_eq!(preparsed_data_size(pp_data), Some(pp_data.len()), "{name}");
            let mut buf = [0u8; super::super::MAX_REPORT_DESCRIPTOR_SIZE];
            let len = reconstruct_pp_data(pp_data, &mut buf).unwrap_or_else(|()| panic!("{name}"));
            assert_eq!(&buf[..len], expected, "{name}");
        }
    }

    #[test]
    fn short_buffers_and_bad_data() {
        let (_, pp_data, expected) = CASES[0];
        // The descriptor is cut short to fit the buffer
        let mut buf = [0u8; 10];
        assert_eq!(reconstruct_pp_data(pp_data, &mut buf), Ok(10));
        assert_eq!(buf, expected[..10]);
        // Not preparsed data
        let mut bad = pp_data.to_vec();
        bad[0] = b'X';
        assert_eq!(reconstruct_pp_data(&bad, &mut buf), Err(()));
        // Truncated, or with an index out of range
        assert_eq!(reconstruct_pp_data(&pp_data[..100], &mut buf), Err(()));
        let mut bad = pp_data.to_vec();
        let parent = PP_DATA_CAPS_OFFSET + usize::from(u16_at(pp_data, 40).unwrap()) + 4;
        bad[parent] = 0xff;
        assert_eq!(reconstruct_pp_data(&bad, &mut buf), Err(()));
    }

    #[test]
    fn short_items() {
        let mut buf = [0u8; 16];
        let mut rd = RdBuffer {
            buf: &mut buf,
            byte_idx: 0,
        };
        rd.write(RdItems::GlobalLogicalMinimum, -1);
        rd.write(RdItems::GlobalLogicalMaximum, 300);
        rd.write(RdItems::GlobalReportCount, 0x10000);
        rd.write(RdItems::MainCollectionEnd, 0);
        assert_eq!(
            rd.write_short_item(RdItems::GlobalPhysicalMinimum, 1 << 40),
            Err(())
        );
        assert_eq!(rd.write_short_item(RdItems::GlobalUnit, -1), Err(()));
        let len = rd.byte_idx;
        assert_eq!(
            &buf[..len],
            [0x15, 0xff, 0x26, 0x2c, 0x01, 0x97, 0x00, 0x00, 0x01, 0x00, 0xc0]
        );
    }
}
