// The Java Native Interface (JNI) ABI, declared by hand for the Android
// platform layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What upstream includes as `<jni.h>` (the NDK's, from libnativehelper):
//! the JNI types, constants and the two function tables, `JNIEnv`'s
//! (`struct JNINativeInterface`, 229 functions after 4 reserved slots) and
//! `JavaVM`'s (`struct JNIInvokeInterface`, 5 after 3), with every slot at
//! jni.h's index (checked at compile time against the offsets, and by a
//! host test against the NDK's jni.h, when there's one). These are the
//! headers' layout facts (the NDK's headers are under the Apache License
//! 2.0; see docs/LICENSING.md), not their code. [`super::env`] wraps them.
//!
//! The variadic functions (`CallVoidMethod(env, obj, mid, ...)`) and their
//! `va_list` forms (`CallVoidMethodV`) are declared as untyped slots: Rust
//! calls the `jvalue` array forms (`CallVoidMethodA`), which take the same
//! arguments.
//!
//! In C, `JNIEnv` is a pointer to the function table, and a native method
//! gets a `JNIEnv *`; the same here (`*mut JNIEnv`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::upper_case_acronyms
)]

use std::ffi::{c_char, c_void};

/// `jboolean`: unsigned 8 bits.
pub(crate) type jboolean = u8;
/// `jbyte`: signed 8 bits.
pub(crate) type jbyte = i8;
/// `jchar`: unsigned 16 bits (a UTF-16 code unit).
pub(crate) type jchar = u16;
/// `jshort`: signed 16 bits.
pub(crate) type jshort = i16;
/// `jint`: signed 32 bits.
pub(crate) type jint = i32;
/// `jlong`: signed 64 bits.
pub(crate) type jlong = i64;
/// `jfloat`: 32-bit IEEE 754.
pub(crate) type jfloat = f32;
/// `jdouble`: 64-bit IEEE 754.
pub(crate) type jdouble = f64;
/// `jsize`: a `jint`.
pub(crate) type jsize = jint;

/// What an object reference points to (`struct _jobject`).
#[repr(C)]
pub(crate) struct _jobject {
    _private: [u8; 0],
}

/// `jobject`, and in C's typedefs (which only C++ tells apart) `jclass`,
/// `jstring`, `jarray`, `jobjectArray`, the primitive arrays, `jthrowable`
/// and `jweak`.
pub(crate) type jobject = *mut _jobject;
pub(crate) type jclass = jobject;
pub(crate) type jstring = jobject;
pub(crate) type jarray = jobject;
pub(crate) type jobjectArray = jarray;
pub(crate) type jbooleanArray = jarray;
pub(crate) type jbyteArray = jarray;
pub(crate) type jcharArray = jarray;
pub(crate) type jshortArray = jarray;
pub(crate) type jintArray = jarray;
pub(crate) type jlongArray = jarray;
pub(crate) type jfloatArray = jarray;
pub(crate) type jdoubleArray = jarray;
pub(crate) type jthrowable = jobject;
pub(crate) type jweak = jobject;

/// `struct _jfieldID`.
#[repr(C)]
pub(crate) struct _jfieldID {
    _private: [u8; 0],
}
/// `jfieldID`.
pub(crate) type jfieldID = *mut _jfieldID;

/// `struct _jmethodID`.
#[repr(C)]
pub(crate) struct _jmethodID {
    _private: [u8; 0],
}
/// `jmethodID`.
pub(crate) type jmethodID = *mut _jmethodID;

/// `jvalue`: an argument of the `Call*MethodA` functions.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) union jvalue {
    pub(crate) z: jboolean,
    pub(crate) b: jbyte,
    pub(crate) c: jchar,
    pub(crate) s: jshort,
    pub(crate) i: jint,
    pub(crate) j: jlong,
    pub(crate) f: jfloat,
    pub(crate) d: jdouble,
    pub(crate) l: jobject,
}

/// `jobjectRefType`.
pub(crate) type jobjectRefType = u32;
pub(crate) const JNIInvalidRefType: jobjectRefType = 0;
pub(crate) const JNILocalRefType: jobjectRefType = 1;
pub(crate) const JNIGlobalRefType: jobjectRefType = 2;
pub(crate) const JNIWeakGlobalRefType: jobjectRefType = 3;

/// `JNINativeMethod`: an entry of `RegisterNatives()`'s table.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct JNINativeMethod {
    pub(crate) name: *const c_char,
    pub(crate) signature: *const c_char,
    pub(crate) fnPtr: *mut c_void,
}

/// `JavaVMAttachArgs`, `AttachCurrentThread()`'s optional arguments.
#[repr(C)]
#[derive(Debug)]
pub(crate) struct JavaVMAttachArgs {
    pub(crate) version: jint,
    pub(crate) name: *const c_char,
    pub(crate) group: jobject,
}

pub(crate) const JNI_FALSE: jboolean = 0;
pub(crate) const JNI_TRUE: jboolean = 1;

pub(crate) const JNI_VERSION_1_1: jint = 0x0001_0001;
pub(crate) const JNI_VERSION_1_2: jint = 0x0001_0002;
pub(crate) const JNI_VERSION_1_4: jint = 0x0001_0004;
pub(crate) const JNI_VERSION_1_6: jint = 0x0001_0006;

