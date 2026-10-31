// The OpenSL ES declarations the OpenSL ES driver uses, from the NDK's
// <SLES/OpenSLES.h>, <SLES/OpenSLES_Platform.h> and
// <SLES/OpenSLES_Android.h>; libOpenSLES is linked, as upstream links it.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

use std::ffi::c_void;

pub(super) type SLuint32 = u32;
pub(super) type SLresult = u32;
pub(super) type SLboolean = u32;

/// `struct SLInterfaceID_` (a UUID, only passed by address here).
#[repr(C)]
pub(super) struct SLInterfaceID_ {
    _private: [u8; 0],
}
pub(super) type SLInterfaceID = *const SLInterfaceID_;

/// An interface: a pointer to a pointer to its table of functions.
pub(super) type SLObjectItf = *const *const SLObjectItf_;
pub(super) type SLEngineItf = *const *const SLEngineItf_;
pub(super) type SLPlayItf = *const *const SLPlayItf_;
pub(super) type SLRecordItf = *const *const SLRecordItf_;
pub(super) type SLAndroidSimpleBufferQueueItf = *const *const SLAndroidSimpleBufferQueueItf_;

/// `slAndroidSimpleBufferQueueCallback`.
pub(super) type slAndroidSimpleBufferQueueCallback =
    unsafe extern "C" fn(caller: SLAndroidSimpleBufferQueueItf, context: *mut c_void);

/// `struct SLObjectItf_`, up to `Destroy`.
#[repr(C)]
pub(super) struct SLObjectItf_ {
    pub(super) Realize: unsafe extern "C" fn(SLObjectItf, SLboolean) -> SLresult,
    _Resume: *const c_void,
    _GetState: *const c_void,
    pub(super) GetInterface:
        unsafe extern "C" fn(SLObjectItf, SLInterfaceID, *mut c_void) -> SLresult,
    _RegisterCallback: *const c_void,
    _AbortAsyncOperation: *const c_void,
    pub(super) Destroy: unsafe extern "C" fn(SLObjectItf),
}

/// `struct SLEngineItf_`, up to `CreateOutputMix`.
#[repr(C)]
pub(super) struct SLEngineItf_ {
    _CreateLEDDevice: *const c_void,
    _CreateVibraDevice: *const c_void,
    pub(super) CreateAudioPlayer: unsafe extern "C" fn(
        SLEngineItf,
        *mut SLObjectItf,
        *mut SLDataSource,
        *mut SLDataSink,
        SLuint32,
        *const SLInterfaceID,
        *const SLboolean,
    ) -> SLresult,
    pub(super) CreateAudioRecorder: unsafe extern "C" fn(
        SLEngineItf,
        *mut SLObjectItf,
        *mut SLDataSource,
        *mut SLDataSink,
        SLuint32,
        *const SLInterfaceID,
        *const SLboolean,
    ) -> SLresult,
    _CreateMidiPlayer: *const c_void,
    _CreateListener: *const c_void,
    _Create3DGroup: *const c_void,
    pub(super) CreateOutputMix: unsafe extern "C" fn(
        SLEngineItf,
        *mut SLObjectItf,
        SLuint32,
        *const SLInterfaceID,
        *const SLboolean,
    ) -> SLresult,
}

/// `struct SLPlayItf_`, up to `SetPlayState`.
#[repr(C)]
pub(super) struct SLPlayItf_ {
    pub(super) SetPlayState: unsafe extern "C" fn(SLPlayItf, SLuint32) -> SLresult,
}

/// `struct SLRecordItf_`, up to `SetRecordState`.
#[repr(C)]
pub(super) struct SLRecordItf_ {
    pub(super) SetRecordState: unsafe extern "C" fn(SLRecordItf, SLuint32) -> SLresult,
}

/// `struct SLAndroidSimpleBufferQueueItf_`.
#[repr(C)]
pub(super) struct SLAndroidSimpleBufferQueueItf_ {
    pub(super) Enqueue:
        unsafe extern "C" fn(SLAndroidSimpleBufferQueueItf, *const c_void, SLuint32) -> SLresult,
    _Clear: *const c_void,
    _GetState: *const c_void,
    pub(super) RegisterCallback: unsafe extern "C" fn(
        SLAndroidSimpleBufferQueueItf,
        slAndroidSimpleBufferQueueCallback,
        *mut c_void,
    ) -> SLresult,
}

/// `SLDataLocator_AndroidSimpleBufferQueue`.
#[repr(C)]
pub(super) struct SLDataLocator_AndroidSimpleBufferQueue {
    pub(super) locatorType: SLuint32,
    pub(super) numBuffers: SLuint32,
}

/// `SLDataLocator_IODevice`.
#[repr(C)]
pub(super) struct SLDataLocator_IODevice {
    pub(super) locatorType: SLuint32,
    pub(super) deviceType: SLuint32,
    pub(super) deviceID: SLuint32,
    pub(super) device: SLObjectItf,
}

/// `SLDataLocator_OutputMix`.
#[repr(C)]
pub(super) struct SLDataLocator_OutputMix {
    pub(super) locatorType: SLuint32,
    pub(super) outputMix: SLObjectItf,
}

