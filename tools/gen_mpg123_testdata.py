#!/usr/bin/env python3
"""Generate sdl3-mixer-mpg123's test audio (sdl3-mixer-mpg123/testdata/).

MPEG audio streams beyond the ones sdl3-mixer's tests use (which
`tools/gen_sdl_mixer_testdata.py` makes, with the helpers and the
synthetic test signal this borrows from it): every layer, MPEG version,
channel mode and the encoders' choices of tables and blocks, and the tags
and headers libmpg123 parses. The layer II and III streams come from
ffmpeg (with its mp2, libtwolame, libmp3lame and libshine encoders); the
layer I streams (there's no layer I encoder), the ID3v2 tags, the RIFF
header and the patched LAME tags and frame headers are put together here.

Encoders change between versions, so regenerating the files will likely
change them, and then the reference output (`testdata/reference.txt`, made
by a C program linked with SDL_mixer's libmpg123) must be regenerated too.

usage: tools/gen_mpg123_testdata.py [OUTDIR]
"""

import math
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gen_sdl_mixer_testdata as gen  # noqa: E402

OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
    os.path.dirname(os.path.abspath(__file__)), '..', 'sdl3-mixer-mpg123', 'testdata')
gen.OUT = OUT


def clicks(freq, channels, seconds, seed=1):
    """Silence broken by short loud bursts (for the encoders' short blocks)."""
    rng = gen.Lcg(seed)
    out = []
    for i in range(int(freq * seconds)):
        burst = (i % int(freq * 0.05)) < freq * 0.004
        for c in range(channels):
            out.append(0.8 * rng.unit() if burst else 0.002 * math.sin(i * 0.1 * (c + 1)))
    return out


def lame(freq, channels, seconds, args, seed, sig=None):
    return gen.ffmpeg('', freq, channels, seconds, ['-c:a', 'libmp3lame'] + args + ['-id3v2_version', '0'],
                      'mp3', seed=seed, sig=sig, save=False)


def syncsafe(n):
    return bytes([(n >> 21) & 0x7F, (n >> 14) & 0x7F, (n >> 7) & 0x7F, n & 0x7F])


def unsync(data):
    """ID3v2 unsynchronisation: a zero after each 0xFF that's followed by 0xE0 and up or a zero (or ends it)."""
    out = bytearray()
    for i, b in enumerate(data):
        out.append(b)
        if b == 0xFF and (i + 1 == len(data) or data[i + 1] >= 0xE0 or data[i + 1] == 0):
            out.append(0)
    return bytes(out)


def frame(version, fid, body, flags=0):
    """An ID3v2 frame (v2.2, 2.3 or 2.4)."""
    if version == 2:
        return fid + struct.pack('>I', len(body))[1:] + body
    size = syncsafe(len(body)) if version == 4 else struct.pack('>I', len(body))
    return fid + size + struct.pack('>H', flags) + body


def tag(version, frames, flags=0, ext=b'', padding=0, footer=False):
    """An ID3v2 tag of `frames` (with its extended header `ext`, padding and footer)."""
    body = ext + b''.join(frames) + bytes(padding)
    hdr = b'ID3' + bytes([version, 0, flags]) + syncsafe(len(body))
    return hdr + body + (b'3DI' + hdr[3:] if footer else b'')


def utf16(text):
    return b'\xff\xfe' + text.encode('utf-16-le')


def picture_data():
    rng = gen.Lcg(7)
    return b'\x89PNG\r\n\x1a\n' + bytes([0xFF, 0xE7, 0xFF, 0x00, 0xFF]) + bytes(rng.next() >> 24 for _ in range(40))


def id3v1(title, track=None, genre=12):
    comment = b'id3v1 comment'.ljust(28 if track else 30, b'\0') + (b'\0' + bytes([track]) if track else b'')
    return b'TAG' + title.ljust(30, b'\0') + b'An artist'.ljust(30, b'\0') + b'An album'.ljust(30, b'\0') + \
        b'1999' + comment + bytes([genre])


def lame_tag_gain(data, radio, audiophile):
    """`data` with the LAME tag's ReplayGain fields (after the encoder
    string, method, lowpass and peak) set."""
    at = max(data.find(b'Xing'), data.find(b'Info'))
    flags = struct.unpack('>I', data[at + 4:at + 8])[0]
    off = at + 8 + (4 if flags & 1 else 0) + (4 if flags & 2 else 0) + (100 if flags & 4 else 0) + \
        (4 if flags & 8 else 0)
    return data[:off + 15] + struct.pack('>HH', radio, audiophile) + data[off + 19:]


