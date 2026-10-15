// The CoreAudio and AudioToolbox declarations the CoreAudio driver uses,
// from <CoreAudio/AudioHardware.h>, <CoreAudio/AudioHardwareBase.h>,
// <CoreAudioTypes/CoreAudioBaseTypes.h> and <AudioToolbox/AudioQueue.h>.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

#![allow(non_upper_case_globals, non_snake_case)]

use std::ffi::c_void;

use crate::core::apple::foundation::{CFRunLoopRef, CFStringRef};

/// `OSStatus`.
pub(super) type OSStatus = i32;
/// `AudioObjectID`.
pub(super) type AudioObjectID = u32;
/// `AudioDeviceID`.
pub(super) type AudioDeviceID = AudioObjectID;
/// `AudioObjectPropertySelector`, `...Scope`, `...Element`.
pub(super) type AudioObjectPropertySelector = u32;
pub(super) type AudioObjectPropertyScope = u32;
pub(super) type AudioObjectPropertyElement = u32;
/// `AudioQueueRef`.
pub(super) type AudioQueueRef = *mut c_void;
/// `AudioQueueBufferRef`.
pub(super) type AudioQueueBufferRef = *mut AudioQueueBuffer;
/// `AudioQueuePropertyID`.
pub(super) type AudioQueuePropertyID = u32;
/// `AudioChannelLayoutTag`.
pub(super) type AudioChannelLayoutTag = u32;

/// A four-character code, as the headers spell them (`'dev#'`).
const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | s[3] as u32
}

pub(super) const noErr: OSStatus = 0;
pub(super) const kAudioHardwareNoError: OSStatus = 0;
pub(super) const kAudioHardwareBadDeviceError: OSStatus = fourcc(b"!dev") as OSStatus;

pub(super) const kAudioObjectSystemObject: AudioObjectID = 1;

pub(super) const kAudioObjectPropertyScopeGlobal: AudioObjectPropertyScope = fourcc(b"glob");
pub(super) const kAudioDevicePropertyScopeInput: AudioObjectPropertyScope = fourcc(b"inpt");
pub(super) const kAudioDevicePropertyScopeOutput: AudioObjectPropertyScope = fourcc(b"outp");
pub(super) const kAudioObjectPropertyElementMain: AudioObjectPropertyElement = 0;

pub(super) const kAudioHardwarePropertyDevices: AudioObjectPropertySelector = fourcc(b"dev#");
pub(super) const kAudioHardwarePropertyDefaultOutputDevice: AudioObjectPropertySelector =
    fourcc(b"dOut");
pub(super) const kAudioHardwarePropertyDefaultInputDevice: AudioObjectPropertySelector =
    fourcc(b"dIn ");
pub(super) const kAudioDevicePropertyDeviceIsAlive: AudioObjectPropertySelector = fourcc(b"livn");
pub(super) const kAudioDevicePropertyStreamConfiguration: AudioObjectPropertySelector =
    fourcc(b"slay");
pub(super) const kAudioDevicePropertyNominalSampleRate: AudioObjectPropertySelector =
    fourcc(b"nsrt");
pub(super) const kAudioObjectPropertyName: AudioObjectPropertySelector = fourcc(b"lnam");
pub(super) const kAudioDevicePropertyDeviceUID: AudioObjectPropertySelector = fourcc(b"uid ");
pub(super) const kAudioDevicePropertyHogMode: AudioObjectPropertySelector = fourcc(b"oink");

pub(super) const kAudioQueueProperty_CurrentDevice: AudioQueuePropertyID = fourcc(b"aqcd");
pub(super) const kAudioQueueProperty_ChannelLayout: AudioQueuePropertyID = fourcc(b"aqcl");

pub(super) const kAudioFormatLinearPCM: u32 = fourcc(b"lpcm");
pub(super) const kLinearPCMFormatFlagIsFloat: u32 = 1 << 0;
pub(super) const kLinearPCMFormatFlagIsBigEndian: u32 = 1 << 1;
pub(super) const kLinearPCMFormatFlagIsSignedInteger: u32 = 1 << 2;
pub(super) const kLinearPCMFormatFlagIsPacked: u32 = 1 << 3;

