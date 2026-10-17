#!/usr/bin/env python3
"""Generate sdl3-mixer's test audio (sdl3-mixer/src/testdata/audio/).

Everything is made from a synthetic signal (a few sines, a chirp and
some LCG noise), so there's nothing copyrighted in the files and they stay
small (a quarter second or so each). The encoded formats come from ffmpeg
(with its libmp3lame, libvorbis, libopus, FLAC, ADPCM and PCM encoders and
its WAV, AIFF, AU, VOC, MP3, MP2, Ogg and FLAC muxers); the files with chunks
or tags ffmpeg doesn't write (WAV sample loops and ID3 chunks, VOC loops,
silence and text blocks, APE tags, junk before the first MP3 frame) and the
Ogg Opus streams spliced from ffmpeg's packets (mode transitions, lost
pages, an output gain) or patched (an unsupported channel mapping) are put
together here.

Encoders change between versions, so regenerating the files will likely
change them, and then sdl3-mixer's reference output
(`testdata/reference.txt`, made by a C program built from upstream
SDL_mixer and SDL3) must be regenerated too. That program hashes the
mixer's F32 output with every NaN sample replaced by the canonical NaN
(0x7FC00000): the raw audio of random bytes taken as floats produces NaNs,
whose payload bits depend on the operand order the compiler and CPU pick.

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


def ffmpeg(name, freq, channels, seconds, args, fmt=None, seed=1, metadata=(), sig=None, save=True):
    """Encode the test signal (or `sig`'s) with ffmpeg; `name` is written unless `save` is false."""
    with tempfile.TemporaryDirectory() as tmp:
        src = os.path.join(tmp, 'src.wav')
        samples = (sig or signal)(freq, channels, seconds, seed)
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
    if save:
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


def corrupt(data, v):
    """`data` with 1 + v % 8 random bytes changed (variant `v`)."""
    out = bytearray(data)
    rng = Lcg(v * 7 + 12345 + len(data))
    for _ in range(1 + v % 8):
        at = (rng.next() >> 8) % len(data)
        out[at] ^= 1 + (rng.next() >> 24) % 255
    return bytes(out)


def mp3_files():
    meta = (('title', 'MP3 test'), ('artist', 'sdl3-mixer'), ('album', 'Test data'),
            ('track', '3/12'), ('date', '2001'), ('copyright', 'none'))
    ffmpeg('mp3_cbr_mono.mp3', 22050, 1, 0.3, ['-c:a', 'libmp3lame', '-b:a', '32k'], 'mp3', seed=50)
    ffmpeg('mp3_vbr_stereo.mp3', 44100, 2, 0.25, ['-c:a', 'libmp3lame', '-q:a', '4', '-id3v2_version', '3',
                                                    '-write_id3v1', '1'], 'mp3', seed=51, metadata=meta)
    ffmpeg('mp3_mpeg2_noxing.mp3', 16000, 1, 0.3, ['-c:a', 'libmp3lame', '-b:a', '24k', '-write_xing', '0',
                                                     '-id3v2_version', '0'], 'mp3', seed=52)
    mpeg25 = ffmpeg('mp3_mpeg25.mp3', 8000, 1, 0.4, ['-c:a', 'libmp3lame', '-b:a', '16k'], 'mp3', seed=53)
    # Corrupted into 8 kHz mixed blocks, which dr_mp3 reorders past its buffer.
    write('mp3_mpeg25_mixed.mp3', corrupt(mpeg25, 77))
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


class BitWriter:
    """An LSb-first bit packer, as libogg's oggpack_write()."""

    def __init__(self):
        self.out = bytearray()
        self.bits = 0

    def write(self, value, bits):
        for i in range(bits):
            if self.bits % 8 == 0:
                self.out.append(0)
            if (value >> i) & 1:
                self.out[-1] |= 1 << (self.bits % 8)
            self.bits += 1

    def string(self, data):
        for b in data:
            self.write(b, 8)

    def data(self):
        return bytes(self.out)


def vq_float(v):
    """A Vorbis codebook's packed float (21-bit mantissa, biased exponent)."""
    sign = 0
    if v < 0:
        sign, v = 0x80000000, -v
    exp = math.floor(math.log(v, 2) + .001)
    mant = round(v * 2.0 ** (20 - exp))
    return sign | ((exp + 768) << 21) | mant


def vorbis_book(w, dim, lengths, maptype=0, ordered=False, q_min=0.0, q_delta=1.0, q_quant=1, seq=0, quant=()):
    """Write a Vorbis codebook into the setup header being written."""
    entries = len(lengths)
    w.write(0x564342, 24)
    w.write(dim, 16)
    w.write(entries, 24)
    if ordered:
        w.write(1, 1)
        w.write(lengths[0] - 1, 5)
        i, cur = 0, lengths[0]
        while i < entries:
            n = 0
            while i + n < entries and lengths[i + n] == cur:
                n += 1
            w.write(n, (entries - i).bit_length())
            i += n
            cur += 1
    else:
        w.write(0, 1)
        sparse = 0 in lengths
        w.write(int(sparse), 1)
        for n in lengths:
            if sparse:
                w.write(int(n > 0), 1)
                if n:
                    w.write(n - 1, 5)
            else:
                w.write(n - 1, 5)
    w.write(maptype, 4)
    if maptype:
        w.write(vq_float(q_min), 32)
        w.write(vq_float(q_delta), 32)
        w.write(q_quant - 1, 4)
        w.write(seq, 1)
        for q in quant:
            w.write(q, q_quant)


def vorbis_floor0_stream():
    """A crafted stereo Vorbis stream with floor 0 (which no encoder in use
    today writes), residues 0, 1 and 2, channel coupling and submaps, and
    codebooks of every kind (ordered and sparse lengths, value mappings 1 and
    2, a single-entry book); its audio packets are random bits, which decode
    to deterministic garbage through all of it."""
    rate, channels, serial = 22050, 2, 0x0f0f
    w = BitWriter()
    w.string(b'\x01vorbis')
    w.write(0, 32)
    w.write(channels, 8)
    w.write(rate, 32)
    w.write(0, 32)
    w.write(64000, 32)
    w.write(0, 32)
    w.write(8, 4)  # 256-sample short blocks
    w.write(11, 4)  # 2048-sample long blocks
    w.write(1, 1)
    ident = w.data()
    w = BitWriter()
    vendor = b'sdl3-mixer floor 0'
    comments = [b'TITLE=Floor 0', b'LOOP_START=300', b'LOOP-LENGTH=4000']
    w.string(b'\x03vorbis')
    w.write(len(vendor), 32)
    w.string(vendor)
    w.write(len(comments), 32)
    for c in comments:
        w.write(len(c), 32)
        w.string(c)
    w.write(1, 1)
    comment = w.data()
    w = BitWriter()
    w.string(b'\x05vorbis')
    w.write(5 - 1, 8)
    # 0: floor 0's LSP book: ordered lengths, value mapping 1, sequenced
    vorbis_book(w, 4, [4] * 16, maptype=1, ordered=True, q_min=0.25, q_delta=0.05, q_quant=2, seq=1, quant=(0, 1))
    # 1: the residues' partition book
    vorbis_book(w, 2, [2, 2, 2, 2])
    # 2: a sparse book, value mapping 1
    vorbis_book(w, 2, [2, 0, 3, 3, 2, 0, 3, 3, 0], maptype=1, q_min=-0.01, q_delta=0.01, q_quant=2, quant=(0, 1, 2))
    # 3: ordered lengths, value mapping 2
    vorbis_book(w, 1, [3] * 8, maptype=2, ordered=True, q_min=-0.005, q_delta=0.00125, q_quant=4,
                quant=(3, 9, 0, 15, 7, 1, 12, 5))
    # 4: a single-entry book
    vorbis_book(w, 2, [1], maptype=1, q_min=0.0003125, q_delta=1.0, q_quant=1, quant=(0,))
    w.write(0, 6)  # one time backend
    w.write(0, 16)
    w.write(1 - 1, 6)  # one floor, of type 0
    w.write(0, 16)
    w.write(10, 8)  # order
    w.write(rate, 16)
    w.write(256, 16)  # bark map size
    w.write(6, 6)  # amplitude bits
    w.write(1, 8)  # amplitude offset (dB)
    w.write(1 - 1, 4)
    w.write(0, 8)
    w.write(3 - 1, 6)  # three residues, of the three types
    for rtype, begin, end in ((0, 0, 1024), (1, 16, 1000), (2, 0, 2048)):
        w.write(rtype, 16)
        w.write(begin, 24)
        w.write(end, 24)
        w.write(16 - 1, 24)  # grouping
        w.write(2 - 1, 6)  # partitions
        w.write(1, 8)  # partition book
        w.write(0, 3)  # partition 0: no stages
        w.write(0, 1)
        w.write(7, 3)  # partition 1: three stages
        w.write(0, 1)
        for book in (2, 3, 4):
            w.write(book, 8)
    w.write(3 - 1, 6)  # three mappings
    # 0: coupled, residue 0
    w.write(0, 16)
    w.write(0, 1)
    w.write(1, 1)
    w.write(0, 8)
    w.write(0, 1)
    w.write(1, 1)
    w.write(0, 2)
    w.write(0, 8)
    w.write(0, 8)
    w.write(0, 8)
    # 1: two submaps, residues 1 and 2
    w.write(0, 16)
    w.write(1, 1)
    w.write(2 - 1, 4)
    w.write(0, 1)
    w.write(0, 2)
    w.write(0, 4)
    w.write(1, 4)
    for res in (1, 2):
        w.write(0, 8)
        w.write(0, 8)
        w.write(res, 8)
    # 2: coupled the other way, residue 2
    w.write(0, 16)
    w.write(0, 1)
    w.write(1, 1)
    w.write(0, 8)
    w.write(1, 1)
    w.write(0, 1)
    w.write(0, 2)
    w.write(0, 8)
    w.write(0, 8)
    w.write(2, 8)
    w.write(3 - 1, 6)  # three modes
    for blockflag, mapping in ((0, 0), (1, 1), (1, 2)):
        w.write(blockflag, 1)
        w.write(0, 16)
        w.write(0, 16)
        w.write(mapping, 8)
    w.write(1, 1)
    setup = w.data()

    rng = Lcg(0x0f0f)
    out = ogg_page(serial, 0, 0, 2, [len(ident)], ident)
    lacing = []
    for p in (comment, setup):
        lacing += [255] * (len(p) // 255) + [len(p) % 255]
    out += ogg_page(serial, 1, 0, 0, lacing, comment + setup)
    blocksizes = (256, 2048)
    modes = [0, 0, 1, 2, 2, 1, 0, 1, 2, 0, 0, 0, 1, 1, 2, 0, 2, 1, 0, 2, 1, 1, 0, 2]
    gp, last, seq = 0, None, 2
    packets = []
    for mode in modes:
        n = 24 + (rng.next() >> 24) % 96
        pkt = bytearray(rng.next() >> 24 for _ in range(n))
        pkt[0] = (pkt[0] & ~7) | (mode << 1)
        bs = blocksizes[1 if mode else 0]
        if last is not None:
            gp += last // 4 + bs // 4
        last = bs
        packets.append((bytes(pkt), gp))
    for start in range(0, len(packets), 4):
        page = packets[start:start + 4]
        eos = start + 4 >= len(packets)
        out += ogg_page(serial, seq, page[-1][1], 4 if eos else 0, [len(p) for p, _ in page],
                        b''.join(p for p, _ in page))
        seq += 1
    return out


def ogg_pages(data):
    """The pages of an Ogg file."""
    pages, p = [], 0
    while p + 27 <= len(data):
        n = 27 + data[p + 26]
        n += sum(data[p + 27:p + n])
        pages.append(data[p:p + n])
        p += n
    return pages


def reserial(data, serial):
    """An Ogg stream's pages with another serial number (checksums fixed)."""
    out = b''
    for page in ogg_pages(data):
        page = bytearray(page)
        page[14:18] = struct.pack('<I', serial)
        page[22:26] = b'\0\0\0\0'
        page[22:26] = struct.pack('<I', ogg_crc(bytes(page)))
        out += page
    return out


def vorbis_files():
    vorbis = ['-c:a', 'libvorbis']
    ffmpeg('ogg_q10_48k.ogg', 48000, 2, 0.08, vorbis + ['-q:a', '10'], 'ogg', seed=64)
    ffmpeg('ogg_abr_mono.ogg', 44100, 1, 0.25, vorbis + ['-b:a', '48k'], 'ogg', seed=65,
           metadata=(('TITLE', 'Vorbis ABR'), ('LOOPSTART', '2000'), ('LOOPEND', '5000')))
    ffmpeg('ogg_3ch.ogg', 16000, 3, 0.15, vorbis + ['-q:a', '3'], 'ogg', seed=66)
    ffmpeg('ogg_8ch.ogg', 11025, 8, 0.1, vorbis + ['-q:a', '0'], 'ogg', seed=67)
    # a stream short enough for its only audio page to be the first and the last
    ffmpeg('ogg_short.ogg', 8000, 1, 0.03, vorbis + ['-q:a', '0'], 'ogg', seed=68)
    # A chained file: four links with different rates and channel counts
    # (the 3- and 6-channel ones take the decoder's channel maps).
    chain = [ffmpeg('', 22050, 2, 0.08, vorbis + ['-q:a', '1', '-serial_offset', '11'], 'ogg', seed=69, save=False),
             ffmpeg('', 16000, 1, 0.08, vorbis + ['-q:a', '0', '-serial_offset', '22'], 'ogg', seed=70, save=False,
                    metadata=(('TITLE', 'Second link'),)),
             ffmpeg('', 11025, 3, 0.08, vorbis + ['-q:a', '2', '-serial_offset', '33'], 'ogg', seed=71, save=False),
             ffmpeg('', 11025, 6, 0.08, vorbis + ['-q:a', '1', '-serial_offset', '44'], 'ogg', seed=72, save=False)]
    write('ogg_chained.ogg', b''.join(chain))
    # two links with junk between them
    junk = bytes(Lcg(73).next() >> 24 for _ in range(300))
    write('ogg_chained_junk.ogg', chain[1] + junk + reserial(chain[0], 55))
    # A Vorbis stream multiplexed with an Opus one (whose BOS page comes first).
    opus = ogg_pages(ffmpeg('', 48000, 1, 0.15, ['-c:a', 'libopus', '-b:a', '24k', '-serial_offset', '66'], 'ogg',
                            seed=74, save=False))
    vorb = ogg_pages(ffmpeg('', 22050, 1, 0.15, vorbis + ['-q:a', '0', '-serial_offset', '77'], 'ogg', seed=75,
                            save=False))
    mux = [opus[0], vorb[0]]
    rest_o, rest_v = opus[1:], vorb[1:]
    while rest_o or rest_v:
        if rest_v:
            mux.append(rest_v.pop(0))
        if rest_o:
            mux.append(rest_o.pop(0))
    write('ogg_mux.ogg', b''.join(mux))
    # a lost page
    pages = ogg_pages(ffmpeg('', 22050, 2, 0.3, vorbis + ['-q:a', '1'], 'ogg', seed=76, save=False))
    write('ogg_hole.ogg', b''.join(pages[:4] + pages[5:]))
    write('ogg_floor0.ogg', vorbis_floor0_stream())

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


def varied(freq, channels, seconds, seed=1):
    """The test signal with noisy, quiet and buzzing stretches, which make
    libopus switch between its SILK, hybrid and CELT modes."""
    base = signal(freq, channels, seconds, seed)
    rng = Lcg(seed + 1000)
    out = []
    for i in range(len(base) // channels):
        seg = int(i / (freq * 0.15)) % 4
        for c in range(channels):
            v = base[i * channels + c]
            if seg == 1:
                v = 0.6 * rng.unit()
            elif seg == 2:
                v *= 0.02
            elif seg == 3:
                v = 0.5 * math.sin(2 * math.pi * 150 * i / freq) * (1 + 0.5 * math.sin(2 * math.pi * 4 * i / freq))
            out.append(v)
    return out


def ogg_crc(data):
    crc = 0
    for b in data:
        crc ^= b << 24
        for _ in range(8):
            crc = ((crc << 1) ^ 0x04C11DB7) & 0xFFFFFFFF if crc & 0x80000000 else (crc << 1) & 0xFFFFFFFF
    return crc


def ogg_page(serial, seq, gp, flags, lacing, body):
    hdr = struct.pack('<4sBBqIII', b'OggS', 0, flags, gp, serial, seq, 0) + bytes([len(lacing)]) + bytes(lacing)
    return hdr[:22] + struct.pack('<I', ogg_crc(hdr + body)) + hdr[26:] + body


def ogg_packets(data):
    """The packets of an Ogg file, in order (of whatever streams)."""
    packets, pkt, p = [], b'', 0
    while p + 27 <= len(data) and data[p:p + 4] == b'OggS':
        lacing = data[p + 27:p + 27 + data[p + 26]]
        q = p + 27 + len(lacing)
        for n in lacing:
            pkt += data[q:q + n]
            q += n
            if n < 255:
                packets.append(pkt)
                pkt = b''
        p = q
    return packets


def opus_duration(pkt):
    """An Opus packet's duration, in 48 kHz samples."""
    cfg = pkt[0] >> 3
    if cfg < 12:
        size = [480, 960, 1920, 2880][cfg & 3]
    elif cfg < 16:
        size = [480, 960][cfg & 1]
    else:
        size = [120, 240, 480, 960][cfg & 3]
    code = pkt[0] & 3
    return size * (1 if code == 0 else 2 if code < 3 else pkt[1] & 0x3F)


def opus_tags(comments):
    out = b'OpusTags' + struct.pack('<I', 15) + b'sdl3-mixer test' + struct.pack('<I', len(comments))
    for c in comments:
        out += struct.pack('<I', len(c)) + c.encode('utf-8')
    return out


def ogg_opus(head, tags, packets, serial=0x0505, max_segments=7, drop_page=None):
    """An Ogg Opus stream of the packets, `max_segments` lacing values a
    page (so the longer packets continue on the next page), with page
    `drop_page` (counting the audio pages) left out as if lost."""
    pre_skip = struct.unpack('<H', head[10:12])[0]
    out = ogg_page(serial, 0, 0, 2, [len(head)], head) + ogg_page(serial, 1, 0, 0, [len(tags)], tags)
    segs = []  # (lacing value, data, granule position if it ends a packet)
    gp = pre_skip
    for pkt in packets:
        gp += opus_duration(pkt)
        n = len(pkt)
        for k in range(n // 255 + 1):
            last = k == n // 255
            segs.append((n - 255 * k if last else 255, pkt[255 * k:255 * (k + 1)], gp if last else None))
    seq, page = 2, 0
    continued = False
    for start in range(0, len(segs), max_segments):
        chunk_ = segs[start:start + max_segments]
        ends = [g for (_, _, g) in chunk_ if g is not None]
        eos = start + max_segments >= len(segs)
        flags = (1 if continued else 0) | (4 if eos else 0)
        if page != drop_page:
            out += ogg_page(serial, seq, ends[-1] if ends else -1, flags, [n for (n, _, _) in chunk_],
                            b''.join(d for (_, d, _) in chunk_))
        continued = chunk_[-1][2] is None
        seq += 1
        page += 1
    return out


def patch_head(data, offset, value):
    """`data` (an Ogg Opus file) with bytes changed in its first page's
    packet (the OpusHead), the page checksum fixed."""
    out = bytearray(data)
    hdr = 27 + out[26]
    out[hdr + offset:hdr + offset + len(value)] = value
    body = sum(out[27:hdr])
    out[22:26] = b'\0\0\0\0'
    out[22:26] = struct.pack('<I', ogg_crc(bytes(out[:hdr + body])))
    return bytes(out)


def opus_files():
    opus = ['-c:a', 'libopus', '-page_duration', '100000']
    ffmpeg('opus_silk_wb.opus', 16000, 1, 0.3, opus + ['-application', 'voip', '-b:a', '12k'], 'ogg', seed=80,
           metadata=(('TITLE', 'Opus test'), ('ARTIST', 'sdl3-mixer'), ('ALBUM', 'Test data'),
                     ('R128_TRACK_GAIN', '-256'), ('R128_ALBUM_GAIN', '+512')))
    ffmpeg('opus_silk_nb_60ms.opus', 8000, 1, 0.5, opus + ['-application', 'voip', '-b:a', '6k', '-frame_duration', '60',
                                                         '-fec', '1', '-packet_loss', '25'], 'ogg', seed=81)
    ffmpeg('opus_silk_mb_stereo.opus', 12000, 2, 0.3, opus + ['-application', 'voip', '-b:a', '16k', '-frame_duration',
                                                            '40'], 'ogg', seed=82)
    ffmpeg('opus_hybrid_stereo.opus', 48000, 2, 0.4, opus + ['-application', 'audio', '-b:a', '24k'], 'ogg', seed=83,
           metadata=(('TITLE', 'Opus loop'), ('LOOPSTART', '4800'), ('LOOPLENGTH', '9600')))
    ffmpeg('opus_hybrid_10ms.opus', 24000, 1, 0.3, opus + ['-application', 'audio', '-b:a', '20k', '-frame_duration',
                                                         '10'], 'ogg', seed=84)
    ffmpeg('opus_celt_44k.opus', 44100, 2, 0.25, opus + ['-application', 'lowdelay', '-b:a', '96k', '-frame_duration',
                                                       '10'], 'ogg', seed=85)
    ffmpeg('opus_celt_2_5ms.opus', 48000, 1, 0.15, opus + ['-application', 'lowdelay', '-b:a', '64k', '-frame_duration',
                                                         '2.5'], 'ogg', seed=86)
    ffmpeg('opus_celt_nb_5ms.opus', 8000, 1, 0.2, opus + ['-application', 'lowdelay', '-b:a', '16k', '-frame_duration',
                                                        '5'], 'ogg', seed=87)
    # The encoder switches modes as the signal changes (and codes the
    # transitions' redundant CELT frames).
    ffmpeg('opus_switch_silk_celt.opus', 16000, 1, 1.2, opus + ['-application', 'audio', '-b:a', '16k'], 'ogg',
           seed=88, sig=varied)
    ffmpeg('opus_switch_hybrid_celt.opus', 24000, 2, 0.9, opus + ['-application', 'audio', '-b:a', '28k'], 'ogg',
           seed=89, sig=varied)
    ffmpeg('opus_6ch.opus', 48000, 6, 0.2, opus + ['-b:a', '160k'], 'ogg', seed=90)
    ffmpeg('opus_quad.opus', 48000, 4, 0.2, opus + ['-ch_layout', 'quad', '-b:a', '96k'], 'ogg', seed=91)

    # Packets of different modes, frame sizes and channel counts spliced
    # into one stream, which the decoder takes as mode transitions.
    def part(freq, channels, seconds, args, seed):
        return [p for p in ogg_packets(ffmpeg('', freq, channels, seconds, opus + args, 'ogg', seed=seed, save=False))
                if p[:8] not in (b'OpusHead', b'OpusTags')]
    silk = part(16000, 1, 0.3, ['-application', 'voip', '-b:a', '12k'], 92)
    celt = part(48000, 1, 0.1, ['-application', 'lowdelay', '-b:a', '128k', '-frame_duration', '20'], 93)
    hybrid = part(48000, 1, 0.2, ['-application', 'audio', '-b:a', '24k'], 94)
    celt_short = part(48000, 1, 0.05, ['-application', 'lowdelay', '-b:a', '64k', '-frame_duration', '2.5'], 95)
    silk60 = part(8000, 1, 0.2, ['-application', 'voip', '-b:a', '6k', '-frame_duration', '60'], 96)
    hybrid10 = part(24000, 1, 0.1, ['-application', 'audio', '-b:a', '20k', '-frame_duration', '10'], 97)
    stereo_silk = part(16000, 2, 0.1, ['-application', 'voip', '-b:a', '20k'], 98)
    stereo_celt = part(48000, 2, 0.1, ['-application', 'lowdelay', '-b:a', '96k', '-frame_duration', '10'], 99)
    spliced = (silk[:6] + celt[:5] + silk[6:10] + hybrid[:5] + silk[10:14] + celt_short[:20] + hybrid[5:10]
               + silk60[:3] + hybrid10[:10] + stereo_silk[:4] + celt[5:] + stereo_celt[:6] + hybrid[10:])
    head = ogg_packets(ffmpeg('', 48000, 1, 0.05, opus, 'ogg', seed=100, save=False))[0]
    tags = opus_tags(['TITLE=Opus spliced', 'LOOPSTART=2000', 'LOOPEND=12000'])
    # with a -6 dB output gain in the header
    write('opus_spliced_mono.opus', ogg_opus(head[:16] + struct.pack('<h', -1536) + head[18:], tags, spliced))
    head2 = head[:9] + b'\x02' + head[10:]
    spliced2 = (stereo_silk[:4] + silk[:5] + stereo_celt[:6] + celt[:4] + hybrid[:6] + stereo_silk[4:] + celt_short[:12]
                + stereo_celt[6:])
    write('opus_spliced_stereo.opus', ogg_opus(head2, opus_tags([]), spliced2, max_segments=5))
    # a lost page
    write('opus_hole.opus', ogg_opus(head2, opus_tags([]), spliced2, max_segments=5, drop_page=3))

    # A chained file: three links with different channel counts.
    chain = ffmpeg('', 48000, 2, 0.2, opus + ['-b:a', '64k', '-serial_offset', '11'], 'ogg', seed=101, save=False)
    chain += ffmpeg('', 16000, 1, 0.25, opus + ['-application', 'voip', '-b:a', '10k', '-serial_offset', '22'], 'ogg',
                    seed=102, save=False, metadata=(('TITLE', 'Second link'),))
    chain += ffmpeg('', 48000, 6, 0.15, opus + ['-b:a', '128k', '-serial_offset', '33'], 'ogg', seed=103, save=False)
    write('opus_chained.opus', chain)

    # Channel mapping family 255, which opusfile doesn't play.
    plain = ffmpeg('', 48000, 2, 0.1, opus + ['-b:a', '48k'], 'ogg', seed=104, save=False)
    write('opus_family255.opus', patch_head(plain, 18, b'\xff'))



def wav_header(freq, channels, bits, nbytes, fmt=1, mask=None, valid=None):
    """A RIFF/WAVE header for PCM (fmt 1) or IEEE float (fmt 3) data,
    WAVE_FORMAT_EXTENSIBLE when there's a channel mask or valid bits."""
    block = channels * ((bits + 7) // 8)
    if mask is None and valid is None:
        fmtck = struct.pack('<HHIIHH', fmt, channels, freq, freq * block, block, bits)
    else:
        guid = struct.pack('<H', fmt) + b'\x00\x00\x00\x00\x10\x00\x80\x00\x00\xaa\x00\x38\x9b\x71'
        fmtck = struct.pack('<HHIIHHHHI', 0xFFFE, channels, freq, freq * block, block, bits, 22,
                            valid or bits, mask or 0) + guid
    body = b'WAVE' + chunk(b'fmt ', fmtck) + b'data' + struct.pack('<I', nbytes)
    return b'RIFF' + struct.pack('<I', len(body) + nbytes + (nbytes & 1)) + body


def correlated(freq, channels, seconds, seed=1):
    """The test signal's first channel in every channel, a little louder or
    quieter and with a little noise of its own, which WavPack's
    cross-channel decorrelation terms are for."""
    base = signal(freq, 1, seconds, seed)
    rng = Lcg(seed + 2000)
    return [base[i] * (0.95 - 0.2 * c) + 0.003 * rng.unit() for i in range(len(base)) for c in range(channels)]


def identical(freq, channels, seconds, seed=1):
    """The same samples in every channel."""
    return [v for v in signal(freq, 1, seconds, seed) for _ in range(channels)]


def sparse(freq, channels, seconds, seed=1):
    """Silence, with a few short bursts of the test signal and loud clicks."""
    base = signal(freq, channels, seconds, seed)
    rng = Lcg(seed + 3000)
    out = []
    n = len(base) // channels
    for i in range(n):
        burst = (i // (freq // 20)) % 5 == 2
        click = rng.next() % 997 == 0
        for c in range(channels):
            out.append(base[i * channels + c] if burst else (0.9 if click else 0.0))
    return out


def pcm(samples, bits, rng=None, zeros=0, low=None):
    """Samples in [-1, 1] as little-endian PCM (8-bit unsigned, or signed),
    with random low bits from `rng` for 32-bit data and `zeros` low bits
    cleared; or, for 32-bit data, the low byte all ones (`low='ones'`), or
    all copies of the bit above it (`'dups'`), or the samples rounded to
    floats and scaled up (`'float'`)."""
    out = bytearray()
    for s in samples:
        if bits == 8:
            out.append(int(round(s * 127)) + 128)
            continue
        full = (1 << (bits - 1)) - 1
        if low == 'float':
            v = int(struct.unpack('<f', struct.pack('<f', s))[0] * 2147483648.0)
            v = max(-2147483648, min(2147483647, v))
        elif bits == 32:
            v = int(round(s * (full >> 8))) << 8
            if low == 'ones':
                v |= 0xff
            elif low == 'dups':
                v |= 0xff if v & 0x100 else 0
            elif rng:
                v |= rng.next() >> 24
        else:
            v = int(round(s * full))
        v &= ~((1 << zeros) - 1)
        out += (v & ((1 << bits) - 1)).to_bytes(bits // 8, 'little')
    return bytes(out)


def floats(samples, special=False, shifted=None):
    """IEEE float data, with some zeros, negative zeros, denormals and
    values past +/-1 among the samples if `special`, or infinities if it's
    `'inf'`; with `shifted`, the samples (kept under 1) have the low
    mantissa bits that WavPack shifts out set to ones (`'ones'`) or, every
    other sample, to ones or zeros (`'same'`)."""
    out = bytearray()
    extra = [0.0, -0.0, 1.5e-42, -3.0e-41, 2.75, -3.5, 1.0, -1.0]
    for i, s in enumerate(samples):
        if special == 'inf' and i % 101 == 50:
            s = math.inf if (i // 101) % 2 else -math.inf
        elif special and special != 'inf' and i % 37 < len(extra) and (i // 37) % 3 == 1:
            s = extra[i % 37]
        if shifted:
            bits = struct.unpack('<I', struct.pack('<f', s * 0.98))[0]
            shift = 126 - ((bits >> 23) & 0xff)
            if 0 < shift < 23:
                mask = (1 << shift) - 1
                if shifted == 'ones' or i % 2:
                    bits |= mask
                else:
                    bits &= ~mask
            out += struct.pack('<I', bits)
        else:
            out += struct.pack('<f', s)
    return bytes(out)


WAVPACK = os.environ.get('WAVPACK', 'wavpack')


def wavpack(name, wav, args, src='src.wav'):
    """Encode `wav` (a source file's bytes) with WavPack's command-line
    encoder; writes `name` (and its .wvc, with -c) unless it's empty, and
    returns the data of both."""
    with tempfile.TemporaryDirectory() as tmp:
        with open(os.path.join(tmp, src), 'wb') as f:
            f.write(wav)
        dst = os.path.join(tmp, 'out.wv')
        subprocess.run([WAVPACK, '-q', '-y', '--no-threads'] + list(args) + [os.path.join(tmp, src), '-o', dst],
                       check=True)
        with open(dst, 'rb') as f:
            out = f.read()
        wvc = None
        if os.path.exists(dst + 'c'):
            with open(dst + 'c', 'rb') as f:
                wvc = f.read()
    if name:
        write(name, out)
        if wvc is not None:
            write(name + 'c', wvc)
    return out, wvc


def wv_pcm(name, freq, channels, seconds, bits, args, seed, mask=None, valid=None, zeros=0, sig=None, low=None):
    samples = (sig or signal)(freq, channels, seconds, seed)
    data = pcm(samples, bits, Lcg(seed + 500), zeros, low)
    pad = b'\0' if len(data) & 1 else b''
    return wavpack(name, wav_header(freq, channels, bits, len(data), 1, mask, valid) + data + pad, args)


def wv_float(name, freq, channels, seconds, args, seed, special=False, shifted=None):
    data = floats(signal(freq, channels, seconds, seed), special, shifted)
    return wavpack(name, wav_header(freq, channels, 32, len(data), 3) + data, args)


def dsf(channels, seconds, seed, noise=False, same=False):
    """A Sony DSF file (2.8224 MHz 1-bit DSD, LSB first) of the test signal
    through a second-order sigma-delta modulator, or of random bits; with
    `same`, every channel has the first one's bits."""
    rate = 2822400
    nbits = int(rate * seconds) // 8 * 8
    pcm_rate = 44100
    base = signal(pcm_rate, channels, seconds + 0.01, seed)
    rng = Lcg(seed)
    chans = []
    for c in range(channels):
        if same and c:
            chans.append(chans[0])
            continue
        bits = bytearray(nbits // 8)
        i1 = i2 = 0.0
        y = 1.0
        for n in range(nbits):
            if noise:
                b = rng.next() >> 31
            else:
                x = 0.5 * base[(n * pcm_rate // rate) * channels + c]
                i1 += x - y
                i2 += i1 - y
                b = 1 if i2 >= 0 else 0
                y = 1.0 if b else -1.0
            if b:
                bits[n >> 3] |= 1 << (n & 7)
        chans.append(bytes(bits))
    block = 4096
    nblocks = (nbits // 8 + block - 1) // block
    data = b''
    for k in range(nblocks):
        for c in range(channels):
            data += chans[c][k * block:(k + 1) * block].ljust(block, b'\0')
    chantype = {1: 1, 2: 2, 3: 3, 4: 4, 5: 6, 6: 7}[channels]
    fmtck = b'fmt ' + struct.pack('<qIIIIIIqII', 52, 1, 0, chantype, channels, rate, 1, nbits, block, 0)
    datack = b'data' + struct.pack('<q', 12 + len(data)) + data
    total = 28 + len(fmtck) + len(datack)
    return b'DSD ' + struct.pack('<qqq', 28, total, 0) + fmtck + datack


def wv_blocks(data):
    """The WavPack blocks of a file: (offset, length) pairs."""
    out, at = [], 0
    while at + 32 <= len(data) and data[at:at + 4] == b'wvpk':
        size = struct.unpack_from('<I', data, at + 4)[0] + 8
        out.append((at, size))
        at += size
    return out


def wv_checksum(block):
    """`block` with its block checksum (ID_BLOCK_CHECKSUM metadata) redone,
    as WavpackVerifySingleBlock() checks it."""
    out = bytearray(block)
    at = 32
    while at + 2 <= len(out):
        mid, n = out[at], out[at + 1] << 1
        hdr = 2
        if mid & 0x80:
            n += (out[at + 2] << 9) + (out[at + 3] << 17)
            hdr = 4
        if (mid & 0x3f) == 0x2f:
            csum = 0xFFFFFFFF
            for k in range(0, at, 2):
                csum = (csum * 3 + out[k] + (out[k + 1] << 8)) & 0xFFFFFFFF
            if n == 4:
                out[at + hdr:at + hdr + 4] = struct.pack('<I', csum)
            else:
                csum ^= csum >> 16
                out[at + hdr:at + hdr + 2] = struct.pack('<H', csum & 0xFFFF)
        at += hdr + n
    return bytes(out)


def wv_renumber(data, first, total):
    """`data`'s blocks with their block indexes moved up by `first` and the
    total sample count set to `total` (0xFFFFFFFF for unknown), checksums
    redone."""
    out = b''
    for at, size in wv_blocks(data):
        block = bytearray(data[at:at + size])
        index = struct.unpack_from('<I', block, 16)[0] + (block[10] << 32) + first
        struct.pack_into('<I', block, 16, index & 0xFFFFFFFF)
        block[10] = index >> 32
        struct.pack_into('<I', block, 12, total)
        block[11] = 0
        out += wv_checksum(bytes(block))
    return out


def wv_samples(data):
    """The sample count in the first block's header."""
    return struct.unpack_from('<I', data, 12)[0]


def wv_splice(parts, gaps=None, start=0, total=None):
    """The blocks of the encoded `parts` one after the other (with `gaps`
    samples missing before each), all numbered from `start`, with `total`
    (or the sum) as the sample count."""
    gaps = gaps or [0] * len(parts)
    if total is None:
        total = sum(wv_samples(p) for p in parts) + sum(gaps)
    out, first = b'', start
    for p, gap in zip(parts, gaps):
        first += gap
        out += wv_renumber(p, first, total)
        first += wv_samples(p)
    return out


def wavpack_files():
    # Lossless, in each bit depth, and the encoder's modes.
    wv_pcm('wv_s16_long.wv', 22050, 1, 0.62, 16, ['--blocksize=4096', '-m'], seed=110)
    wv_pcm('wv_s16_stereo.wv', 11025, 2, 0.2, 16, [], seed=109)
    wv_pcm('wv_u8_mono.wv', 8000, 1, 0.3, 8, ['-f'], seed=111)
    wv_pcm('wv_s24_mono.wv', 11025, 1, 0.3, 24, ['-hh', '-x6'], seed=112)
    wv_pcm('wv_s32_stereo.wv', 8000, 2, 0.25, 32, ['-h', '-x3'], seed=113)
    wv_pcm('wv_s32_zeros.wv', 8000, 1, 0.25, 32, ['-x1'], seed=114, zeros=7)
    wv_pcm('wv_s32_int32.wv', 8000, 1, 0.25, 32, ['--optimize-int32', '-x4'], seed=115, low='float')
    wv_pcm('wv_s32_ones.wv', 8000, 2, 0.15, 32, [], seed=140, low='ones')
    wv_pcm('wv_s32_dups.wv', 8000, 1, 0.15, 32, ['-h'], seed=141, low='dups')
    wv_pcm('wv_s12_stereo.wv', 11025, 2, 0.25, 16, ['-j0', '-f', '-x2'], seed=116, valid=12, zeros=4)
    wv_float('wv_f32_stereo.wv', 11025, 2, 0.25, ['-h'], seed=117, special=True)
    wv_float('wv_f32_mono.wv', 16000, 1, 0.25, ['-f', '-x5'], seed=118)
    wv_float('wv_f32_ones.wv', 8000, 1, 0.15, [], seed=142, shifted='ones')
    wv_float('wv_f32_same.wv', 8000, 2, 0.15, [], seed=143, shifted='same')
    wv_float('wv_f32_inf.wv', 8000, 1, 0.1, [], seed=144, special='inf')
    wv_pcm('wv_s16_raw.wv', 16000, 1, 0.3, 16, ['-r', '--blocksize=1000', '-j1'], seed=119)
    wv_pcm('wv_correlated.wv', 11025, 2, 0.25, 16, ['-hh', '-x6'], seed=145, sig=correlated)
    wv_pcm('wv_false_stereo.wv', 8000, 2, 0.2, 16, [], seed=146, sig=identical)
    wv_pcm('wv_silence.wv', 8000, 1, 0.5, 16, [], seed=147, sig=sparse)
    wv_pcm('wv_37800.wv', 37800, 1, 0.08, 16, [], seed=148)

    # Hybrid: lossy, and lossless with a correction file.
    wv_pcm('wv_hybrid_lossy.wv', 22050, 2, 0.6, 16, ['-b3', '-s0.5', '--blocksize=2048'], seed=120)
    wv_pcm('wv_hybrid.wv', 22050, 2, 0.6, 16, ['-b3.5', '-c', '--blocksize=3000', '-j1'], seed=121)
    wv_pcm('wv_hybrid_cc_s24.wv', 11025, 1, 0.3, 24, ['-b4', '-cc', '-h', '--use-dns'], seed=122)
    wv_pcm('wv_hybrid_s8.wv', 8000, 2, 0.3, 8, ['-b2.5', '-c', '-s-0.5', '--cross-decorr'], seed=123)
    wv_pcm('wv_hybrid_kbps.wv', 16000, 1, 0.3, 16, ['-b48', '-x3', '-hh'], seed=124)
    wv_pcm('wv_hybrid_s32.wv', 8000, 2, 0.25, 32, ['-b5', '-c', '-f'], seed=125)
    wv_pcm('wv_hybrid_s32_ones.wv', 8000, 1, 0.15, 32, ['-b4'], seed=149, low='ones')
    wv_float('wv_hybrid_f32.wv', 11025, 1, 0.25, ['-b3', '-c'], seed=126)
    wv_pcm('wv_hybrid_xdecorr.wv', 11025, 2, 0.25, 16, ['-b3', '-c', '-hh', '--cross-decorr', '-x6'], seed=150,
           sig=correlated)
    wv_pcm('wv_hybrid_silence.wv', 8000, 2, 0.5, 16, ['-b6', '-c'], seed=151, sig=sparse)

    # Multichannel.
    wv_pcm('wv_6ch.wv', 8000, 6, 0.2, 16, ['-h', '--blocksize=512'], seed=127, mask=0x3F)
    wv_pcm('wv_quad_hybrid.wv', 8000, 4, 0.2, 16, ['-b3', '-c', '--blocksize=600'], seed=128, mask=0x33)
    wv_pcm('wv_3ch.wv', 8000, 3, 0.2, 24, ['--pair-unassigned-chans', '-x2'], seed=129, mask=0)
    wv_pcm('wv_10ch.wv', 8000, 10, 0.05, 8, [], seed=152, mask=0)
    wv_pcm('wv_reorder.wv', 8000, 4, 0.05, 16, ['--channel-order=FR,FL,...'], seed=153)

    # APEv2 tags (written by the encoder), an ID3v1 tag after them, and loop
    # tags (that only Ogg files honor); an ID3v2 tag before the blocks; one
    # of each alone at the end.
    tagged, _ = wv_pcm('', 11025, 2, 0.2, 16, ['-w', 'Title=WavPack test', '-w', 'Artist=sdl3-mixer',
                                                '-w', 'Album=Test data', '-w', 'Year=2004', '-w', 'Track=5/9',
                                                '-w', 'LOOPSTART=100', '-w', 'LOOPLENGTH=1000'], seed=130)
    id3v1 = b'TAG' + b'ID3v1 title'.ljust(30, b'\0') + b'ID3v1 artist'.ljust(30, b'\0') + \
        b'ID3v1 album'.ljust(30, b'\0') + b'1987' + b'comment'.ljust(28, b'\0') + b'\0\x05' + b'\x0c'
    write('wv_tags.wv', tagged + id3v1)
    plain, _ = wv_pcm('', 8000, 1, 0.2, 16, [], seed=131)
    write('wv_id3v2.wv', id3v2([(b'TIT2', 'ID3 before WavPack')]) + plain)
    write('wv_apetag.wv', plain + ape_tag([('Title', 'APE title'), ('Artist', 'APE artist')]))
    write('wv_id3v1.wv', plain + id3v1)

    # DSD, which SDL_mixer opens without OPEN_DSD_AS_PCM (so the library
    # refuses it), and DSD blocks (fast, high and raw, stereo, mono and
    # "false stereo") after PCM ones, and multichannel.
    fast, _ = wavpack('wv_dsd.wv', dsf(2, 0.003, 132), [], src='src.dsf')
    high, _ = wavpack('', dsf(2, 0.003, 133), ['-h'], src='src.dsf')
    fast1, _ = wavpack('', dsf(1, 0.002, 136), [], src='src.dsf')
    high1, _ = wavpack('', dsf(1, 0.002, 137), ['-h'], src='src.dsf')
    same, _ = wavpack('', dsf(2, 0.002, 138, same=True), [], src='src.dsf')
    raw, _ = wavpack('', dsf(2, 0.002, 134, noise=True), [], src='src.dsf')
    head, _ = wv_pcm('', 44100, 2, 0.02, 16, ['--blocksize=300'], seed=135)
    write('wv_dsd_spliced.wv', wv_splice([head, fast, high, fast1, high1, same, raw]))
    head3, _ = wv_pcm('', 44100, 3, 0.01, 16, [], seed=139, mask=0x7)
    dsd3, _ = wavpack('', dsf(3, 0.002, 139), [], src='src.dsf')
    write('wv_dsd_3ch.wv', wv_splice([head3, dsd3]))

    # Blocks that don't start at 0 (with the total unknown), and missing
    # blocks: a short gap, then a long one.
    parts = [wv_pcm('', 11025, 2, 0.05, 16, [], seed=154 + k)[0] for k in range(3)]
    write('wv_offset.wv', wv_splice(parts, start=50000, total=0xFFFFFFFF))
    six, _ = wv_pcm('', 8000, 6, 0.1, 16, ['--blocksize=256'], seed=157, mask=0x3F)
    write('wv_6ch_offset.wv', wv_splice([six], start=3000, total=0xFFFFFFFF))
    write('wv_gap.wv', wv_splice(parts, gaps=[0, 1000, 300000]))


def main():
    os.makedirs(OUT, exist_ok=True)
    wav_files()
    aiff_files()
    au_files()
    voc_files()
    mp3_files()
    ogg_files()
    vorbis_files()
    flac_files()
    opus_files()
    wavpack_files()


if __name__ == '__main__':
    main()
