// A writer of DEX files, for the Java classes of super::classes.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The DEX file (`classes.dex`) of [`CLASSES`], which Android's runtime
//! (ART) loads the app's classes from, written here and kept generated in
//! `classes_dex.rs`; the test `classes_dex_is_current` checks that it is
//! current, and writes it again with `SDL3_ANDROID_WRITE_DEX=1`:
//!
//! ```sh
//! SDL3_ANDROID_WRITE_DEX=1 cargo test -p sdl3 --lib core::android::dex
//! ```
//!
//! It's the format of version 035, which every Android version reads (API
//! 13 and later), as Android's documentation describes it ("Dalvik
//! Executable format"):
//!
//! * the header, then the identifier lists: the strings, the types, the
//!   prototypes, the fields, the methods (each sorted as the format asks:
//!   strings by their UTF-16 code units, types by their name's index,
//!   prototypes by return type then parameters, fields and methods by
//!   class, name, then type or prototype), and the class definitions (a
//!   class after its superclass and interfaces when they are in the file);
//! * then the data: the code items (4-byte aligned), the type lists (the
//!   prototypes' parameters and the classes' interfaces, 4-byte aligned,
//!   each once), the strings' data (Modified UTF-8, with their UTF-16
//!   lengths), the classes' data (their fields and methods, by index, with
//!   their access flags and code), and the map of it all, in offset order;
//! * the SHA-1 signature of everything after it, then the Adler-32
//!   checksum of everything after it.
//!
//! The only bytecode is the constructors' (`invoke-direct` of the
//! superclass's constructor, `invoke-direct/range` with more than five
//! registers, then `return-void`) and the static initializer's
//! (`const-string` and `invoke-static` of `System.loadLibrary()` for each
//! library, then `return-void`). There is no debug information, no
//! annotation and no source file name.

use super::classes::{Class, Code, CLASSES, CONSTRUCTOR, PRIVATE, STATIC};
use std::collections::{BTreeMap, BTreeSet};

/// The DEX file of `classes`.
pub(crate) fn write(classes: &[Class]) -> Vec<u8> {
    Writer::new(classes).write()
}

/// The type descriptor of a class name (`org/libsdl/app/SDLActivity`:
/// `Lorg/libsdl/app/SDLActivity;`).
fn class_descriptor(name: &str) -> String {
    format!("L{name};")
}

/// A method descriptor's return type and parameter types.
fn parse_method_descriptor(descriptor: &str) -> (String, Vec<String>) {
    let inner = descriptor.strip_prefix('(').expect("a method descriptor");
    let (params, ret) = inner.split_once(')').expect("a method descriptor");
    let mut types = Vec::new();
    let bytes = params.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        while bytes[i] == b'[' {
            i += 1;
        }
        if bytes[i] == b'L' {
            i += params[i..].find(';').expect("a class type") + 1;
        } else {
            i += 1;
        }
        types.push(params[start..i].to_string());
    }
    (ret.to_string(), types)
}

/// The shorty of a prototype.
fn shorty(ret: &str, params: &[String]) -> String {
    std::iter::once(ret)
        .chain(params.iter().map(String::as_str))
        .map(|t| match t.as_bytes()[0] {
            b'L' | b'[' => 'L',
            c => c as char,
        })
        .collect()
}

/// How many registers a value of the type takes (`J` and `D`: two).
fn width(descriptor: &str) -> u16 {
    if descriptor == "J" || descriptor == "D" {
        2
    } else {
        1
    }
}

/// A string as its UTF-16 code units, the order of a DEX file's strings.
fn utf16_key(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// A prototype: its return type and parameter types.
type Proto = (String, Vec<String>);
/// A field reference: its class, name and type.
type FieldRef = (String, String, String);
/// A method reference: its class, name and prototype.
type MethodRef = (String, String, Proto);

struct Writer<'a> {
    classes: Vec<&'a Class>,
    strings: Vec<String>,
    string_index: BTreeMap<String, u32>,
    types: Vec<String>,
    type_index: BTreeMap<String, u32>,
    protos: Vec<Proto>,
    proto_index: BTreeMap<Proto, u32>,
    fields: Vec<FieldRef>,
    field_index: BTreeMap<FieldRef, u32>,
    methods: Vec<MethodRef>,
    method_index: BTreeMap<MethodRef, u32>,
}

const SYSTEM: &str = "Ljava/lang/System;";
const STRING: &str = "Ljava/lang/String;";
const NO_INDEX: u32 = 0xffff_ffff;

/// The map's item types.
const TYPE_HEADER_ITEM: u16 = 0x0000;
const TYPE_STRING_ID_ITEM: u16 = 0x0001;
const TYPE_TYPE_ID_ITEM: u16 = 0x0002;
const TYPE_PROTO_ID_ITEM: u16 = 0x0003;
const TYPE_FIELD_ID_ITEM: u16 = 0x0004;
const TYPE_METHOD_ID_ITEM: u16 = 0x0005;
const TYPE_CLASS_DEF_ITEM: u16 = 0x0006;
const TYPE_MAP_LIST: u16 = 0x1000;
const TYPE_TYPE_LIST: u16 = 0x1001;
const TYPE_CLASS_DATA_ITEM: u16 = 0x2000;
const TYPE_CODE_ITEM: u16 = 0x2001;
const TYPE_STRING_DATA_ITEM: u16 = 0x2002;

/// The opcodes used.
const OP_RETURN_VOID: u16 = 0x0e;
const OP_CONST_STRING: u16 = 0x1a;
const OP_CONST_STRING_JUMBO: u16 = 0x1b;
const OP_INVOKE_DIRECT: u16 = 0x70;
const OP_INVOKE_STATIC: u16 = 0x71;
const OP_INVOKE_DIRECT_RANGE: u16 = 0x76;

const HEADER_SIZE: u32 = 0x70;
const ENDIAN_CONSTANT: u32 = 0x1234_5678;

/// A method's code item, before it's laid out.
struct CodeItem {
    registers: u16,
    ins: u16,
    outs: u16,
    insns: Vec<u16>,
}