def gain_field(name, originator, db):
    return (name << 13) | (originator << 10) | ((1 if db < 0 else 0) << 9) | int(round(abs(db) * 10))


def riff_mp3(mp3, freq, channels):
    fmt = struct.pack('<HHIIHH', 0x55, channels, freq, 4000, 1, 0) + struct.pack('<H', 0)
    body = b'WAVE' + gen.chunk(b'fmt ', fmt) + gen.chunk(b'fact', struct.pack('<I', 1000)) + gen.chunk(b'data', mp3)
    return b'RIFF' + struct.pack('<I', len(body)) + body


def layer1_files():
    gen.write('mp1_mono.mp1', gen.mp1_stream(32000, 1, 0.2, 96, seed=200))
    gen.write('mp1_joint_crc.mp1', gen.mp1_stream(48000, 2, 0.15, 224, seed=201, mode=1, bound=8, crc=True))
    gen.write('mp1_lsf.mp1', gen.mp1_stream(22050, 2, 0.2, 128, seed=202, mode=1, bound=16))
    gen.write('mp1_dual.mp1', gen.mp1_stream(16000, 2, 0.2, 64, seed=203, mode=2))
    gen.write('mp1_free.mp1', gen.mp1_stream(44100, 1, 0.15, 0, seed=204, free=236))


def layer2_files():
    gen.ffmpeg('mp2_joint_crc.mp2', 44100, 2, 0.2, ['-c:a', 'libtwolame', '-b:a', '160k', '-mode', 'joint_stereo',
                                                    '-error_protection', '1'], 'mp2', seed=210)
    gen.ffmpeg('mp2_dual.mp2', 48000, 2, 0.15, ['-c:a', 'libtwolame', '-b:a', '192k', '-mode', 'dual_channel'],
               'mp2', seed=211)
    gen.ffmpeg('mp2_lsf_stereo.mp2', 16000, 2, 0.25, ['-c:a', 'mp2', '-b:a', '64k'], 'mp2', seed=212)
    # The two low-bitrate allocation tables.
    gen.ffmpeg('mp2_48k_low.mp2', 48000, 1, 0.15, ['-c:a', 'mp2', '-b:a', '32k'], 'mp2', seed=213)
    gen.ffmpeg('mp2_32k_low.mp2', 32000, 1, 0.2, ['-c:a', 'libtwolame', '-b:a', '48k', '-mode', 'mono'], 'mp2',
               seed=214)


def layer3_files():
    gen.ffmpeg('mp3_shine.mp3', 32000, 2, 0.2, ['-c:a', 'libshine', '-b:a', '64k'], 'mp3', seed=220)
    # Plain stereo with an Info tag, with the emphasis, copyright and
    # private bits set in a few frames.
    info = lame(48000, 2, 0.15, ['-b:a', '128k', '-joint_stereo', '0'], seed=221)
    gen.write('mp3_stereo_info.mp3', gen.patch_frames(
        info, lambda i, h: h[:2] + bytes([h[2] | (i == 3)]) + bytes([h[3] | (0x09 if i % 4 == 2 else 0)])))
    gen.write('mp3_short_blocks.mp3', lame(24000, 2, 0.2, ['-b:a', '48k'], seed=222, sig=clicks))
    gen.write('mp3_mpeg25_stereo.mp3', lame(11025, 2, 0.3, ['-b:a', '24k'], seed=223))
    gen.write('mp3_lsf_intensity.mp3', gen.intensity(lame(22050, 2, 0.2, ['-b:a', '40k', '-joint_stereo', '1'],
                                                          seed=224)))
    gen.write('mp3_abr.mp3', lame(44100, 1, 0.2, ['-b:a', '48k', '-abr', '1'], seed=225))
    vbr = lame(22050, 1, 0.25, ['-q:a', '6'], seed=226)
    gen.write('mp3_replaygain.mp3', lame_tag_gain(vbr, gain_field(1, 3, -5.3), gain_field(2, 1, 2.0)))
    gen.write('mp3_riff.wav', riff_mp3(lame(16000, 1, 0.2, ['-b:a', '24k', '-write_xing', '0'], seed=227), 16000, 1))