/// Success.
pub(crate) const JNI_OK: jint = 0;
/// An error.
pub(crate) const JNI_ERR: jint = -1;
/// The thread isn't attached to the VM.
pub(crate) const JNI_EDETACHED: jint = -2;
/// The JNI version isn't supported.
pub(crate) const JNI_EVERSION: jint = -3;
/// Not enough memory.
pub(crate) const JNI_ENOMEM: jint = -4;
/// A VM exists already.
pub(crate) const JNI_EEXIST: jint = -5;
/// An argument isn't valid.
pub(crate) const JNI_EINVAL: jint = -6;

/// `Release*ArrayElements()`: copy the elements back, keep the buffer.
pub(crate) const JNI_COMMIT: jint = 1;
/// `Release*ArrayElements()`: free the buffer without copying back.
pub(crate) const JNI_ABORT: jint = 2;

/// `JNIEnv` (C's: `const struct JNINativeInterface *`).
pub(crate) type JNIEnv = *const JNINativeInterface;
/// `JavaVM` (C's: `const struct JNIInvokeInterface *`).
pub(crate) type JavaVM = *const JNIInvokeInterface;

/// An untyped slot: a variadic function, or one taking a `va_list`.
pub(crate) type Untyped = *const c_void;

type E = *mut JNIEnv;