impl<'a> Writer<'a> {
    fn new(table: &'a [Class]) -> Writer<'a> {
        let classes = definition_order(table);
        let mut strings = BTreeSet::new();
        let mut types = BTreeSet::new();
        let mut protos = BTreeSet::new();
        let mut fields = BTreeSet::new();
        let mut methods = BTreeSet::new();
        let mut method = |class: String, name: &str, proto: Proto| {
            methods.insert((class, name.to_string(), proto));
        };
        for class in &classes {
            let this = class_descriptor(class.name);
            let superclass = class_descriptor(class.superclass);
            types.insert(this.clone());
            types.insert(superclass.clone());
            types.extend(class.interfaces.iter().map(|i| class_descriptor(i)));
            for field in class.fields {
                fields.insert((
                    this.clone(),
                    field.name.to_string(),
                    field.descriptor.to_string(),
                ));
            }
            for m in class.methods {
                let proto = parse_method_descriptor(m.descriptor);
                match m.code {
                    Code::SuperConstructor => method(superclass.clone(), "<init>", proto.clone()),
                    Code::LoadLibraries(libraries) => {
                        method(
                            SYSTEM.to_string(),
                            "loadLibrary",
                            ("V".to_string(), vec![STRING.to_string()]),
                        );
                        strings.extend(libraries.iter().map(|l| l.to_string()));
                    }
                    Code::Native(_) => {}
                }
                method(this.clone(), m.name, proto);
            }
        }
        for (class, name, ty) in &fields {
            types.insert(class.clone());
            types.insert(ty.clone());
            strings.insert(name.clone());
        }
        for (class, name, proto) in &methods {
            types.insert(class.clone());
            strings.insert(name.clone());
            protos.insert(proto.clone());
        }
        for (ret, params) in &protos {
            types.insert(ret.clone());
            types.extend(params.iter().cloned());
            strings.insert(shorty(ret, params));
        }
        strings.extend(types.iter().cloned());

        let mut strings: Vec<String> = strings.into_iter().collect();
        strings.sort_by_key(|s| utf16_key(s));
        let string_index: BTreeMap<String, u32> = strings
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), i as u32))
            .collect();
        let mut types: Vec<String> = types.into_iter().collect();
        types.sort_by_key(|t| string_index[t]);
        let type_index: BTreeMap<String, u32> = types
            .iter()
            .enumerate()
            .map(|(i, t)| (t.clone(), i as u32))
            .collect();
        let mut protos: Vec<Proto> = protos.into_iter().collect();
        protos.sort_by_key(|(ret, params)| {
            (
                type_index[ret],
                params.iter().map(|p| type_index[p]).collect::<Vec<_>>(),
            )
        });
        let proto_index: BTreeMap<Proto, u32> = protos
            .iter()
            .enumerate()
            .map(|(i, p)| (p.clone(), i as u32))
            .collect();
        let mut fields: Vec<FieldRef> = fields.into_iter().collect();
        fields.sort_by_key(|(class, name, ty)| {
            (type_index[class], string_index[name], type_index[ty])
        });
        let field_index: BTreeMap<FieldRef, u32> = fields
            .iter()
            .enumerate()
            .map(|(i, f)| (f.clone(), i as u32))
            .collect();
        let mut methods: Vec<MethodRef> = methods.into_iter().collect();
        methods.sort_by_key(|(class, name, proto)| {
            (type_index[class], string_index[name], proto_index[proto])
        });
        let method_index: BTreeMap<MethodRef, u32> = methods
            .iter()
            .enumerate()
            .map(|(i, m)| (m.clone(), i as u32))
            .collect();
        assert!(
            types.len() <= 0x1_0000 && protos.len() <= 0x1_0000,
            "too many types or prototypes"
        );
        assert!(
            methods.len() <= 0x1_0000 && fields.len() <= 0x1_0000,
            "too many methods or fields"
        );
        Writer {
            classes,
            strings,
            string_index,
            types,
            type_index,
            protos,
            proto_index,
            fields,
            field_index,
            methods,
            method_index,
        }
    }

    fn method_ref(&self, class: &str, name: &str, proto: Proto) -> u16 {
        self.method_index[&(class.to_string(), name.to_string(), proto)] as u16
    }

    /// The code of a method that has some.
    fn code(&self, class: &Class, method: &super::classes::Method) -> Option<CodeItem> {
        let proto = parse_method_descriptor(method.descriptor);
        match method.code {
            Code::Native(_) => None,
            Code::SuperConstructor => {
                // invoke-direct {this, the parameters...}, super.<init>; return-void
                let registers = 1 + proto.1.iter().map(|p| width(p)).sum::<u16>();
                let target = self.method_ref(&class_descriptor(class.superclass), "<init>", proto);
                let mut insns = if registers <= 5 {
                    let mut regs = [0u16; 5];
                    for (i, r) in regs.iter_mut().enumerate().take(registers as usize) {
                        *r = i as u16;
                    }
                    vec![
                        (registers << 12) | (regs[4] << 8) | OP_INVOKE_DIRECT,
                        target,
                        (regs[3] << 12) | (regs[2] << 8) | (regs[1] << 4) | regs[0],
                    ]
                } else {
                    vec![(registers << 8) | OP_INVOKE_DIRECT_RANGE, target, 0]
                };
                insns.push(OP_RETURN_VOID);
                Some(CodeItem {
                    registers,
                    ins: registers,
                    outs: registers,
                    insns,
                })
            }
            Code::LoadLibraries(libraries) => {
                // const-string v0, "name"; invoke-static {v0}, System.loadLibrary; ...; return-void
                let load = self.method_ref(
                    SYSTEM,
                    "loadLibrary",
                    ("V".to_string(), vec![STRING.to_string()]),
                );
                let mut insns = Vec::new();
                for library in libraries {
                    let index = self.string_index[*library];
                    if index <= 0xffff {
                        insns.extend([OP_CONST_STRING, index as u16]);
                    } else {
                        insns.extend([OP_CONST_STRING_JUMBO, index as u16, (index >> 16) as u16]);
                    }
                    insns.extend([(1 << 12) | OP_INVOKE_STATIC, load, 0]);
                }
                insns.push(OP_RETURN_VOID);
                Some(CodeItem {
                    registers: 1,
                    ins: 0,
                    outs: 1,
                    insns,
                })
            }
        }
    }

    fn write(&self) -> Vec<u8> {
        let string_ids_off = HEADER_SIZE;
        let type_ids_off = string_ids_off + 4 * self.strings.len() as u32;
        let proto_ids_off = type_ids_off + 4 * self.types.len() as u32;
        let field_ids_off = proto_ids_off + 12 * self.protos.len() as u32;
        let method_ids_off = field_ids_off + 8 * self.fields.len() as u32;
        let class_defs_off = method_ids_off + 8 * self.methods.len() as u32;
        let data_off = class_defs_off + 32 * self.classes.len() as u32;

        let mut data = Data {
            base: data_off,
            bytes: Vec::new(),
        };
        let mut map = vec![(TYPE_HEADER_ITEM, 1, 0)];
        let sections = |map: &mut Vec<(u16, u32, u32)>, kind, count: usize, offset| {
            if count > 0 {
                map.push((kind, count as u32, offset));
            }
        };
        sections(
            &mut map,
            TYPE_STRING_ID_ITEM,
            self.strings.len(),
            string_ids_off,
        );
        sections(&mut map, TYPE_TYPE_ID_ITEM, self.types.len(), type_ids_off);
        sections(
            &mut map,
            TYPE_PROTO_ID_ITEM,
            self.protos.len(),
            proto_ids_off,
        );
        sections(
            &mut map,
            TYPE_FIELD_ID_ITEM,
            self.fields.len(),
            field_ids_off,
        );
        sections(
            &mut map,
            TYPE_METHOD_ID_ITEM,
            self.methods.len(),
            method_ids_off,
        );
        sections(
            &mut map,
            TYPE_CLASS_DEF_ITEM,
            self.classes.len(),
            class_defs_off,
        );

        // The code items.
        let mut code_offsets = BTreeMap::new();
        let mut first = None;
        for class in &self.classes {
            for method in class.methods {
                if let Some(code) = self.code(class, method) {
                    data.align(4);
                    first.get_or_insert(data.offset());
                    code_offsets
                        .insert((class.name, method.name, method.descriptor), data.offset());
                    data.u16(code.registers);
                    data.u16(code.ins);
                    data.u16(code.outs);
                    data.u16(0); // tries_size
                    data.u32(0); // debug_info_off
                    data.u32(code.insns.len() as u32);
                    for insn in code.insns {
                        data.u16(insn);
                    }
                }
            }
        }
        if let Some(offset) = first {
            sections(&mut map, TYPE_CODE_ITEM, code_offsets.len(), offset);
        }

        // The type lists: the prototypes' parameters, then the classes'
        // interfaces, each list once.
        let mut lists: Vec<Vec<String>> = self
            .protos
            .iter()
            .map(|(_, params)| params.clone())
            .collect();
        lists.extend(
            self.classes
                .iter()
                .map(|c| c.interfaces.iter().map(|i| class_descriptor(i)).collect()),
        );
        let mut list_offsets: BTreeMap<Vec<String>, u32> = BTreeMap::new();
        let mut first = None;
        for list in lists {
            if list.is_empty() || list_offsets.contains_key(&list) {
                continue;
            }
            data.align(4);
            first.get_or_insert(data.offset());
            list_offsets.insert(list.clone(), data.offset());
            data.u32(list.len() as u32);
            for ty in &list {
                data.u16(self.type_index[ty] as u16);
            }
        }
        if let Some(offset) = first {
            sections(&mut map, TYPE_TYPE_LIST, list_offsets.len(), offset);
        }

        // The strings' data.
        let mut string_offsets = Vec::new();
        sections(
            &mut map,
            TYPE_STRING_DATA_ITEM,
            self.strings.len(),
            data.offset(),
        );
        for s in &self.strings {
            string_offsets.push(data.offset());
            data.uleb128(s.encode_utf16().count() as u32);
            data.bytes.extend(mutf8(s));
            data.bytes.push(0);
        }

        // The classes' data.
        let mut class_data_offsets = Vec::new();
        sections(
            &mut map,
            TYPE_CLASS_DATA_ITEM,
            self.classes.len(),
            data.offset(),
        );
        for class in &self.classes {
            class_data_offsets.push(data.offset());
            let this = class_descriptor(class.name);
            let field = |f: &&super::classes::Field| {
                self.field_index[&(this.clone(), f.name.to_string(), f.descriptor.to_string())]
            };
            let mut static_fields: Vec<_> = class
                .fields
                .iter()
                .filter(|f| f.access & STATIC != 0)
                .collect();
            let mut instance_fields: Vec<_> = class
                .fields
                .iter()
                .filter(|f| f.access & STATIC == 0)
                .collect();
            static_fields.sort_by_key(field);
            instance_fields.sort_by_key(field);
            let method = |m: &&super::classes::Method| {
                self.method_index[&(
                    this.clone(),
                    m.name.to_string(),
                    parse_method_descriptor(m.descriptor),
                )]
            };
            // (direct: static, private or a constructor; the rest virtual)
            let direct_flags = STATIC | PRIVATE | CONSTRUCTOR;
            let mut direct: Vec<_> = class
                .methods
                .iter()
                .filter(|m| m.access & direct_flags != 0)
                .collect();
            let mut virtual_: Vec<_> = class
                .methods
                .iter()
                .filter(|m| m.access & direct_flags == 0)
                .collect();
            direct.sort_by_key(method);
            virtual_.sort_by_key(method);
            data.uleb128(static_fields.len() as u32);
            data.uleb128(instance_fields.len() as u32);
            data.uleb128(direct.len() as u32);
            data.uleb128(virtual_.len() as u32);
            for list in [&static_fields, &instance_fields] {
                let mut previous = 0;
                for f in list {
                    let index = field(f);
                    data.uleb128(index - previous);
                    data.uleb128(f.access);
                    previous = index;
                }
            }
            for list in [&direct, &virtual_] {
                let mut previous = 0;
                for m in list {
                    let index = method(m);
                    data.uleb128(index - previous);
                    data.uleb128(m.access);
                    data.uleb128(
                        code_offsets
                            .get(&(class.name, m.name, m.descriptor))
                            .copied()
                            .unwrap_or(0),
                    );
                    previous = index;
                }
            }
        }

        // The map, which ends the file.
        data.align(4);
        let map_off = data.offset();
        map.push((TYPE_MAP_LIST, 1, map_off));
        map.sort_by_key(|&(_, _, offset)| offset);
        data.u32(map.len() as u32);
        for (kind, count, offset) in map {
            data.u16(kind);
            data.u16(0);
            data.u32(count);
            data.u32(offset);
        }
        let file_size = data.offset();

        // The header and the identifiers.
        let mut out = Vec::with_capacity(file_size as usize);
        out.extend(b"dex\n035\0");
        out.extend([0; 4 + 20]); // the checksum and signature, below
        for value in [file_size, HEADER_SIZE, ENDIAN_CONSTANT, 0, 0, map_off] {
            out.extend(value.to_le_bytes());
        }
        let ids = [
            (self.strings.len(), string_ids_off),
            (self.types.len(), type_ids_off),
            (self.protos.len(), proto_ids_off),
            (self.fields.len(), field_ids_off),
            (self.methods.len(), method_ids_off),
            (self.classes.len(), class_defs_off),
            (data.bytes.len(), data_off),
        ];
        for (size, offset) in ids {
            out.extend((size as u32).to_le_bytes());
            out.extend((if size == 0 { 0 } else { offset }).to_le_bytes());
        }
        assert_eq!(out.len(), HEADER_SIZE as usize);
        for offset in string_offsets {
            out.extend(offset.to_le_bytes());
        }
        for ty in &self.types {
            out.extend(self.string_index[ty].to_le_bytes());
        }
        for (ret, params) in &self.protos {
            out.extend(self.string_index[&shorty(ret, params)].to_le_bytes());
            out.extend(self.type_index[ret].to_le_bytes());
            out.extend(list_offsets.get(params).copied().unwrap_or(0).to_le_bytes());
        }
        for (class, name, ty) in &self.fields {
            out.extend((self.type_index[class] as u16).to_le_bytes());
            out.extend((self.type_index[ty] as u16).to_le_bytes());
            out.extend(self.string_index[name].to_le_bytes());
        }
        for (class, name, proto) in &self.methods {
            out.extend((self.type_index[class] as u16).to_le_bytes());
            out.extend((self.proto_index[proto] as u16).to_le_bytes());
            out.extend(self.string_index[name].to_le_bytes());
        }
        for (class, data_offset) in self.classes.iter().zip(class_data_offsets) {
            let interfaces: Vec<String> = class
                .interfaces
                .iter()
                .map(|i| class_descriptor(i))
                .collect();
            for value in [
                self.type_index[&class_descriptor(class.name)],
                class.access,
                self.type_index[&class_descriptor(class.superclass)],
                list_offsets.get(&interfaces).copied().unwrap_or(0),
                NO_INDEX, // source_file_idx
                0,        // annotations_off
                data_offset,
                0, // static_values_off
            ] {
                out.extend(value.to_le_bytes());
            }
        }
        assert_eq!(out.len(), data_off as usize);
        out.extend(data.bytes);
        assert_eq!(out.len(), file_size as usize);

        let signature = sha1(&out[32..]);
        out[12..32].copy_from_slice(&signature);
        let checksum = adler32(&out[12..]);
        out[8..12].copy_from_slice(&checksum.to_le_bytes());
        out
    }
}