pub(super) const kAudioChannelLayoutTag_Mono: AudioChannelLayoutTag = (100 << 16) | 1;
pub(super) const kAudioChannelLayoutTag_Stereo: AudioChannelLayoutTag = (101 << 16) | 2;
pub(super) const kAudioChannelLayoutTag_Quadraphonic: AudioChannelLayoutTag = (108 << 16) | 4;
pub(super) const kAudioChannelLayoutTag_MPEG_5_1_A: AudioChannelLayoutTag = (121 << 16) | 6;
pub(super) const kAudioChannelLayoutTag_MPEG_6_1_A: AudioChannelLayoutTag = (125 << 16) | 7;
pub(super) const kAudioChannelLayoutTag_MPEG_7_1_C: AudioChannelLayoutTag = (128 << 16) | 8;
pub(super) const kAudioChannelLayoutTag_DVD_4: AudioChannelLayoutTag = (133 << 16) | 3;
pub(super) const kAudioChannelLayoutTag_DVD_6: AudioChannelLayoutTag = (135 << 16) | 5;
pub(super) const kAudioChannelLayoutTag_DVD_12: AudioChannelLayoutTag =
    kAudioChannelLayoutTag_MPEG_5_1_A;
pub(super) const kAudioChannelLayoutTag_WAVE_6_1: AudioChannelLayoutTag = (188 << 16) | 7;
pub(super) const kAudioChannelLayoutTag_WAVE_7_1: AudioChannelLayoutTag = (189 << 16) | 8;

/// `AudioObjectPropertyAddress`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AudioObjectPropertyAddress {
    pub(super) mSelector: AudioObjectPropertySelector,
    pub(super) mScope: AudioObjectPropertyScope,
    pub(super) mElement: AudioObjectPropertyElement,
}

/// `AudioBuffer`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct AudioBuffer {
    pub(super) mNumberChannels: u32,
    pub(super) mDataByteSize: u32,
    pub(super) mData: *mut c_void,
}

/// The fixed part of `AudioBufferList` (`mBuffers` has
/// `mNumberBuffers` elements).
#[repr(C)]
pub(super) struct AudioBufferList {
    pub(super) mNumberBuffers: u32,
    pub(super) mBuffers: [AudioBuffer; 1],
}

/// `AudioStreamBasicDescription`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct AudioStreamBasicDescription {
    pub(super) mSampleRate: f64,
    pub(super) mFormatID: u32,
    pub(super) mFormatFlags: u32,
    pub(super) mBytesPerPacket: u32,
    pub(super) mFramesPerPacket: u32,
    pub(super) mBytesPerFrame: u32,
    pub(super) mChannelsPerFrame: u32,
    pub(super) mBitsPerChannel: u32,
    pub(super) mReserved: u32,
}

/// `AudioChannelDescription`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct AudioChannelDescription {
    pub(super) mChannelLabel: u32,
    pub(super) mChannelFlags: u32,
    pub(super) mCoordinates: [f32; 3],
}

/// `AudioChannelLayout` (with its one inline channel description).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct AudioChannelLayout {
    pub(super) mChannelLayoutTag: AudioChannelLayoutTag,
    pub(super) mChannelBitmap: u32,
    pub(super) mNumberChannelDescriptions: u32,
    pub(super) mChannelDescriptions: [AudioChannelDescription; 1],
}

/// `AudioQueueBuffer`.
#[repr(C)]
pub(super) struct AudioQueueBuffer {
    pub(super) mAudioDataBytesCapacity: u32,
    pub(super) mAudioData: *mut c_void,
    pub(super) mAudioDataByteSize: u32,
    pub(super) mUserData: *mut c_void,
    pub(super) mPacketDescriptionCapacity: u32,
    pub(super) mPacketDescriptions: *mut c_void,
    pub(super) mPacketDescriptionCount: u32,
}

/// `AudioTimeStamp` (only passed by pointer).
#[repr(C)]
pub(super) struct AudioTimeStamp {
    _private: [u8; 0],
}

/// `AudioStreamPacketDescription` (only passed by pointer).
#[repr(C)]
pub(super) struct AudioStreamPacketDescription {
    _private: [u8; 0],
}