/// Declares a function table as jni.h does (one function pointer per slot,
/// in order), with each slot's index (checked against its offset at
/// compile time) and the slots' names in order, for the host test that
/// reads them from jni.h.
macro_rules! function_table {
    (
        $(#[$meta:meta])*
        struct $name:ident, names $names:ident {
            $($index:literal $field:ident: $ty:ty,)*
        }
    ) => {
        $(#[$meta])*
        #[repr(C)]
        pub(crate) struct $name {
            $(pub(crate) $field: $ty,)*
        }

        /// The slots' names, in order (jni.h's member names).
        #[cfg_attr(not(test), allow(dead_code))]
        pub(crate) const $names: &[&str] = &[$(stringify!($field)),*];

        const _: () = {
            $(assert!(
                std::mem::offset_of!($name, $field) == $index * std::mem::size_of::<*const c_void>(),
                concat!("the slot of ", stringify!($field), " isn't jni.h's")
            );)*
            assert!(std::mem::size_of::<$name>() == $names.len() * std::mem::size_of::<*const c_void>());
        };
    };
}

function_table! {
    /// `struct JNINativeInterface`: what a `JNIEnv` points to.
    struct JNINativeInterface, names NATIVE_INTERFACE_SLOTS {
        0 reserved0: Untyped,
        1 reserved1: Untyped,
        2 reserved2: Untyped,
        3 reserved3: Untyped,

        4 GetVersion: unsafe extern "system" fn(E) -> jint,

        5 DefineClass: unsafe extern "system" fn(E, *const c_char, jobject, *const jbyte, jsize) -> jclass,
        6 FindClass: unsafe extern "system" fn(E, *const c_char) -> jclass,

        7 FromReflectedMethod: unsafe extern "system" fn(E, jobject) -> jmethodID,
        8 FromReflectedField: unsafe extern "system" fn(E, jobject) -> jfieldID,
        9 ToReflectedMethod: unsafe extern "system" fn(E, jclass, jmethodID, jboolean) -> jobject,

        10 GetSuperclass: unsafe extern "system" fn(E, jclass) -> jclass,
        11 IsAssignableFrom: unsafe extern "system" fn(E, jclass, jclass) -> jboolean,

        12 ToReflectedField: unsafe extern "system" fn(E, jclass, jfieldID, jboolean) -> jobject,

        13 Throw: unsafe extern "system" fn(E, jthrowable) -> jint,
        14 ThrowNew: unsafe extern "system" fn(E, jclass, *const c_char) -> jint,
        15 ExceptionOccurred: unsafe extern "system" fn(E) -> jthrowable,
        16 ExceptionDescribe: unsafe extern "system" fn(E),
        17 ExceptionClear: unsafe extern "system" fn(E),
        18 FatalError: unsafe extern "system" fn(E, *const c_char),

        19 PushLocalFrame: unsafe extern "system" fn(E, jint) -> jint,
        20 PopLocalFrame: unsafe extern "system" fn(E, jobject) -> jobject,

        21 NewGlobalRef: unsafe extern "system" fn(E, jobject) -> jobject,
        22 DeleteGlobalRef: unsafe extern "system" fn(E, jobject),
        23 DeleteLocalRef: unsafe extern "system" fn(E, jobject),
        24 IsSameObject: unsafe extern "system" fn(E, jobject, jobject) -> jboolean,

        25 NewLocalRef: unsafe extern "system" fn(E, jobject) -> jobject,
        26 EnsureLocalCapacity: unsafe extern "system" fn(E, jint) -> jint,

        27 AllocObject: unsafe extern "system" fn(E, jclass) -> jobject,
        28 NewObject: Untyped,
        29 NewObjectV: Untyped,
        30 NewObjectA: unsafe extern "system" fn(E, jclass, jmethodID, *const jvalue) -> jobject,

        31 GetObjectClass: unsafe extern "system" fn(E, jobject) -> jclass,
        32 IsInstanceOf: unsafe extern "system" fn(E, jobject, jclass) -> jboolean,
        33 GetMethodID: unsafe extern "system" fn(E, jclass, *const c_char, *const c_char) -> jmethodID,

        34 CallObjectMethod: Untyped,
        35 CallObjectMethodV: Untyped,
        36 CallObjectMethodA: unsafe extern "system" fn(E, jobject, jmethodID, *const jvalue) -> jobject,
        37 CallBooleanMethod: Untyped,
        38 CallBooleanMethodV: Untyped,
        39 CallBooleanMethodA: unsafe extern "system" fn(E, jobject, jmethodID, *const jvalue) -> jboolean,
        40 CallByteMethod: Untyped,
        41 CallByteMethodV: Untyped,
        42 CallByteMethodA: unsafe extern "system" fn(E, jobject, jmethodID, *const jvalue) -> jbyte,
        43 CallCharMethod: Untyped,
        44 CallCharMethodV: Untyped,
        45 CallCharMethodA: unsafe extern "system" fn(E, jobject, jmethodID, *const jvalue) -> jchar,
        46 CallShortMethod: Untyped,
        47 CallShortMethodV: Untyped,
        48 CallShortMethodA: unsafe extern "system" fn(E, jobject, jmethodID, *const jvalue) -> jshort,
        49 CallIntMethod: Untyped,
        50 CallIntMethodV: Untyped,
        51 CallIntMethodA: unsafe extern "system" fn(E, jobject, jmethodID, *const jvalue) -> jint,
        52 CallLongMethod: Untyped,
        53 CallLongMethodV: Untyped,
        54 CallLongMethodA: unsafe extern "system" fn(E, jobject, jmethodID, *const jvalue) -> jlong,
        55 CallFloatMethod: Untyped,
        56 CallFloatMethodV: Untyped,
        57 CallFloatMethodA: unsafe extern "system" fn(E, jobject, jmethodID, *const jvalue) -> jfloat,
        58 CallDoubleMethod: Untyped,
        59 CallDoubleMethodV: Untyped,
        60 CallDoubleMethodA: unsafe extern "system" fn(E, jobject, jmethodID, *const jvalue) -> jdouble,
        61 CallVoidMethod: Untyped,
        62 CallVoidMethodV: Untyped,
        63 CallVoidMethodA: unsafe extern "system" fn(E, jobject, jmethodID, *const jvalue),

        64 CallNonvirtualObjectMethod: Untyped,
        65 CallNonvirtualObjectMethodV: Untyped,
        66 CallNonvirtualObjectMethodA: unsafe extern "system" fn(E, jobject, jclass, jmethodID, *const jvalue) -> jobject,
        67 CallNonvirtualBooleanMethod: Untyped,
        68 CallNonvirtualBooleanMethodV: Untyped,
        69 CallNonvirtualBooleanMethodA: unsafe extern "system" fn(E, jobject, jclass, jmethodID, *const jvalue) -> jboolean,
        70 CallNonvirtualByteMethod: Untyped,
        71 CallNonvirtualByteMethodV: Untyped,
        72 CallNonvirtualByteMethodA: unsafe extern "system" fn(E, jobject, jclass, jmethodID, *const jvalue) -> jbyte,
        73 CallNonvirtualCharMethod: Untyped,
        74 CallNonvirtualCharMethodV: Untyped,
        75 CallNonvirtualCharMethodA: unsafe extern "system" fn(E, jobject, jclass, jmethodID, *const jvalue) -> jchar,
        76 CallNonvirtualShortMethod: Untyped,
        77 CallNonvirtualShortMethodV: Untyped,
        78 CallNonvirtualShortMethodA: unsafe extern "system" fn(E, jobject, jclass, jmethodID, *const jvalue) -> jshort,
        79 CallNonvirtualIntMethod: Untyped,
        80 CallNonvirtualIntMethodV: Untyped,
        81 CallNonvirtualIntMethodA: unsafe extern "system" fn(E, jobject, jclass, jmethodID, *const jvalue) -> jint,
        82 CallNonvirtualLongMethod: Untyped,
        83 CallNonvirtualLongMethodV: Untyped,
        84 CallNonvirtualLongMethodA: unsafe extern "system" fn(E, jobject, jclass, jmethodID, *const jvalue) -> jlong,
        85 CallNonvirtualFloatMethod: Untyped,
        86 CallNonvirtualFloatMethodV: Untyped,
        87 CallNonvirtualFloatMethodA: unsafe extern "system" fn(E, jobject, jclass, jmethodID, *const jvalue) -> jfloat,
        88 CallNonvirtualDoubleMethod: Untyped,
        89 CallNonvirtualDoubleMethodV: Untyped,
        90 CallNonvirtualDoubleMethodA: unsafe extern "system" fn(E, jobject, jclass, jmethodID, *const jvalue) -> jdouble,
        91 CallNonvirtualVoidMethod: Untyped,
        92 CallNonvirtualVoidMethodV: Untyped,
        93 CallNonvirtualVoidMethodA: unsafe extern "system" fn(E, jobject, jclass, jmethodID, *const jvalue),

        94 GetFieldID: unsafe extern "system" fn(E, jclass, *const c_char, *const c_char) -> jfieldID,

        95 GetObjectField: unsafe extern "system" fn(E, jobject, jfieldID) -> jobject,
        96 GetBooleanField: unsafe extern "system" fn(E, jobject, jfieldID) -> jboolean,
        97 GetByteField: unsafe extern "system" fn(E, jobject, jfieldID) -> jbyte,
        98 GetCharField: unsafe extern "system" fn(E, jobject, jfieldID) -> jchar,
        99 GetShortField: unsafe extern "system" fn(E, jobject, jfieldID) -> jshort,
        100 GetIntField: unsafe extern "system" fn(E, jobject, jfieldID) -> jint,
        101 GetLongField: unsafe extern "system" fn(E, jobject, jfieldID) -> jlong,
        102 GetFloatField: unsafe extern "system" fn(E, jobject, jfieldID) -> jfloat,
        103 GetDoubleField: unsafe extern "system" fn(E, jobject, jfieldID) -> jdouble,

        104 SetObjectField: unsafe extern "system" fn(E, jobject, jfieldID, jobject),
        105 SetBooleanField: unsafe extern "system" fn(E, jobject, jfieldID, jboolean),
        106 SetByteField: unsafe extern "system" fn(E, jobject, jfieldID, jbyte),
        107 SetCharField: unsafe extern "system" fn(E, jobject, jfieldID, jchar),
        108 SetShortField: unsafe extern "system" fn(E, jobject, jfieldID, jshort),
        109 SetIntField: unsafe extern "system" fn(E, jobject, jfieldID, jint),
        110 SetLongField: unsafe extern "system" fn(E, jobject, jfieldID, jlong),
        111 SetFloatField: unsafe extern "system" fn(E, jobject, jfieldID, jfloat),
        112 SetDoubleField: unsafe extern "system" fn(E, jobject, jfieldID, jdouble),

        113 GetStaticMethodID: unsafe extern "system" fn(E, jclass, *const c_char, *const c_char) -> jmethodID,

        114 CallStaticObjectMethod: Untyped,
        115 CallStaticObjectMethodV: Untyped,
        116 CallStaticObjectMethodA: unsafe extern "system" fn(E, jclass, jmethodID, *const jvalue) -> jobject,
        117 CallStaticBooleanMethod: Untyped,
        118 CallStaticBooleanMethodV: Untyped,
        119 CallStaticBooleanMethodA: unsafe extern "system" fn(E, jclass, jmethodID, *const jvalue) -> jboolean,
        120 CallStaticByteMethod: Untyped,
        121 CallStaticByteMethodV: Untyped,
        122 CallStaticByteMethodA: unsafe extern "system" fn(E, jclass, jmethodID, *const jvalue) -> jbyte,
        123 CallStaticCharMethod: Untyped,
        124 CallStaticCharMethodV: Untyped,
        125 CallStaticCharMethodA: unsafe extern "system" fn(E, jclass, jmethodID, *const jvalue) -> jchar,
        126 CallStaticShortMethod: Untyped,
        127 CallStaticShortMethodV: Untyped,
        128 CallStaticShortMethodA: unsafe extern "system" fn(E, jclass, jmethodID, *const jvalue) -> jshort,
        129 CallStaticIntMethod: Untyped,
        130 CallStaticIntMethodV: Untyped,
        131 CallStaticIntMethodA: unsafe extern "system" fn(E, jclass, jmethodID, *const jvalue) -> jint,
        132 CallStaticLongMethod: Untyped,
        133 CallStaticLongMethodV: Untyped,
        134 CallStaticLongMethodA: unsafe extern "system" fn(E, jclass, jmethodID, *const jvalue) -> jlong,
        135 CallStaticFloatMethod: Untyped,
        136 CallStaticFloatMethodV: Untyped,
        137 CallStaticFloatMethodA: unsafe extern "system" fn(E, jclass, jmethodID, *const jvalue) -> jfloat,
        138 CallStaticDoubleMethod: Untyped,
        139 CallStaticDoubleMethodV: Untyped,
        140 CallStaticDoubleMethodA: unsafe extern "system" fn(E, jclass, jmethodID, *const jvalue) -> jdouble,
        141 CallStaticVoidMethod: Untyped,
        142 CallStaticVoidMethodV: Untyped,
        143 CallStaticVoidMethodA: unsafe extern "system" fn(E, jclass, jmethodID, *const jvalue),

        144 GetStaticFieldID: unsafe extern "system" fn(E, jclass, *const c_char, *const c_char) -> jfieldID,

        145 GetStaticObjectField: unsafe extern "system" fn(E, jclass, jfieldID) -> jobject,
        146 GetStaticBooleanField: unsafe extern "system" fn(E, jclass, jfieldID) -> jboolean,
        147 GetStaticByteField: unsafe extern "system" fn(E, jclass, jfieldID) -> jbyte,
        148 GetStaticCharField: unsafe extern "system" fn(E, jclass, jfieldID) -> jchar,
        149 GetStaticShortField: unsafe extern "system" fn(E, jclass, jfieldID) -> jshort,
        150 GetStaticIntField: unsafe extern "system" fn(E, jclass, jfieldID) -> jint,
        151 GetStaticLongField: unsafe extern "system" fn(E, jclass, jfieldID) -> jlong,
        152 GetStaticFloatField: unsafe extern "system" fn(E, jclass, jfieldID) -> jfloat,
        153 GetStaticDoubleField: unsafe extern "system" fn(E, jclass, jfieldID) -> jdouble,

        154 SetStaticObjectField: unsafe extern "system" fn(E, jclass, jfieldID, jobject),
        155 SetStaticBooleanField: unsafe extern "system" fn(E, jclass, jfieldID, jboolean),
        156 SetStaticByteField: unsafe extern "system" fn(E, jclass, jfieldID, jbyte),
        157 SetStaticCharField: unsafe extern "system" fn(E, jclass, jfieldID, jchar),
        158 SetStaticShortField: unsafe extern "system" fn(E, jclass, jfieldID, jshort),
        159 SetStaticIntField: unsafe extern "system" fn(E, jclass, jfieldID, jint),
        160 SetStaticLongField: unsafe extern "system" fn(E, jclass, jfieldID, jlong),
        161 SetStaticFloatField: unsafe extern "system" fn(E, jclass, jfieldID, jfloat),
        162 SetStaticDoubleField: unsafe extern "system" fn(E, jclass, jfieldID, jdouble),

        163 NewString: unsafe extern "system" fn(E, *const jchar, jsize) -> jstring,
        164 GetStringLength: unsafe extern "system" fn(E, jstring) -> jsize,
        165 GetStringChars: unsafe extern "system" fn(E, jstring, *mut jboolean) -> *const jchar,
        166 ReleaseStringChars: unsafe extern "system" fn(E, jstring, *const jchar),
        167 NewStringUTF: unsafe extern "system" fn(E, *const c_char) -> jstring,
        168 GetStringUTFLength: unsafe extern "system" fn(E, jstring) -> jsize,
        169 GetStringUTFChars: unsafe extern "system" fn(E, jstring, *mut jboolean) -> *const c_char,
        170 ReleaseStringUTFChars: unsafe extern "system" fn(E, jstring, *const c_char),
        171 GetArrayLength: unsafe extern "system" fn(E, jarray) -> jsize,
        172 NewObjectArray: unsafe extern "system" fn(E, jsize, jclass, jobject) -> jobjectArray,
        173 GetObjectArrayElement: unsafe extern "system" fn(E, jobjectArray, jsize) -> jobject,
        174 SetObjectArrayElement: unsafe extern "system" fn(E, jobjectArray, jsize, jobject),

        175 NewBooleanArray: unsafe extern "system" fn(E, jsize) -> jbooleanArray,
        176 NewByteArray: unsafe extern "system" fn(E, jsize) -> jbyteArray,
        177 NewCharArray: unsafe extern "system" fn(E, jsize) -> jcharArray,
        178 NewShortArray: unsafe extern "system" fn(E, jsize) -> jshortArray,
        179 NewIntArray: unsafe extern "system" fn(E, jsize) -> jintArray,
        180 NewLongArray: unsafe extern "system" fn(E, jsize) -> jlongArray,
        181 NewFloatArray: unsafe extern "system" fn(E, jsize) -> jfloatArray,
        182 NewDoubleArray: unsafe extern "system" fn(E, jsize) -> jdoubleArray,

        183 GetBooleanArrayElements: unsafe extern "system" fn(E, jbooleanArray, *mut jboolean) -> *mut jboolean,
        184 GetByteArrayElements: unsafe extern "system" fn(E, jbyteArray, *mut jboolean) -> *mut jbyte,
        185 GetCharArrayElements: unsafe extern "system" fn(E, jcharArray, *mut jboolean) -> *mut jchar,
        186 GetShortArrayElements: unsafe extern "system" fn(E, jshortArray, *mut jboolean) -> *mut jshort,
        187 GetIntArrayElements: unsafe extern "system" fn(E, jintArray, *mut jboolean) -> *mut jint,
        188 GetLongArrayElements: unsafe extern "system" fn(E, jlongArray, *mut jboolean) -> *mut jlong,
        189 GetFloatArrayElements: unsafe extern "system" fn(E, jfloatArray, *mut jboolean) -> *mut jfloat,
        190 GetDoubleArrayElements: unsafe extern "system" fn(E, jdoubleArray, *mut jboolean) -> *mut jdouble,

        191 ReleaseBooleanArrayElements: unsafe extern "system" fn(E, jbooleanArray, *mut jboolean, jint),
        192 ReleaseByteArrayElements: unsafe extern "system" fn(E, jbyteArray, *mut jbyte, jint),
        193 ReleaseCharArrayElements: unsafe extern "system" fn(E, jcharArray, *mut jchar, jint),
        194 ReleaseShortArrayElements: unsafe extern "system" fn(E, jshortArray, *mut jshort, jint),
        195 ReleaseIntArrayElements: unsafe extern "system" fn(E, jintArray, *mut jint, jint),
        196 ReleaseLongArrayElements: unsafe extern "system" fn(E, jlongArray, *mut jlong, jint),
        197 ReleaseFloatArrayElements: unsafe extern "system" fn(E, jfloatArray, *mut jfloat, jint),
        198 ReleaseDoubleArrayElements: unsafe extern "system" fn(E, jdoubleArray, *mut jdouble, jint),

        199 GetBooleanArrayRegion: unsafe extern "system" fn(E, jbooleanArray, jsize, jsize, *mut jboolean),
        200 GetByteArrayRegion: unsafe extern "system" fn(E, jbyteArray, jsize, jsize, *mut jbyte),
        201 GetCharArrayRegion: unsafe extern "system" fn(E, jcharArray, jsize, jsize, *mut jchar),
        202 GetShortArrayRegion: unsafe extern "system" fn(E, jshortArray, jsize, jsize, *mut jshort),
        203 GetIntArrayRegion: unsafe extern "system" fn(E, jintArray, jsize, jsize, *mut jint),
        204 GetLongArrayRegion: unsafe extern "system" fn(E, jlongArray, jsize, jsize, *mut jlong),
        205 GetFloatArrayRegion: unsafe extern "system" fn(E, jfloatArray, jsize, jsize, *mut jfloat),
        206 GetDoubleArrayRegion: unsafe extern "system" fn(E, jdoubleArray, jsize, jsize, *mut jdouble),

        207 SetBooleanArrayRegion: unsafe extern "system" fn(E, jbooleanArray, jsize, jsize, *const jboolean),
        208 SetByteArrayRegion: unsafe extern "system" fn(E, jbyteArray, jsize, jsize, *const jbyte),
        209 SetCharArrayRegion: unsafe extern "system" fn(E, jcharArray, jsize, jsize, *const jchar),
        210 SetShortArrayRegion: unsafe extern "system" fn(E, jshortArray, jsize, jsize, *const jshort),
        211 SetIntArrayRegion: unsafe extern "system" fn(E, jintArray, jsize, jsize, *const jint),
        212 SetLongArrayRegion: unsafe extern "system" fn(E, jlongArray, jsize, jsize, *const jlong),
        213 SetFloatArrayRegion: unsafe extern "system" fn(E, jfloatArray, jsize, jsize, *const jfloat),
        214 SetDoubleArrayRegion: unsafe extern "system" fn(E, jdoubleArray, jsize, jsize, *const jdouble),

        215 RegisterNatives: unsafe extern "system" fn(E, jclass, *const JNINativeMethod, jint) -> jint,
        216 UnregisterNatives: unsafe extern "system" fn(E, jclass) -> jint,
        217 MonitorEnter: unsafe extern "system" fn(E, jobject) -> jint,
        218 MonitorExit: unsafe extern "system" fn(E, jobject) -> jint,
        219 GetJavaVM: unsafe extern "system" fn(E, *mut *mut JavaVM) -> jint,

        220 GetStringRegion: unsafe extern "system" fn(E, jstring, jsize, jsize, *mut jchar),
        221 GetStringUTFRegion: unsafe extern "system" fn(E, jstring, jsize, jsize, *mut c_char),

        222 GetPrimitiveArrayCritical: unsafe extern "system" fn(E, jarray, *mut jboolean) -> *mut c_void,
        223 ReleasePrimitiveArrayCritical: unsafe extern "system" fn(E, jarray, *mut c_void, jint),

        224 GetStringCritical: unsafe extern "system" fn(E, jstring, *mut jboolean) -> *const jchar,
        225 ReleaseStringCritical: unsafe extern "system" fn(E, jstring, *const jchar),

        226 NewWeakGlobalRef: unsafe extern "system" fn(E, jobject) -> jweak,
        227 DeleteWeakGlobalRef: unsafe extern "system" fn(E, jweak),

        228 ExceptionCheck: unsafe extern "system" fn(E) -> jboolean,

        229 NewDirectByteBuffer: unsafe extern "system" fn(E, *mut c_void, jlong) -> jobject,
        230 GetDirectBufferAddress: unsafe extern "system" fn(E, jobject) -> *mut c_void,
        231 GetDirectBufferCapacity: unsafe extern "system" fn(E, jobject) -> jlong,

        232 GetObjectRefType: unsafe extern "system" fn(E, jobject) -> jobjectRefType,
    }
}

function_table! {
    /// `struct JNIInvokeInterface`: what a `JavaVM` points to.
    struct JNIInvokeInterface, names INVOKE_INTERFACE_SLOTS {
        0 reserved0: Untyped,
        1 reserved1: Untyped,
        2 reserved2: Untyped,

        3 DestroyJavaVM: unsafe extern "system" fn(*mut JavaVM) -> jint,
        4 AttachCurrentThread: unsafe extern "system" fn(*mut JavaVM, *mut *mut JNIEnv, *mut c_void) -> jint,
        5 DetachCurrentThread: unsafe extern "system" fn(*mut JavaVM) -> jint,
        6 GetEnv: unsafe extern "system" fn(*mut JavaVM, *mut *mut c_void, jint) -> jint,
        7 AttachCurrentThreadAsDaemon: unsafe extern "system" fn(*mut JavaVM, *mut *mut JNIEnv, *mut c_void) -> jint,
    }
}

/// A JNI type's character in a method's "shorty" (its return type's and
/// parameters', `L` for every reference type), for the compile-time check
/// that a native method's Rust function has the parameters its Java
/// declaration says ([`native_shorty`]).
pub(crate) trait JniType {
    const SHORTY: u8;
}

impl JniType for () {
    const SHORTY: u8 = b'V';
}
impl JniType for jboolean {
    const SHORTY: u8 = b'Z';
}
impl JniType for jbyte {
    const SHORTY: u8 = b'B';
}
impl JniType for jchar {
    const SHORTY: u8 = b'C';
}
impl JniType for jshort {
    const SHORTY: u8 = b'S';
}
impl JniType for jint {
    const SHORTY: u8 = b'I';
}
impl JniType for jlong {
    const SHORTY: u8 = b'J';
}
impl JniType for jfloat {
    const SHORTY: u8 = b'F';
}
impl JniType for jdouble {
    const SHORTY: u8 = b'D';
}
impl JniType for jobject {
    const SHORTY: u8 = b'L';
}

/// A native method's Rust function: `extern "system" fn(JNIEnv *, jobject
/// or jclass, the Java parameters...) -> the Java return type`.
pub(crate) trait NativeFunction: Copy {
    /// The shorty of the Java method it implements (return type first).
    const SHORTY: &'static [u8];
    /// The function as `JNINativeMethod::fnPtr`.
    fn pointer(self) -> *mut c_void;
}

macro_rules! native_function {
    ($($arg:ident)*) => {
        impl<R: JniType, $($arg: JniType),*> NativeFunction for extern "system" fn(*mut JNIEnv, jobject, $($arg),*) -> R {
            const SHORTY: &'static [u8] = &[R::SHORTY, $($arg::SHORTY),*];
            fn pointer(self) -> *mut c_void {
                self as *mut c_void
            }
        }
    };
}

native_function!();
native_function!(A);
native_function!(A B);
native_function!(A B C);
native_function!(A B C D);
native_function!(A B C D F);
native_function!(A B C D F G);
native_function!(A B C D F G H);
native_function!(A B C D F G H I);
native_function!(A B C D F G H I J);
native_function!(A B C D F G H I J K);
native_function!(A B C D F G H I J K L);
native_function!(A B C D F G H I J K L M);
native_function!(A B C D F G H I J K L M N);
native_function!(A B C D F G H I J K L M N O);

/// The shorty of a method descriptor (`(Ljava/lang/String;I)V`: `VLI`),
/// in `out`, and its length. Panics (at compile time, where it's used) on
/// a malformed descriptor.
pub(crate) const fn descriptor_shorty(descriptor: &str, out: &mut [u8; 32]) -> usize {
    let d = descriptor.as_bytes();
    assert!(
        !d.is_empty() && d[0] == b'(',
        "a method descriptor starts with ("
    );
    let mut i = 1;
    let mut n = 1; // (out[0] is the return type's)
    while i < d.len() && d[i] != b')' {
        let (c, next) = descriptor_type(d, i);
        out[n] = c;
        n += 1;
        i = next;
    }
    assert!(i < d.len(), "a method descriptor has a )");
    let (c, next) = descriptor_type(d, i + 1);
    assert!(
        next == d.len(),
        "a method descriptor ends after its return type"
    );
    out[0] = c;
    n
}

/// The shorty character of the field type at `d[i]`, and where it ends.
const fn descriptor_type(d: &[u8], mut i: usize) -> (u8, usize) {
    assert!(i < d.len(), "a type is missing");
    match d[i] {
        b'V' | b'Z' | b'B' | b'C' | b'S' | b'I' | b'J' | b'F' | b'D' => (d[i], i + 1),
        b'L' => {
            while i < d.len() && d[i] != b';' {
                i += 1;
            }
            assert!(i < d.len(), "a class type ends with ;");
            (b'L', i + 1)
        }
        b'[' => {
            while i < d.len() && d[i] == b'[' {
                i += 1;
            }
            let (_, next) = descriptor_type(d, i);
            (b'L', next)
        }
        _ => panic!("not a type descriptor"),
    }
}

/// The shorty of the Java method `f` implements (by its parameters).
pub(crate) const fn native_shorty_of<F: NativeFunction>(_f: &F) -> &'static [u8] {
    F::SHORTY
}

/// Whether `shorty` (a [`NativeFunction::SHORTY`]) is the shorty of
/// `descriptor`: the compile-time check of [`super::classes`]'s `native!`.
pub(crate) const fn native_shorty(descriptor: &str, shorty: &[u8]) -> bool {
    let mut buf = [0u8; 32];
    let n = descriptor_shorty(descriptor, &mut buf);
    if n != shorty.len() {
        return false;
    }
    let mut i = 0;
    while i < n {
        if buf[i] != shorty[i] {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorties() {
        let mut buf = [0u8; 32];
        let n = descriptor_shorty("(Ljava/lang/String;I[[JZ)V", &mut buf);
        assert_eq!(&buf[..n], b"VLILZ");
        let n = descriptor_shorty("()Ljava/lang/String;", &mut buf);
        assert_eq!(&buf[..n], b"L");
        let n = descriptor_shorty("([Ljava/lang/String;ZILjava/lang/String;I)Z", &mut buf);
        assert_eq!(&buf[..n], b"ZLZILI");
        assert!(native_shorty("(IIJFFF)V", b"VIIJFFF"));
        assert!(!native_shorty("(IIJFFF)V", b"VIIIFFF"));
        assert!(!native_shorty("(I)V", b"V"));
        assert_eq!(
            <extern "system" fn(*mut JNIEnv, jobject, jint, jobject) -> jboolean as NativeFunction>::SHORTY,
            b"ZIL"
        );
    }

    #[test]
    fn tables() {
        assert_eq!(NATIVE_INTERFACE_SLOTS.len(), 233);
        assert_eq!(INVOKE_INTERFACE_SLOTS.len(), 8);
        assert_eq!(NATIVE_INTERFACE_SLOTS[6], "FindClass");
        assert_eq!(NATIVE_INTERFACE_SLOTS[228], "ExceptionCheck");
        assert_eq!(INVOKE_INTERFACE_SLOTS[6], "GetEnv");
    }

    /// The slots against the NDK's jni.h, when there's an NDK (see
    /// `ndk_jni_h`): the members of `struct JNINativeInterface` and `struct
    /// JNIInvokeInterface`, in order.
    #[test]
    fn slots_are_jni_h_s() {
        let Some(path) = ndk_jni_h() else {
            crate::test_support::skip(
                "android-sdk",
                "no Android NDK (SDL3_ANDROID_NDK, ANDROID_NDK_HOME, ANDROID_NDK_ROOT, or ANDROID_HOME's ndk/) with jni.h",
            );
            return;
        };
        let text = std::fs::read_to_string(&path).unwrap();
        for (name, ours) in [
            ("JNINativeInterface", NATIVE_INTERFACE_SLOTS),
            ("JNIInvokeInterface", INVOKE_INTERFACE_SLOTS),
        ] {
            let theirs = struct_members(&text, name);
            assert_eq!(ours, &theirs[..], "struct {name} of {}", path.display());
        }
        println!("the JNI tables are {}'s", path.display());
    }

    /// The member names of the C struct `name` (each member a function
    /// pointer `(*Name)(...)`, or a `void* reservedN`), in order.
    fn struct_members(text: &str, name: &str) -> Vec<String> {
        let start = text
            .find(&format!("struct {name} {{"))
            .unwrap_or_else(|| panic!("no struct {name} in jni.h"));
        let body = &text[start..];
        let body = &body[..body.find("};").unwrap()];
        // (comments out, then the declarations, split at their semicolons)
        let mut code = String::new();
        let mut rest = body;
        while let Some(i) = rest.find("/*") {
            code.push_str(&rest[..i]);
            rest = &rest[i + rest[i..].find("*/").unwrap() + 2..];
        }
        code.push_str(rest);
        let code: String = code
            .lines()
            .map(|l| l.split("//").next().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let code = &code[code.find('{').unwrap() + 1..];
        let mut names = Vec::new();
        for declaration in code.split(';') {
            let declaration = declaration.trim();
            if declaration.is_empty() {
                continue;
            }
            if let Some(i) = declaration.find("(*") {
                let after = &declaration[i + 2..];
                names.push(after[..after.find(')').unwrap()].trim().to_string());
            } else {
                // void*       reserved0
                let word = declaration
                    .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .next()
                    .unwrap();
                names.push(word.to_string());
            }
        }
        names
    }

    /// jni.h of the NDK the Android link uses (tools/android_link.py's
    /// choice): SDL3_ANDROID_NDK's, ANDROID_NDK_HOME's, ANDROID_NDK_ROOT's,
    /// else the newest in the SDK's ndk/.
    pub(crate) fn ndk_jni_h() -> Option<std::path::PathBuf> {
        use std::path::PathBuf;
        let mut ndks: Vec<PathBuf> = ["SDL3_ANDROID_NDK", "ANDROID_NDK_HOME", "ANDROID_NDK_ROOT"]
            .iter()
            .filter_map(|name| std::env::var_os(name).map(PathBuf::from))
            .collect();
        for sdk in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
            if let Some(sdk) = std::env::var_os(sdk) {
                let mut found: Vec<PathBuf> = std::fs::read_dir(PathBuf::from(sdk).join("ndk"))
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .collect();
                found.sort();
                ndks.extend(found.into_iter().rev());
            }
        }
        ndks.into_iter().find_map(|ndk| {
            let prebuilt = ndk.join("toolchains/llvm/prebuilt");
            std::fs::read_dir(prebuilt)
                .ok()?
                .flatten()
                .find_map(|host| {
                    let path = host.path().join("sysroot/usr/include/jni.h");
                    path.is_file().then_some(path)
                })
        })
    }
}