/// The data section as it's written.
struct Data {
    base: u32,
    bytes: Vec<u8>,
}

impl Data {
    fn offset(&self) -> u32 {
        self.base + self.bytes.len() as u32
    }

    fn align(&mut self, alignment: u32) {
        while !self.offset().is_multiple_of(alignment) {
            self.bytes.push(0);
        }
    }

    fn u16(&mut self, value: u16) {
        self.bytes.extend(value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend(value.to_le_bytes());
    }

    fn uleb128(&mut self, mut value: u32) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                self.bytes.push(byte);
                return;
            }
            self.bytes.push(byte | 0x80);
        }
    }
}

/// The classes in the order of their definitions: each after its
/// superclass and its interfaces when they're among them (else in the
/// table's order).
fn definition_order(table: &[Class]) -> Vec<&Class> {
    let mut order: Vec<&Class> = Vec::new();
    fn visit<'a>(class: &'a Class, table: &'a [Class], order: &mut Vec<&'a Class>, depth: usize) {
        assert!(depth <= table.len(), "{} extends itself", class.name);
        if order.iter().any(|c| c.name == class.name) {
            return;
        }
        for parent in std::iter::once(&class.superclass).chain(class.interfaces) {
            if let Some(parent) = table.iter().find(|c| c.name == *parent) {
                visit(parent, table, order, depth + 1);
            }
        }
        order.push(class);
    }
    for class in table {
        visit(class, table, &mut order, 0);
    }
    order
}

