#!/usr/bin/env python3
"""Generate sdl3-mixer's test audio (sdl3-mixer/src/testdata/audio/).

Everything is made from a synthetic signal (a few sines, a chirp and
some LCG noise), so there's nothing copyrighted in the files and they stay
small (a quarter second or so each). The encoded formats come from ffmpeg
(with its libmp3lame, libvorbis, FLAC, ADPCM and PCM encoders and its WAV,
AIFF, AU, VOC, MP3, MP2, Ogg and FLAC muxers); the files with chunks or tags
ffmpeg doesn't write (WAV sample loops and ID3 chunks, VOC loops, silence and
text blocks, APE tags, junk before the first MP3 frame) are put together
here.

Encoders change between versions, so regenerating the files will likely
change them, and then sdl3-mixer's reference output
(`testdata/reference.txt`, made by a C program built from upstream
SDL_mixer and SDL3) must be regenerated too.

usage: tools/gen_sdl_mixer_testdata.py [OUTDIR]
"""

import math
import os
import struct
import subprocess
import sys
import tempfile

OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
    os.path.dirname(os.path.abspath(__file__)), '..', 'sdl3-mixer', 'src', 'testdata', 'audio')


class Lcg:
    def __init__(self, seed):
        self.s = seed & 0xFFFFFFFF

    def next(self):
        self.s = (self.s * 1664525 + 1013904223) & 0xFFFFFFFF
        return self.s

    def unit(self):
        return self.next() / 2147483648.0 - 1.0


def signal(freq, channels, seconds, seed=1):
    """Interleaved float samples in [-1, 1]."""
    n = int(freq * seconds)
    rng = Lcg(seed)
    out = []
    for i in range(n):
        t = i / freq
        env = min(1.0, i / (freq * 0.02)) * min(1.0, (n - i) / (freq * 0.02))
        for c in range(channels):
            f0 = 220.0 * (c + 1)
            v = 0.45 * math.sin(2 * math.pi * f0 * t)
            v += 0.2 * math.sin(2 * math.pi * (f0 * 3.01) * t + c)
            v += 0.15 * math.sin(2 * math.pi * (300.0 + 3000.0 * t) * t)
            v += 0.05 * rng.unit()
            out.append(max(-1.0, min(1.0, v * env)))
    return out


def s16(samples):
    return b''.join(struct.pack('<h', int(round(s * 32767))) for s in samples)


def write(name, data):
    with open(os.path.join(OUT, name), 'wb') as f:
        f.write(data)


def ffmpeg(name, freq, channels, seconds, args, fmt=None, seed=1, metadata=()):
    """Encode the test signal with ffmpeg."""
    with tempfile.TemporaryDirectory() as tmp:
        src = os.path.join(tmp, 'src.wav')
        samples = signal(freq, channels, seconds, seed)
        data = s16(samples)
        hdr = struct.pack('<4sI4s4sIHHIIHH4sI', b'RIFF', 36 + len(data), b'WAVE', b'fmt ', 16, 1,
                          channels, freq, freq * channels * 2, channels * 2, 16, b'data', len(data))
        with open(src, 'wb') as f:
            f.write(hdr + data)
        cmd = ['ffmpeg', '-hide_banner', '-loglevel', 'error', '-y', '-i', src,
               '-map_metadata', '-1', '-fflags', '+bitexact', '-flags:a', '+bitexact']
        for k, v in metadata:
            cmd += ['-metadata', '%s=%s' % (k, v)]
        cmd += list(args)
        if fmt:
            cmd += ['-f', fmt]
        dst = os.path.join(tmp, 'out')
        cmd.append(dst)
        subprocess.run(cmd, check=True)
        with open(dst, 'rb') as f:
            out = f.read()
    write(name, out)
    return out


def chunk(cid, data):
    pad = b'\0' if len(data) & 1 else b''
    return cid + struct.pack('<I', len(data)) + data + pad


def id3v2(frames, version=3):
    """A minimal ID3v2.3/2.4 tag with text frames."""
    body = b''
    for fid, text in frames:
        payload = b'\x00' + text.encode('latin-1')
        if version == 4:
            n = len(payload)
            size = bytes([(n >> 21) & 0x7F, (n >> 14) & 0x7F, (n >> 7) & 0x7F, n & 0x7F])
        else:
            size = struct.pack('>I', len(payload))
        body += fid + size + b'\0\0' + payload
    n = len(body)
    syncsafe = bytes([(n >> 21) & 0x7F, (n >> 14) & 0x7F, (n >> 7) & 0x7F, n & 0x7F])
    return b'ID3' + bytes([version, 0, 0]) + syncsafe + body