def tag_files():
    plain = lame(16000, 1, 0.15, ['-b:a', '24k'], seed=230)
    pic = picture_data()

    # ID3v2.2: text, comment, extra text and picture frames.
    v22 = [frame(2, b'TT2', b'\x00v2.2 title'), frame(2, b'TP1', b'\x01' + utf16('v2.2 artist')),
           frame(2, b'TAL', b'\x00v2.2 album'), frame(2, b'TYE', b'\x001998'), frame(2, b'TCO', b'\x00(17)Rock'),
           frame(2, b'COM', b'\x00eng\x00a v2.2 comment'), frame(2, b'TXX', b'\x00RVA_ALBUM\x00-3.5'),
           frame(2, b'PIC', b'\x00PNG\x03a picture\x00' + pic)]
    gen.write('mp3_id3v22_frames.mp3', tag(2, v22, padding=16) + plain)

    # ID3v2.3, unsynchronised (frame by frame, as libmpg123 reads it), with
    # an extended header: UTF-16 text, comments (one of them an RVA one),
    # ReplayGain extra text, lyrics, pictures, numeric genres and unknown
    # and compressed frames.
    v23 = [(b'TIT2', b'\x01' + utf16('v2.3 title é')), (b'TPE1', b'\x00v2.3 artist'),
           (b'TALB', b'\x02' + 'v2.3 album'.encode('utf-16-be')), (b'TYER', b'\x002003'),
           (b'TCON', b'\x00(13)(RX)'), (b'TRCK', b'\x003/9'),
           (b'COMM', b'\x00eng\x00the comment'), (b'COMM', b'\x00deurva\x00-2.25'),
           (b'COMM', b'\x01eng' + utf16('desc') + b'\0\0' + utf16('UTF-16 comment')),
           (b'TXXX', b'\x00replaygain_track_gain\x00-6.48 dB'),
           (b'TXXX', b'\x00replaygain_album_gain\x00-4.1 dB'),
           (b'TXXX', b'\x00replaygain_track_peak\x000.98'),
           (b'USLT', b'\x00eng\x00la la la'),
           (b'APIC', b'\x00image/png\x00\x03front\x00' + pic),
           (b'APIC', b'\x00-->\x00\x00http://example.invalid/'),
           (b'XYZW', b'unknown'), (b'TCOP', b'\x00compressed', 0x0080)]
    ext = struct.pack('>IHI', 6, 0, 32)
    v23 = [frame(3, f[0], unsync(f[1]), *f[2:]) for f in v23]
    gen.write('mp3_id3v23_unsync.mp3', tag(3, v23, flags=0xC0, ext=ext, padding=32) + plain)

    # ID3v2.4 with a footer: UTF-8 text, frames with their own
    # unsynchronisation, data length indicators and group bytes, RVA2,
    # TDRC, multiple values, an encrypted frame.
    rva2 = b'track\x00\x01' + struct.pack('>h', -7 * 512 // 2) + b'\x00'
    rva2_album = b'album\x00\x02\x00\x10\x00\x01' + struct.pack('>h', 3 * 512) + b'\x00'
    sync_body = b'\x00' + b'\xff\xe0 unsynced'
    dli = b'\x00data length title'
    v24 = [frame(4, b'TIT2', syncsafe(len(dli)) + dli, flags=0x0001),
           frame(4, b'TPE1', b'\x03v2.4 \xc3\xa4rtist\x00second artist'),
           frame(4, b'TALB', b'\x03v2.4 album'),
           frame(4, b'TDRC', b'\x032004-05-06'), frame(4, b'TCON', b'\x0313\x00Pop'),
           frame(4, b'TCOM', unsync(sync_body), flags=0x0002),
           frame(4, b'TIT3', b'\x05' + b'\x00group text', flags=0x0040),
           frame(4, b'RVA2', rva2), frame(4, b'RVA2', rva2_album),
           frame(4, b'COMM', b'\x03eng\x00v2.4 comment'),
           frame(4, b'TPE2', b'\x00secret', flags=0x0004)]
    gen.write('mp3_id3v24_footer.mp3', tag(4, v24, flags=0x10, footer=True) + plain)

    # Tags in the middle and at the end of the stream: an ID3v2 tag between
    # frames (and one after them), an APE tag and ID3v1.1.
    frames = gen.mpeg_frames(plain)
    cut = frames[4][0]
    mid = gen.id3v2([(b'TIT2', 'A tag in between')])
    gen.write('mp3_id3_mid.mp3', plain[:cut] + mid + plain[cut:] + gen.ape_tag([('Title', 'APE')]) +
              id3v1(b'v1.1 title', track=7))
    gen.write('mp3_id3v1_end.mp3', plain + tag(4, [frame(4, b'TIT2', b'\x00appended')], flags=0x10, footer=True) +
              id3v1(b'v1.0 title', genre=200))


def main():
    os.makedirs(OUT, exist_ok=True)
    layer1_files()
    layer2_files()
    layer3_files()
    tag_files()


if __name__ == '__main__':
    main()