/// `AudioObjectPropertyListenerProc`.
pub(super) type AudioObjectPropertyListenerProc = unsafe extern "C" fn(
    in_object_id: AudioObjectID,
    in_number_addresses: u32,
    in_addresses: *const AudioObjectPropertyAddress,
    in_client_data: *mut c_void,
) -> OSStatus;

/// `AudioQueueOutputCallback`.
pub(super) type AudioQueueOutputCallback = unsafe extern "C" fn(
    in_user_data: *mut c_void,
    in_aq: AudioQueueRef,
    in_buffer: AudioQueueBufferRef,
);

/// `AudioQueueInputCallback`.
pub(super) type AudioQueueInputCallback = unsafe extern "C" fn(
    in_user_data: *mut c_void,
    in_aq: AudioQueueRef,
    in_buffer: AudioQueueBufferRef,
    in_start_time: *const AudioTimeStamp,
    in_number_packet_descriptions: u32,
    in_packet_descs: *const AudioStreamPacketDescription,
);

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    pub(super) fn AudioObjectGetPropertyDataSize(
        in_object_id: AudioObjectID,
        in_address: *const AudioObjectPropertyAddress,
        in_qualifier_data_size: u32,
        in_qualifier_data: *const c_void,
        out_data_size: *mut u32,
    ) -> OSStatus;
    pub(super) fn AudioObjectGetPropertyData(
        in_object_id: AudioObjectID,
        in_address: *const AudioObjectPropertyAddress,
        in_qualifier_data_size: u32,
        in_qualifier_data: *const c_void,
        io_data_size: *mut u32,
        out_data: *mut c_void,
    ) -> OSStatus;
    pub(super) fn AudioObjectAddPropertyListener(
        in_object_id: AudioObjectID,
        in_address: *const AudioObjectPropertyAddress,
        in_listener: AudioObjectPropertyListenerProc,
        in_client_data: *mut c_void,
    ) -> OSStatus;
    pub(super) fn AudioObjectRemovePropertyListener(
        in_object_id: AudioObjectID,
        in_address: *const AudioObjectPropertyAddress,
        in_listener: AudioObjectPropertyListenerProc,
        in_client_data: *mut c_void,
    ) -> OSStatus;
}

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    pub(super) fn AudioQueueNewOutput(
        in_format: *const AudioStreamBasicDescription,
        in_callback_proc: AudioQueueOutputCallback,
        in_user_data: *mut c_void,
        in_callback_run_loop: CFRunLoopRef,
        in_callback_run_loop_mode: CFStringRef,
        in_flags: u32,
        out_aq: *mut AudioQueueRef,
    ) -> OSStatus;
    pub(super) fn AudioQueueNewInput(
        in_format: *const AudioStreamBasicDescription,
        in_callback_proc: AudioQueueInputCallback,
        in_user_data: *mut c_void,
        in_callback_run_loop: CFRunLoopRef,
        in_callback_run_loop_mode: CFStringRef,
        in_flags: u32,
        out_aq: *mut AudioQueueRef,
    ) -> OSStatus;
    pub(super) fn AudioQueueSetProperty(
        in_aq: AudioQueueRef,
        in_id: AudioQueuePropertyID,
        in_data: *const c_void,
        in_data_size: u32,
    ) -> OSStatus;
    pub(super) fn AudioQueueAllocateBuffer(
        in_aq: AudioQueueRef,
        in_buffer_byte_size: u32,
        out_buffer: *mut AudioQueueBufferRef,
    ) -> OSStatus;
    pub(super) fn AudioQueueEnqueueBuffer(
        in_aq: AudioQueueRef,
        in_buffer: AudioQueueBufferRef,
        in_num_packet_descs: u32,
        in_packet_descs: *const AudioStreamPacketDescription,
    ) -> OSStatus;
    pub(super) fn AudioQueueStart(
        in_aq: AudioQueueRef,
        in_start_time: *const AudioTimeStamp,
    ) -> OSStatus;
    pub(super) fn AudioQueueFlush(in_aq: AudioQueueRef) -> OSStatus;
    pub(super) fn AudioQueueStop(in_aq: AudioQueueRef, in_immediate: u8) -> OSStatus;
    pub(super) fn AudioQueueDispose(in_aq: AudioQueueRef, in_immediate: u8) -> OSStatus;
}