/// A string in Modified UTF-8 (Java's, a DEX file's and JNI's): UTF-8 but
/// for U+0000, which is two bytes (C0 80), and the characters beyond
/// U+FFFF, which are their UTF-16 surrogates, three bytes each.
pub(crate) fn mutf8(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for unit in s.encode_utf16() {
        let c = unit as u32;
        match c {
            0x01..=0x7f => out.push(c as u8),
            0x00 | 0x80..=0x7ff => out.extend([0xc0 | (c >> 6) as u8, 0x80 | (c & 0x3f) as u8]),
            _ => out.extend([
                0xe0 | (c >> 12) as u8,
                0x80 | ((c >> 6) & 0x3f) as u8,
                0x80 | (c & 0x3f) as u8,
            ]),
        }
    }
    out
}

/// Adler-32 (RFC 1950's), the DEX file's checksum.
pub(crate) fn adler32(data: &[u8]) -> u32 {
    const MOD_ADLER: u32 = 65521;
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &byte in chunk {
            a += byte as u32;
            b += a;
        }
        a %= MOD_ADLER;
        b %= MOD_ADLER;
    }
    (b << 16) | a
}

/// SHA-1 (FIPS 180-4's), the DEX file's signature.
pub(crate) fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend(((data.len() as u64) * 8).to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (hi, v) in h.iter_mut().zip([a, b, c, d, e]) {
            *hi = hi.wrapping_add(v);
        }
    }
    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