/// `SLDataFormat_PCM`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct SLDataFormat_PCM {
    pub(super) formatType: SLuint32,
    pub(super) numChannels: SLuint32,
    pub(super) samplesPerSec: SLuint32,
    pub(super) bitsPerSample: SLuint32,
    pub(super) containerSize: SLuint32,
    pub(super) channelMask: SLuint32,
    pub(super) endianness: SLuint32,
}

/// `SLAndroidDataFormat_PCM_EX`.
#[repr(C)]
pub(super) struct SLAndroidDataFormat_PCM_EX {
    pub(super) formatType: SLuint32,
    pub(super) numChannels: SLuint32,
    pub(super) sampleRate: SLuint32,
    pub(super) bitsPerSample: SLuint32,
    pub(super) containerSize: SLuint32,
    pub(super) channelMask: SLuint32,
    pub(super) endianness: SLuint32,
    pub(super) representation: SLuint32,
}

/// `SLDataSource`.
#[repr(C)]
pub(super) struct SLDataSource {
    pub(super) pLocator: *mut c_void,
    pub(super) pFormat: *mut c_void,
}

/// `SLDataSink`.
#[repr(C)]
pub(super) struct SLDataSink {
    pub(super) pLocator: *mut c_void,
    pub(super) pFormat: *mut c_void,
}

pub(super) const SL_RESULT_SUCCESS: SLresult = 0;
pub(super) const SL_BOOLEAN_FALSE: SLboolean = 0;
pub(super) const SL_BOOLEAN_TRUE: SLboolean = 1;

pub(super) const SL_DATALOCATOR_IODEVICE: SLuint32 = 0x0000_0003;
pub(super) const SL_DATALOCATOR_OUTPUTMIX: SLuint32 = 0x0000_0004;
pub(super) const SL_DATALOCATOR_ANDROIDSIMPLEBUFFERQUEUE: SLuint32 = 0x8000_07BD;
pub(super) const SL_IODEVICE_AUDIOINPUT: SLuint32 = 0x0000_0001;
pub(super) const SL_DEFAULTDEVICEID_AUDIOINPUT: SLuint32 = 0xFFFF_FFFF;

pub(super) const SL_DATAFORMAT_PCM: SLuint32 = 0x0000_0002;
pub(super) const SL_ANDROID_DATAFORMAT_PCM_EX: SLuint32 = 0x0000_0004;
pub(super) const SL_ANDROID_PCM_REPRESENTATION_FLOAT: SLuint32 = 0x0000_0003;
pub(super) const SL_BYTEORDER_BIGENDIAN: SLuint32 = 0x0000_0001;
pub(super) const SL_BYTEORDER_LITTLEENDIAN: SLuint32 = 0x0000_0002;

pub(super) const SL_PLAYSTATE_STOPPED: SLuint32 = 0x0000_0001;
pub(super) const SL_PLAYSTATE_PAUSED: SLuint32 = 0x0000_0002;
pub(super) const SL_PLAYSTATE_PLAYING: SLuint32 = 0x0000_0003;
pub(super) const SL_RECORDSTATE_STOPPED: SLuint32 = 0x0000_0001;
pub(super) const SL_RECORDSTATE_RECORDING: SLuint32 = 0x0000_0003;

pub(super) const SL_SPEAKER_FRONT_LEFT: SLuint32 = 0x0000_0001;
pub(super) const SL_SPEAKER_FRONT_RIGHT: SLuint32 = 0x0000_0002;
pub(super) const SL_SPEAKER_FRONT_CENTER: SLuint32 = 0x0000_0004;
pub(super) const SL_SPEAKER_LOW_FREQUENCY: SLuint32 = 0x0000_0008;
pub(super) const SL_SPEAKER_BACK_LEFT: SLuint32 = 0x0000_0010;
pub(super) const SL_SPEAKER_BACK_RIGHT: SLuint32 = 0x0000_0020;
pub(super) const SL_SPEAKER_BACK_CENTER: SLuint32 = 0x0000_0100;
pub(super) const SL_SPEAKER_SIDE_LEFT: SLuint32 = 0x0000_0200;
pub(super) const SL_SPEAKER_SIDE_RIGHT: SLuint32 = 0x0000_0400;

#[link(name = "OpenSLES")]
unsafe extern "C" {
    pub(super) fn slCreateEngine(
        pEngine: *mut SLObjectItf,
        numOptions: SLuint32,
        pEngineOptions: *const c_void,
        numInterfaces: SLuint32,
        pInterfaceIds: *const SLInterfaceID,
        pInterfaceRequired: *const SLboolean,
    ) -> SLresult;

    pub(super) static SL_IID_ENGINE: SLInterfaceID;
    pub(super) static SL_IID_VOLUME: SLInterfaceID;
    pub(super) static SL_IID_PLAY: SLInterfaceID;
    pub(super) static SL_IID_RECORD: SLInterfaceID;
    pub(super) static SL_IID_ANDROIDSIMPLEBUFFERQUEUE: SLInterfaceID;
}