def ape_tag(items):
    body = b''
    for k, v in items:
        v = v.encode('utf-8')
        body += struct.pack('<II', len(v), 0) + k.encode('ascii') + b'\0' + v
    size = len(body) + 32
    footer = b'APETAGEX' + struct.pack('<IIII', 2000, size, len(items), 0) + b'\0' * 8
    header = b'APETAGEX' + struct.pack('<IIII', 2000, size, len(items), 0xA0000000) + b'\0' * 8
    return header + body + footer


def wav_files():
    ffmpeg('wav_u8_mono.wav', 8000, 1, 0.25, ['-c:a', 'pcm_u8'], 'wav')
    ffmpeg('wav_s16_stereo.wav', 22050, 2, 0.25, ['-c:a', 'pcm_s16le'], 'wav', seed=2)
    ffmpeg('wav_s24_mono.wav', 11025, 1, 0.25, ['-c:a', 'pcm_s24le'], 'wav', seed=3)
    ffmpeg('wav_s32_stereo.wav', 11025, 2, 0.2, ['-c:a', 'pcm_s32le'], 'wav', seed=4)
    ffmpeg('wav_f32_mono.wav', 16000, 1, 0.25, ['-c:a', 'pcm_f32le'], 'wav', seed=5)
    ffmpeg('wav_f64_stereo.wav', 8000, 2, 0.2, ['-c:a', 'pcm_f64le'], 'wav', seed=6)
    ffmpeg('wav_ulaw.wav', 8000, 1, 0.25, ['-c:a', 'pcm_mulaw'], 'wav', seed=7)
    ffmpeg('wav_alaw.wav', 8000, 2, 0.25, ['-c:a', 'pcm_alaw'], 'wav', seed=8)
    ffmpeg('wav_msadpcm_mono.wav', 11025, 1, 0.3, ['-c:a', 'adpcm_ms'], 'wav', seed=9)
    ffmpeg('wav_msadpcm_stereo.wav', 22050, 2, 0.25, ['-c:a', 'adpcm_ms', '-block_size', '512'], 'wav', seed=10)
    ffmpeg('wav_ima_mono.wav', 11025, 1, 0.3, ['-c:a', 'adpcm_ima_wav'], 'wav', seed=11)
    ffmpeg('wav_ima_stereo.wav', 22050, 2, 0.25, ['-c:a', 'adpcm_ima_wav', '-block_size', '512'], 'wav', seed=12)
    ffmpeg('wav_quad.wav', 8000, 4, 0.2, ['-c:a', 'pcm_s16le'], 'wav', seed=13)

    # Sample loops, LIST/INFO and id3 chunks.
    freq, channels = 11025, 1
    data = s16(signal(freq, channels, 0.25, 14))
    fmt = struct.pack('<HHIIHH', 1, channels, freq, freq * channels * 2, channels * 2, 16)
    loop = struct.pack('<IIIIII', 0, 0, 1000, 2000, 0, 2)  # id, type, start, end, fraction, count
    smpl = struct.pack('<IIIIIIIII', 0, 0, 1000000000 // freq, 60, 0, 0, 0, 1, 0) + loop
    info = b'INFO' + chunk(b'INAM', b'Loop test\0') + chunk(b'IART', b'sdl3-mixer\0') + \
        chunk(b'IALB', b'Test data\0') + chunk(b'BCPR', b'none\0')
    id3 = id3v2([(b'TIT2', 'ID3 in WAV'), (b'TRCK', '7/9'), (b'TYER', '1999')])
    body = b'WAVE' + chunk(b'fmt ', fmt) + chunk(b'LIST', info) + chunk(b'smpl', smpl) + \
        chunk(b'data', data) + chunk(b'id3 ', id3)
    write('wav_loop.wav', b'RIFF' + struct.pack('<I', len(body)) + body)


def aiff_files():
    meta = (('title', 'AIFF test'), ('author', 'sdl3-mixer'), ('copyright', 'none'), ('comment', 'a note'))
    ffmpeg('aiff_s16_stereo.aiff', 22050, 2, 0.2, ['-c:a', 'pcm_s16be'], 'aiff', seed=20, metadata=meta)
    ffmpeg('aiff_s8_mono.aiff', 8000, 1, 0.25, ['-c:a', 'pcm_s8'], 'aiff', seed=21)
    ffmpeg('aiff_s24_mono.aiff', 11025, 1, 0.2, ['-c:a', 'pcm_s24be'], 'aiff', seed=22)
    ffmpeg('aiff_s32_stereo.aiff', 8000, 2, 0.2, ['-c:a', 'pcm_s32be'], 'aiff', seed=23)
    ffmpeg('aifc_f32_mono.aiff', 11025, 1, 0.2, ['-c:a', 'pcm_f32be'], 'aiff', seed=24)
    ffmpeg('aifc_f64_mono.aiff', 8000, 1, 0.2, ['-c:a', 'pcm_f64be'], 'aiff', seed=25)
    ffmpeg('aifc_ulaw.aiff', 8000, 1, 0.25, ['-c:a', 'pcm_mulaw'], 'aiff', seed=26)
    ffmpeg('aifc_alaw.aiff', 8000, 2, 0.2, ['-c:a', 'pcm_alaw'], 'aiff', seed=27)


def au_files():
    ffmpeg('au_ulaw.au', 8000, 1, 0.25, ['-c:a', 'pcm_mulaw'], 'au', seed=30)
    ffmpeg('au_s8_stereo.au', 11025, 2, 0.2, ['-c:a', 'pcm_s8'], 'au', seed=31)
    ffmpeg('au_s16_mono.au', 16000, 1, 0.2, ['-c:a', 'pcm_s16be'], 'au', seed=32)
    ffmpeg('au_s24_unsupported.au', 8000, 1, 0.1, ['-c:a', 'pcm_s24be'], 'au', seed=33)
    # No header: 8kHz mono mu-law.
    rng = Lcg(34)
    write('au_headerless.au', bytes(rng.next() >> 24 for _ in range(2000)))


def voc_block(btype, data):
    return bytes([btype]) + struct.pack('<I', len(data))[:3] + data


def voc_files():
    ffmpeg('voc_u8_mono.voc', 8000, 1, 0.25, ['-c:a', 'pcm_u8'], 'voc', seed=40)
    ffmpeg('voc_s16_stereo.voc', 11025, 2, 0.2, ['-c:a', 'pcm_s16le'], 'voc', seed=41)

    # Old style blocks: sound data, silence, text, a repeated block, and an
    # extended (stereo) block. The format gives the end-of-repeat block a
    # (zero) size like every other block, but SDL_mixer reads it without
    # one (and stops at what it takes for a terminator block), so there's a
    # file of each.
    hdr = b'Creative Voice File\x1a' + struct.pack('<HHH', 26, 0x010A, (~0x010A + 0x1234) & 0xFFFF)
    rate = 256 - 1000000 // 8000
    u8 = lambda samples: bytes(int(round(s * 127)) + 128 for s in samples)
    for name, endloop, count in (('voc_blocks.voc', voc_block(7, b''), 2), ('voc_loop_nosize.voc', b'\x07', 2),
                                 ('voc_loop_infinite.voc', b'\x07', 0xFFFF)):
        blocks = voc_block(1, bytes([rate, 0]) + u8(signal(8000, 1, 0.05, 42)))
        blocks += voc_block(3, struct.pack('<HB', 399, rate))  # 400 frames of silence
        blocks += voc_block(5, b'some text\0')
        blocks += voc_block(6, struct.pack('<H', count))  # repeat the next blocks
        blocks += voc_block(1, bytes([rate, 0]) + u8(signal(8000, 1, 0.02, 43)))
        blocks += endloop
        tc = 65536 - (256000000 // (2 * 11025))
        blocks += voc_block(8, struct.pack('<HBB', tc, 0, 1))
        blocks += voc_block(1, bytes([0, 0]) + u8(signal(11025, 2, 0.03, 44)))
        blocks += b'\0'
        write(name, hdr + blocks)

    # New style (type 9) 16-bit blocks.
    data = s16(signal(11025, 1, 0.1, 45))
    blocks = voc_block(9, struct.pack('<IBBHI', 11025, 16, 1, 4, 0) + data)
    blocks += voc_block(9, struct.pack('<IBBHI', 11025, 8, 1, 0, 0) + u8(signal(11025, 1, 0.05, 46)))
    blocks += b'\0'
    hdr = b'Creative Voice File\x1a' + struct.pack('<HHH', 26, 0x0114, (~0x0114 + 0x1234) & 0xFFFF)
    write('voc_type9.voc', hdr + blocks)


def mp3_files():
    meta = (('title', 'MP3 test'), ('artist', 'sdl3-mixer'), ('album', 'Test data'),
            ('track', '3/12'), ('date', '2001'), ('copyright', 'none'))
    ffmpeg('mp3_cbr_mono.mp3', 22050, 1, 0.3, ['-c:a', 'libmp3lame', '-b:a', '32k'], 'mp3', seed=50)
    ffmpeg('mp3_vbr_stereo.mp3', 44100, 2, 0.25, ['-c:a', 'libmp3lame', '-q:a', '4', '-id3v2_version', '3',
                                                    '-write_id3v1', '1'], 'mp3', seed=51, metadata=meta)
    ffmpeg('mp3_mpeg2_noxing.mp3', 16000, 1, 0.3, ['-c:a', 'libmp3lame', '-b:a', '24k', '-write_xing', '0',
                                                     '-id3v2_version', '0'], 'mp3', seed=52)
    ffmpeg('mp3_mpeg25.mp3', 8000, 1, 0.4, ['-c:a', 'libmp3lame', '-b:a', '16k'], 'mp3', seed=53)
    ffmpeg('mp3_joint_stereo.mp3', 32000, 2, 0.25, ['-c:a', 'libmp3lame', '-b:a', '64k', '-joint_stereo', '1',
                                                      '-id3v2_version', '4'], 'mp3', seed=54, metadata=meta[:2])
    ffmpeg('mp2_stereo.mp2', 32000, 2, 0.25, ['-c:a', 'mp2', '-b:a', '128k'], 'mp2', seed=55)
    plain = ffmpeg('mp3_plain.mp3', 22050, 2, 0.2, ['-c:a', 'libmp3lame', '-b:a', '48k', '-write_xing', '0',
                                                      '-id3v2_version', '0'], 'mp3', seed=56)
    os.remove(os.path.join(OUT, 'mp3_plain.mp3'))
    id3v1 = b'TAG' + b'ID3v1 title'.ljust(30, b'\0') + b'ID3v1 artist'.ljust(30, b'\0') + \
        b'ID3v1 album'.ljust(30, b'\0') + b'1987' + b'comment'.ljust(28, b'\0') + b'\0\x05' + b'\x0c'
    write('mp3_ape_id3v1.mp3', plain + ape_tag([('Title', 'APE title'), ('Artist', 'APE artist')]) + id3v1)
    rng = Lcg(57)
    write('mp3_junk.mp3', bytes(rng.next() >> 24 for _ in range(700)) + plain)


def ogg_files():
    ffmpeg('ogg_mono.ogg', 22050, 1, 0.3, ['-c:a', 'libvorbis', '-q:a', '0'], 'ogg', seed=60,
           metadata=(('TITLE', 'Vorbis test'), ('ARTIST', 'sdl3-mixer'), ('LOOPSTART', '1000'),
                     ('LOOPLENGTH', '2000')))
    ffmpeg('ogg_stereo.ogg', 44100, 2, 0.25, ['-c:a', 'libvorbis', '-q:a', '2'], 'ogg', seed=61,
           metadata=(('LOOPSTART', '00:00:00.010'), ('LOOPEND', '0.2')))
    ffmpeg('ogg_6ch.ogg', 11025, 6, 0.2, ['-c:a', 'libvorbis', '-q:a', '1'], 'ogg', seed=62)
    ffmpeg('ogg_8k.ogg', 8000, 1, 0.5, ['-c:a', 'libvorbis', '-q:a', '-1'], 'ogg', seed=63)


def flac_files():
    ffmpeg('flac_s16_stereo.flac', 22050, 2, 0.25, ['-c:a', 'flac'], 'flac', seed=70,
           metadata=(('TITLE', 'FLAC test'), ('ARTIST', 'sdl3-mixer'), ('ALBUM', 'Test data'),
                     ('LOOPSTART', '500'), ('LOOPLENGTH', '3000')))
    ffmpeg('flac_s24_mono.flac', 11025, 1, 0.25, ['-c:a', 'flac', '-sample_fmt', 's32', '-bits_per_raw_sample', '24',
                                                    '-compression_level', '12'], 'flac', seed=71)
    ffmpeg('flac_fixed_mono.flac', 8000, 1, 0.3, ['-c:a', 'flac', '-compression_level', '0'], 'flac', seed=72)
    ffmpeg('flac_6ch.flac', 8000, 6, 0.15, ['-c:a', 'flac', '-compression_level', '5'], 'flac', seed=73)
    ffmpeg('flac_ogg.oga', 16000, 2, 0.2, ['-c:a', 'flac'], 'ogg', seed=74,
           metadata=(('TITLE', 'Ogg FLAC'),))


def main():
    os.makedirs(OUT, exist_ok=True)
    wav_files()
    aiff_files()
    au_files()
    voc_files()
    mp3_files()
    ogg_files()
    flac_files()


if __name__ == '__main__':
    main()