/// `classes_dex.rs`: the DEX file of [`CLASSES`] as a byte array.
pub(crate) fn classes_dex_source() -> String {
    let dex = write(CLASSES);
    let mut s = String::new();
    s.push_str(
        "// The DEX file of super::classes::CLASSES (classes.dex).\n\
         // Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>\n\
         // This is an altered (translated) version of the original software; see LICENSE.txt.\n\
         // Generated by super::dex (SDL3_ANDROID_WRITE_DEX=1 cargo test -p sdl3 --lib core::android::dex); do not edit.\n\
         \n\
         //! The classes the app's DEX file declares (super::classes::CLASSES),\n\
         //! as Android's runtime reads them: `classes.dex`. In Android builds\n\
         //! it's in a section of its own, `.sdl3_android_dex`, from where\n\
         //! `tools/android_run.py` puts it in the app's package.\n\
         \n",
    );
    s.push_str("/// `classes.dex`.\n");
    s.push_str("#[rustfmt::skip]\n");
    s.push_str(
        "#[cfg_attr(target_os = \"android\", used, link_section = \".sdl3_android_dex\")]\n",
    );
    s.push_str(&format!(
        "pub(crate) static SDL_CLASSES_DEX: [u8; {}] = [\n",
        dex.len()
    ));
    for line in dex.chunks(16) {
        s.push_str("   ");
        for byte in line {
            s.push_str(&format!(" 0x{byte:02x},"));
        }
        s.push('\n');
    }
    s.push_str("];\n");
    s
}

#[cfg(test)]
mod tests {
    use super::super::classes::{Field, Method, Native, NATIVE, PUBLIC};
    use super::*;

    #[test]
    fn checksums() {
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
        let a = vec![0xffu8; 100_000];
        // (the sums taken modulo 65521 along the way)
        let (mut x, mut y) = (1u64, 0u64);
        for &b in &a {
            x = (x + b as u64) % 65521;
            y = (y + x) % 65521;
        }
        assert_eq!(adler32(&a), ((y << 16) | x) as u32);
        let hex = |d: [u8; 20]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        assert_eq!(
            hex(sha1(&vec![b'a'; 1_000_000])),
            "34aa973cd4c4daa4f61eeb2bdbad27316534016f"
        );
    }

    #[test]
    fn modified_utf8() {
        assert_eq!(mutf8("main"), b"main");
        assert_eq!(mutf8("\0"), [0xc0, 0x80]);
        assert_eq!(mutf8("é"), "é".as_bytes());
        assert_eq!(mutf8("€"), "€".as_bytes());
        // (U+1F600: its surrogates D83D DE00)
        assert_eq!(mutf8("😀"), [0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80]);
    }

    /// `classes_dex.rs` is the DEX file of the table as it is now
    /// (written again with SDL3_ANDROID_WRITE_DEX=1).
    #[test]
    fn classes_dex_is_current() {
        let source = classes_dex_source();
        if std::env::var_os("SDL3_ANDROID_WRITE_DEX").is_some_and(|v| v == "1") {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/core/android/classes_dex.rs");
            std::fs::write(&path, &source).unwrap();
            println!("wrote {}", path.display());
            return;
        }
        assert!(
            source == include_str!("classes_dex.rs"),
            "sdl3/src/core/android/classes_dex.rs isn't the DEX file of classes.rs: write it again with \
             SDL3_ANDROID_WRITE_DEX=1 cargo test -p sdl3 --lib core::android::dex"
        );
        assert_eq!(
            super::super::classes::SDL_CLASSES_DEX[..],
            write(CLASSES)[..]
        );
    }

    /// The file the table makes, read back: its structure as the format
    /// asks, and the classes the table declares.
    #[test]
    fn classes_dex_reads_back() {
        let dex = write(CLASSES);
        let file = read(&dex);
        assert_eq!(file.classes.len(), CLASSES.len());
        for class in CLASSES {
            let read = file
                .classes
                .iter()
                .find(|c| c.name == class_descriptor(class.name))
                .unwrap_or_else(|| panic!("no {} in the file", class.name));
            assert_eq!(read.superclass, class_descriptor(class.superclass));
            assert_eq!(read.access, class.access);
            let interfaces: Vec<String> = class
                .interfaces
                .iter()
                .map(|i| class_descriptor(i))
                .collect();
            assert_eq!(read.interfaces, interfaces);
            let mut want: Vec<(String, String, u32, bool)> = class
                .methods
                .iter()
                .map(|m| {
                    let native = matches!(m.code, Code::Native(_));
                    (
                        m.name.to_string(),
                        m.descriptor.to_string(),
                        m.access,
                        !native,
                    )
                })
                .collect();
            want.sort();
            let mut got = read.methods.clone();
            got.sort();
            assert_eq!(got, want, "{}", class.name);
            let mut want: Vec<(String, String, u32)> = class
                .fields
                .iter()
                .map(|f| (f.name.to_string(), f.descriptor.to_string(), f.access))
                .collect();
            want.sort();
            let mut got = read.fields.clone();
            got.sort();
            assert_eq!(got, want, "{}", class.name);
        }
        // SDLActivity.<clinit>: const-string v0, "main"; invoke-static {v0},
        // System.loadLibrary; return-void
        let activity = &file
            .classes
            .iter()
            .find(|c| c.name == "Lorg/libsdl/app/SDLActivity;")
            .unwrap();
        let clinit = &activity.code["<clinit>()V"];
        assert_eq!((clinit.registers, clinit.ins, clinit.outs), (1, 0, 1));
        assert_eq!(clinit.insns.len(), 6);
        assert_eq!(file.strings[clinit.insns[1] as usize], "main");
        assert_eq!(
            file.methods[clinit.insns[3] as usize],
            "Ljava/lang/System;->loadLibrary(Ljava/lang/String;)V"
        );
        assert_eq!(
            [
                clinit.insns[0],
                clinit.insns[2],
                clinit.insns[4],
                clinit.insns[5]
            ],
            [0x1a, 0x1071, 0, 0x0e]
        );
        let init = &activity.code["<init>()V"];
        assert_eq!(init.insns.len(), 4);
        assert_eq!(
            file.methods[init.insns[1] as usize],
            "Landroid/app/Activity;-><init>()V"
        );
        assert_eq!(
            [init.insns[0], init.insns[2], init.insns[3]],
            [0x1070, 0, 0x0e]
        );
    }

    /// Constructors of every width (wide parameters, `/range` past five
    /// registers), fields, interfaces, a subclass of a class in the file,
    /// and strings beyond ASCII.
    #[test]
    fn other_classes() {
        static TABLE: &[Class] = &[
            // (before its superclass, which the file must reverse)
            Class {
                name: "org/libsdl/test/B",
                access: PUBLIC,
                superclass: "org/libsdl/test/A",
                interfaces: &["java/lang/Runnable", "android/view/View$OnKeyListener"],
                fields: &[],
                methods: &[
                    Method {
                        name: "<init>",
                        descriptor: "(JIDZ)V",
                        access: PUBLIC | CONSTRUCTOR,
                        code: Code::SuperConstructor,
                    },
                    Method {
                        name: "run",
                        descriptor: "()V",
                        access: PUBLIC | NATIVE,
                        code: Code::Native(native()),
                    },
                    Method {
                        name: "onKey",
                        descriptor: "(Landroid/view/View;ILandroid/view/KeyEvent;)Z",
                        access: PUBLIC | NATIVE,
                        code: Code::Native(native()),
                    },
                ],
            },
            Class {
                name: "org/libsdl/test/A",
                access: PUBLIC,
                superclass: "java/lang/Object",
                interfaces: &["java/lang/Runnable"],
                fields: &[
                    Field {
                        name: "ptr",
                        descriptor: "J",
                        access: PUBLIC,
                    },
                    Field {
                        name: "count",
                        descriptor: "I",
                        access: PUBLIC | STATIC,
                    },
                    Field {
                        name: "naïve",
                        descriptor: "[Ljava/lang/String;",
                        access: PRIVATE,
                    },
                ],
                methods: &[
                    Method {
                        name: "<init>",
                        descriptor: "()V",
                        access: PUBLIC | CONSTRUCTOR,
                        code: Code::SuperConstructor,
                    },
                    Method {
                        name: "<init>",
                        descriptor: "(JIDZ)V",
                        access: PUBLIC | CONSTRUCTOR,
                        code: Code::SuperConstructor,
                    },
                    Method {
                        name: "run",
                        descriptor: "()V",
                        access: PUBLIC | NATIVE,
                        code: Code::Native(native()),
                    },
                    Method {
                        name: "\u{1F600}",
                        descriptor: "(II)I",
                        access: PUBLIC | STATIC | NATIVE,
                        code: Code::Native(native()),
                    },
                ],
            },
        ];
        let dex = write(TABLE);
        let file = read(&dex);
        assert_eq!(file.classes[0].name, "Lorg/libsdl/test/A;");
        assert_eq!(file.classes[1].name, "Lorg/libsdl/test/B;");
        let a = &file.classes[0];
        assert_eq!(a.fields.len(), 3);
        // super(long, int, double, boolean): seven registers, so
        // invoke-direct/range {v0 .. v6}
        let wide = &a.code["<init>(JIDZ)V"];
        assert_eq!((wide.registers, wide.ins, wide.outs), (7, 7, 7));
        assert_eq!(wide.insns[0], 0x0776);
        assert_eq!(
            file.methods[wide.insns[1] as usize],
            "Ljava/lang/Object;-><init>(JIDZ)V"
        );
        assert_eq!(wide.insns[2], 0);
        let b = &file.classes[1];
        let init = &b.code["<init>(JIDZ)V"];
        assert_eq!(
            file.methods[init.insns[1] as usize],
            "Lorg/libsdl/test/A;-><init>(JIDZ)V"
        );
        assert_eq!(
            b.interfaces,
            ["Ljava/lang/Runnable;", "Landroid/view/View$OnKeyListener;"]
        );
        assert!(file.strings.iter().any(|s| s == "\u{1F600}"));
    }

    #[cfg(target_os = "android")]
    const fn native() -> Native {
        Native(std::ptr::null_mut())
    }

    #[cfg(not(target_os = "android"))]
    const fn native() -> Native {
        Native
    }

    /// dexdump (the SDK's build-tools': SDL3_ANDROID_BUILD_TOOLS's version,
    /// else the newest) verifies the file (ART's DEX file verifier) and
    /// lists it, with its code.
    #[test]
    fn dexdump_accepts_it() {
        let Some(dexdump) = dexdump() else {
            crate::test_support::skip(
                "android-sdk",
                "no dexdump (the Android SDK's build-tools, in ANDROID_HOME or ANDROID_SDK_ROOT)",
            );
            return;
        };
        let dir = std::env::temp_dir().join(format!("sdl3-dexdump-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("classes.dex");
        std::fs::write(&path, write(CLASSES)).unwrap();
        let run = |args: &[&str]| {
            let output = std::process::Command::new(&dexdump)
                .args(args)
                .arg(&path)
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&output.stdout).into_owned()
                + &String::from_utf8_lossy(&output.stderr);
            assert!(
                output.status.success(),
                "{} {args:?} failed ({}):\n{text}",
                dexdump.display(),
                output.status
            );
            text
        };
        // (-c: the checksum, -d: the code too, -f: the header)
        let checksum = run(&["-c"]);
        let listing = run(&["-d", "-f"]);
        std::fs::remove_dir_all(&dir).unwrap();
        for text in [&checksum, &listing] {
            assert!(
                !text.contains("ERROR") && !text.contains("Failure"),
                "{text}"
            );
        }
        for class in CLASSES {
            assert!(
                listing.contains(&format!(
                    "Class descriptor  : '{}'",
                    class_descriptor(class.name)
                )),
                "{listing}"
            );
        }
        assert!(
            listing.contains(
                "invoke-static {v0}, Ljava/lang/System;.loadLibrary:(Ljava/lang/String;)V"
            ),
            "{listing}"
        );
        println!(
            "{} accepts it:\n{}",
            dexdump.display(),
            listing.lines().take(40).collect::<Vec<_>>().join("\n")
        );
    }

    /// The build-tools' dexdump.
    fn dexdump() -> Option<std::path::PathBuf> {
        use std::path::PathBuf;
        let sdk = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
            .iter()
            .find_map(|name| std::env::var_os(name).map(PathBuf::from))?;
        let tools = sdk.join("build-tools");
        let versions: Vec<PathBuf> = match std::env::var_os("SDL3_ANDROID_BUILD_TOOLS") {
            Some(version) => vec![tools.join(version)],
            None => {
                let mut found: Vec<(Vec<u32>, PathBuf)> = std::fs::read_dir(&tools)
                    .ok()?
                    .flatten()
                    .map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let version = name
                            .split(['.', '-'])
                            .map(|p| p.parse().unwrap_or(0))
                            .collect();
                        (version, e.path())
                    })
                    .collect();
                found.sort();
                found.into_iter().rev().map(|(_, p)| p).collect()
            }
        };
        versions.into_iter().find_map(|dir| {
            let path = dir.join(if cfg!(windows) {
                "dexdump.exe"
            } else {
                "dexdump"
            });
            path.is_file().then_some(path)
        })
    }

    /// What `read` reads of a DEX file.
    struct File {
        strings: Vec<String>,
        methods: Vec<String>,
        classes: Vec<ReadClass>,
    }

    struct ReadClass {
        name: String,
        access: u32,
        superclass: String,
        interfaces: Vec<String>,
        /// (name, type, access)
        fields: Vec<(String, String, u32)>,
        /// (name, descriptor, access, whether it has code)
        methods: Vec<(String, String, u32, bool)>,
        /// The code items, by name and descriptor.
        code: BTreeMap<String, ReadCode>,
    }

    struct ReadCode {
        registers: u16,
        ins: u16,
        outs: u16,
        insns: Vec<u16>,
    }

    /// Read a DEX file, checking what the format asks of it as it goes:
    /// the header, the checksum and the signature, the identifiers'
    /// order, the map's offsets and order, the alignments.
    fn read(dex: &[u8]) -> File {
        let u2 = |o: usize| u16::from_le_bytes([dex[o], dex[o + 1]]);
        let u4 = |o: usize| u32::from_le_bytes([dex[o], dex[o + 1], dex[o + 2], dex[o + 3]]);
        let uleb = |o: &mut usize| {
            let (mut value, mut shift) = (0u32, 0);
            loop {
                let byte = dex[*o];
                *o += 1;
                value |= ((byte & 0x7f) as u32) << shift;
                shift += 7;
                if byte & 0x80 == 0 {
                    return value;
                }
            }
        };
        assert_eq!(&dex[..8], b"dex\n035\0");
        assert_eq!(u4(8), adler32(&dex[12..]), "checksum");
        assert_eq!(dex[12..32], sha1(&dex[32..]), "signature");
        assert_eq!(u4(32) as usize, dex.len(), "file_size");
        assert_eq!(u4(36), HEADER_SIZE);
        assert_eq!(u4(40), ENDIAN_CONSTANT);
        let [ns, so, nt, to, np, po, nf, fo, nm, mo, nc, co, ds, doff] =
            std::array::from_fn(|i| u4(56 + 4 * i) as usize);
        assert_eq!(doff + ds, dex.len(), "the data runs to the end");
        assert_eq!(ds % 4, 0, "data_size");
        let string = |i: usize| {
            let mut o = u4(so + 4 * i) as usize;
            let length = uleb(&mut o) as usize;
            let end = o + dex[o..].iter().position(|&b| b == 0).unwrap();
            let bytes = &dex[o..end];
            // (Modified UTF-8 of the string's own: re-encoded, the same)
            let s = decode_mutf8(bytes);
            assert_eq!(mutf8(&s), bytes);
            assert_eq!(s.encode_utf16().count(), length);
            s
        };
        let strings: Vec<String> = (0..ns).map(string).collect();
        assert!(
            strings
                .windows(2)
                .all(|w| utf16_key(&w[0]) < utf16_key(&w[1])),
            "strings sorted"
        );
        let types: Vec<usize> = (0..nt).map(|i| u4(to + 4 * i) as usize).collect();
        assert!(types.windows(2).all(|w| w[0] < w[1]), "types sorted");
        let ty = |i: usize| strings[types[i]].clone();
        let type_list = |o: usize| -> Vec<u16> {
            if o == 0 {
                return Vec::new();
            }
            assert_eq!(o % 4, 0, "type_list alignment");
            (0..u4(o) as usize).map(|k| u2(o + 4 + 2 * k)).collect()
        };
        let mut protos = Vec::new();
        for i in 0..np {
            let (shorty_idx, ret, params) =
                (u4(po + 12 * i), u4(po + 12 * i + 4), u4(po + 12 * i + 8));
            let params = type_list(params as usize);
            let names: Vec<String> = params.iter().map(|&p| ty(p as usize)).collect();
            assert_eq!(
                strings[shorty_idx as usize],
                shorty(&ty(ret as usize), &names)
            );
            protos.push((
                (ret, params),
                format!("({}){}", names.concat(), ty(ret as usize)),
            ));
        }
        assert!(protos.windows(2).all(|w| w[0].0 < w[1].0), "protos sorted");
        let fields: Vec<((u16, u32, u16), String, String)> = (0..nf)
            .map(|i| {
                let (class, t, name) = (u2(fo + 8 * i), u2(fo + 8 * i + 2), u4(fo + 8 * i + 4));
                (
                    (class, name, t),
                    strings[name as usize].clone(),
                    ty(t as usize),
                )
            })
            .collect();
        assert!(fields.windows(2).all(|w| w[0].0 < w[1].0), "fields sorted");
        let method_ids: Vec<((u16, u32, u16), String, String)> = (0..nm)
            .map(|i| {
                let (class, proto, name) = (u2(mo + 8 * i), u2(mo + 8 * i + 2), u4(mo + 8 * i + 4));
                (
                    (class, name, proto),
                    strings[name as usize].clone(),
                    protos[proto as usize].1.clone(),
                )
            })
            .collect();
        assert!(
            method_ids.windows(2).all(|w| w[0].0 < w[1].0),
            "methods sorted"
        );
        let methods: Vec<String> = method_ids
            .iter()
            .map(|((class, _, _), name, descriptor)| {
                format!("{}->{name}{descriptor}", ty(*class as usize))
            })
            .collect();
        let mut classes = Vec::new();
        for i in 0..nc {
            let base = co + 32 * i;
            let [class_idx, access, super_idx, interfaces_off, source_file, annotations, data_off, static_values] =
                std::array::from_fn(|k| u4(base + 4 * k));
            assert_eq!((source_file, annotations, static_values), (NO_INDEX, 0, 0));
            let mut class = ReadClass {
                name: ty(class_idx as usize),
                access,
                superclass: ty(super_idx as usize),
                interfaces: type_list(interfaces_off as usize)
                    .iter()
                    .map(|&t| ty(t as usize))
                    .collect(),
                fields: Vec::new(),
                methods: Vec::new(),
                code: BTreeMap::new(),
            };
            let mut o = data_off as usize;
            let counts = [uleb(&mut o), uleb(&mut o), uleb(&mut o), uleb(&mut o)];
            for count in &counts[..2] {
                let mut index = 0;
                for k in 0..*count {
                    let diff = uleb(&mut o);
                    assert!(k == 0 || diff > 0, "fields in order");
                    index += diff;
                    let f = &fields[index as usize];
                    assert_eq!(ty(f.0 .0 as usize), class.name);
                    class.fields.push((f.1.clone(), f.2.clone(), uleb(&mut o)));
                }
            }
            for (direct, count) in [(true, counts[2]), (false, counts[3])] {
                let mut index = 0;
                for k in 0..count {
                    let diff = uleb(&mut o);
                    assert!(k == 0 || diff > 0, "methods in order");
                    index += diff;
                    let (access, code_off) = (uleb(&mut o), uleb(&mut o) as usize);
                    let m = &method_ids[index as usize];
                    assert_eq!(ty(m.0 .0 as usize), class.name);
                    assert_eq!(
                        direct,
                        access & (STATIC | PRIVATE | CONSTRUCTOR) != 0,
                        "{}",
                        m.1
                    );
                    assert_eq!(
                        code_off == 0,
                        access & NATIVE != 0,
                        "{}: code only if not native",
                        m.1
                    );
                    class
                        .methods
                        .push((m.1.clone(), m.2.clone(), access, code_off != 0));
                    if code_off != 0 {
                        assert_eq!(code_off % 4, 0, "code_item alignment");
                        assert_eq!(
                            (u2(code_off + 6), u4(code_off + 8)),
                            (0, 0),
                            "no tries, no debug info"
                        );
                        let n = u4(code_off + 12) as usize;
                        class.code.insert(
                            format!("{}{}", m.1, m.2),
                            ReadCode {
                                registers: u2(code_off),
                                ins: u2(code_off + 2),
                                outs: u2(code_off + 4),
                                insns: (0..n).map(|k| u2(code_off + 16 + 2 * k)).collect(),
                            },
                        );
                    }
                }
            }
            // (a class after its superclass and interfaces, if they're here)
            for parent in std::iter::once(&class.superclass).chain(&class.interfaces) {
                let defined: Vec<&String> = classes.iter().map(|c: &ReadClass| &c.name).collect();
                let later = (i + 1..nc).any(|j| ty(u4(co + 32 * j) as usize) == *parent);
                assert!(
                    defined.contains(&parent) || !later,
                    "{} before {parent}",
                    class.name
                );
            }
            classes.push(class);
        }
        // The map: every section, in offset order, the header first and
        // the map itself last.
        let map_off = u4(52) as usize;
        assert_eq!(map_off % 4, 0);
        let entries: Vec<(u16, u32, u32)> = (0..u4(map_off) as usize)
            .map(|k| {
                let o = map_off + 4 + 12 * k;
                (u2(o), u4(o + 4), u4(o + 8))
            })
            .collect();
        assert!(
            entries.windows(2).all(|w| w[0].2 < w[1].2),
            "map in offset order"
        );
        assert_eq!(entries[0], (TYPE_HEADER_ITEM, 1, 0));
        assert_eq!(*entries.last().unwrap(), (TYPE_MAP_LIST, 1, map_off as u32));
        assert_eq!(
            map_off + 4 + 12 * entries.len(),
            dex.len(),
            "the map ends the file"
        );
        for (kind, count, offset) in [
            (TYPE_STRING_ID_ITEM, ns, so),
            (TYPE_TYPE_ID_ITEM, nt, to),
            (TYPE_PROTO_ID_ITEM, np, po),
            (TYPE_FIELD_ID_ITEM, nf, fo),
            (TYPE_METHOD_ID_ITEM, nm, mo),
            (TYPE_CLASS_DEF_ITEM, nc, co),
        ] {
            if count > 0 {
                assert!(
                    entries.contains(&(kind, count as u32, offset as u32)),
                    "map entry {kind:#x}"
                );
            } else {
                assert_eq!(offset, 0);
            }
        }
        File {
            strings,
            methods,
            classes,
        }
    }

    fn decode_mutf8(bytes: &[u8]) -> String {
        let mut units = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i] as u16;
            if b < 0x80 {
                assert_ne!(b, 0);
                units.push(b);
                i += 1;
            } else if b & 0xe0 == 0xc0 {
                units.push(((b & 0x1f) << 6) | (bytes[i + 1] as u16 & 0x3f));
                i += 2;
            } else {
                units.push(
                    ((b & 0x0f) << 12)
                        | ((bytes[i + 1] as u16 & 0x3f) << 6)
                        | (bytes[i + 2] as u16 & 0x3f),
                );
                i += 3;
            }
        }
        String::from_utf16(&units).unwrap()
    }
}
