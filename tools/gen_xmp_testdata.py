#!/usr/bin/env python3
"""Generate sdl3-mixer's tracker module test files (sdl3-mixer/src/testdata/modules/).

The modules are written byte by byte here, from synthetic samples (square,
saw and sine waves and LCG noise), so there's nothing copyrighted in them
and they stay small (a few seconds of music each). Each one exercises a
loader of libxmp (translated in sdl3-mixer/src/xmp/) and a set of its
effects, envelopes, loops and instrument features; the IT files carry
samples compressed the way Impulse Tracker 2.14 and 2.15 compress them,
and the OXM file an Ogg Vorbis sample (testdata/audio/ogg_mono.ogg). Every
other loader SDL_mixer's build of libxmp enables gets a module too (all but
the Magnetic Fields Packer, which libxmp only accepts from a file handle):
MED synth and octave instruments, Digitrakker's packed samples, Digital
Symphony's LZW and sigma-delta streams, a MUSE file's deflate stream
(Python's zlib) and an Unreal package around one of the S3M files among
them.

sdl3-mixer's reference output (`testdata/reference.txt`, made by a C program
built from upstream SDL_mixer, its vendored libxmp and SDL3) must be
regenerated when these files change.

usage: tools/gen_xmp_testdata.py [OUTDIR]
"""

import math
import os
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, '..', 'sdl3-mixer', 'src', 'testdata', 'modules')
AUDIO = os.path.join(HERE, '..', 'sdl3-mixer', 'src', 'testdata', 'audio')


def write(name, data):
    with open(os.path.join(OUT, name), 'wb') as f:
        f.write(data)


def pad(s, n, fill=b'\0'):
    if isinstance(s, str):
        s = s.encode('latin-1')
    return (s + fill * n)[:n]


# ---------------------------------------------------------------------------
# Samples (lists of signed integers)

class Lcg:
    def __init__(self, seed):
        self.s = seed & 0xFFFFFFFF

    def next(self):
        self.s = (self.s * 1664525 + 1013904223) & 0xFFFFFFFF
        return self.s


def square(n, period, amp):
    return [amp if (i % period) < period // 2 else -amp for i in range(n)]


def saw(n, period, amp):
    return [int(-amp + (2 * amp * (i % period)) // period) for i in range(n)]


def sine(n, period, amp):
    return [int(round(amp * math.sin(2 * math.pi * i / period))) for i in range(n)]


def noise(n, amp, seed):
    rng = Lcg(seed)
    return [((rng.next() >> 16) % (2 * amp + 1)) - amp for _ in range(n)]


def decay(samples, start=1.0, end=0.1):
    n = len(samples)
    return [int(round(s * (start + (end - start) * i / max(1, n - 1)))) for i, s in enumerate(samples)]


def s8(samples):
    return bytes((s & 0xFF) for s in samples)


def u8(samples):
    return bytes(((s + 128) & 0xFF) for s in samples)


def s16le(samples):
    return b''.join(struct.pack('<h', s) for s in samples)


def u16le(samples):
    return b''.join(struct.pack('<H', (s + 32768) & 0xFFFF) for s in samples)


def delta8(samples):
    out, prev = [], 0
    for s in samples:
        out.append((s - prev) & 0xFF)
        prev = s
    return bytes(out)


def delta16le(samples):
    out, prev = b'', 0
    for s in samples:
        out += struct.pack('<H', (s - prev) & 0xFFFF)
        prev = s
    return out


NOTES = ['C-', 'C#', 'D-', 'D#', 'E-', 'F-', 'F#', 'G-', 'G#', 'A-', 'A#', 'B-']


def note_index(name):
    """'C-4' -> semitones from C-0."""
    return NOTES.index(name[:2]) + 12 * int(name[2])


# ---------------------------------------------------------------------------
# Protracker MOD

PT_PERIODS = [
    856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453,
    428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226,
    214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113,
]


def mod_period(note):
    if note is None:
        return 0
    return PT_PERIODS[note_index(note) - 12]


def parse_mod_cell(cell):
    """'C-2 01 F03' (note or '---', instrument or '..', effect or '...')."""
    note, ins, fx = cell.split()
    note = None if note == '---' else note
    ins = 0 if ins == '..' else int(ins, 16)
    fxt, fxp = (0, 0) if fx == '...' else (int(fx[0], 16), int(fx[1:], 16))
    return note, ins, fxt, fxp


def mod_pattern(chn, rows, cells):
    """cells: {row: {channel: cell}} -> 64 rows of 4-byte events."""
    out = b''
    for r in range(rows):
        for c in range(chn):
            cell = cells.get(r, {}).get(c)
            if cell is None:
                out += b'\0\0\0\0'
                continue
            note, ins, fxt, fxp = parse_mod_cell(cell)
            p = mod_period(note)
            out += bytes([(ins & 0xF0) | (p >> 8), p & 0xFF, ((ins & 0x0F) << 4) | fxt, fxp])
    return out


def mod_file(title, samples, orders, patterns, magic=b'M.K.', restart=0x7F, chn=4, empty_loop=1,
             sample_prefix=None, extra=b''):
    """samples: up to 31 (name, data, finetune, volume, loop_start, loop_len) in bytes."""
    hdr = pad(title, 20)
    body = b''
    for i in range(31):
        if i < len(samples) and samples[i] is not None:
            name, data, fin, vol, ls, ll = samples[i]
            hdr += pad(name, 22) + struct.pack('>HBBHH', len(data) // 2, fin & 0xF, vol, ls // 2, ll // 2)
        else:
            hdr += pad('', 22) + struct.pack('>HBBHH', 0, 0, 0, 0, empty_loop)
    hdr += bytes([len(orders), restart]) + pad(bytes(orders), 128) + magic + extra
    for cells in patterns:
        body += mod_pattern(chn, 64, cells)
    for i, s in enumerate(samples):
        if s is not None:
            if sample_prefix and i in sample_prefix:
                body += sample_prefix[i]
            else:
                body += s[1]
    return hdr + body


def adpcm4(samples):
    """ModPlug ADPCM4: a 16-entry delta table, then nibbles."""
    table = [0, 1, 2, 4, 8, 16, 32, 64, -1, -2, -4, -8, -16, -32, -48, -64]
    out, cur = [], 0
    for s in samples:
        best = min(range(16), key=lambda k: abs(((cur + table[k] + 128) & 0xFF) - 128 - s))
        out.append(best)
        cur = ((cur + table[best] + 128) & 0xFF) - 128
    if len(out) & 1:
        out.append(0)
    nibbles = bytes(out[i] | (out[i + 1] << 4) for i in range(0, len(out), 2))
    return b'ADPCM' + bytes(t & 0xFF for t in table) + nibbles


def mod_files():
    sq = s8(square(64, 16, 100))
    sw = s8(decay(saw(512, 32, 110)))
    sn = s8(sine(256, 64, 90))
    nz = s8(noise(300, 60, 7))
    smp = [
        ('square', sq, 0, 64, 0, 64),
        ('saw down', sw, 0xE, 48, 0, 2),
        ('sine', sn, 3, 40, 128, 128),
        ('noise', nz, 0, 30, 100, 200),
    ]
    p0 = {
        0: {0: 'C-2 01 F03', 1: 'E-2 02 047', 2: 'G-2 03 744', 3: 'C-3 04 C20'},
        1: {0: '--- .. 105', 1: '--- .. 047', 2: '--- .. 700', 3: '--- .. A02'},
        2: {0: '--- .. 205', 1: '--- .. 037', 2: '--- .. E72', 3: '--- .. A20'},
        3: {0: 'E-2 01 308', 2: '--- .. 733'},
        4: {0: '--- .. 300', 1: 'A-2 02 446', 3: 'B-1 04 000'},
        5: {0: '--- .. 502', 1: '--- .. 400', 2: 'C-2 03 E60'},
        6: {0: 'G-2 01 E12', 1: '--- .. 601', 2: '--- .. A10', 3: '--- .. 880'},
        7: {0: '--- .. E22', 1: '--- .. E41', 2: '--- .. E62'},
        8: {0: 'C-2 01 901', 1: 'A-2 02 438', 3: '--- .. E8C'},
        9: {0: '--- .. E93', 2: '--- .. EE1'},
        10: {0: '--- .. EC1', 1: '--- .. E31', 3: '--- .. F7D'},
        11: {0: 'C-2 01 ED1', 1: 'C-3 02 310'},
        12: {0: '--- .. EA2', 1: '--- .. 310', 2: 'G-1 03 EF4'},
        13: {0: '--- .. EB4', 1: '--- .. E30', 2: '--- .. EF4'},
        15: {0: '--- .. D00'},
    }
    p1 = {
        0: {0: 'C-1 02 ...', 1: '--- .. E01', 2: 'C-2 01 ...', 3: 'D-2 .. ...'},
        1: {0: 'D-1 02 ...', 3: '--- 03 ...'},
        2: {0: 'E-1 02 ...', 1: '--- .. E00', 2: '--- .. E54'},
        3: {0: 'F#1 02 ...', 3: 'F-3 03 ...'},
        4: {0: 'A-1 02 ...', 2: 'C-2 01 ...'},
        5: {0: 'B-1 02 ...', 1: 'G#2 04 ...'},
        6: {0: 'C#2 02 ...', 3: 'B-3 03 ...'},
        7: {0: 'D#3 02 ...', 1: '--- .. 0F0'},
        8: {0: '--- 02 ...', 2: 'A#2 01 C30'},
        10: {0: 'G-3 02 ...', 3: '--- .. E9F'},
        12: {1: 'C-2 04 3FF'},
        15: {0: '--- .. B02'},
    }
    p2 = {
        0: {0: 'C-2 03 ...', 1: 'E-2 03 ...', 2: 'G-2 03 ...', 3: 'B-2 03 ...'},
        4: {0: '--- .. F02', 2: '--- .. 0C0'},
        6: {1: '--- .. D10'},
    }
    write('mod_pt.mod', mod_file('Protracker test', smp, [0, 1, 2, 1], [p0, p1, p2]))

    # empty instruments with 0-length loops: a Protracker clone
    q0 = {
        0: {0: 'C-2 01 F04', 1: 'G-2 03 880', 2: 'C-3 02 800', 3: 'E-2 04 8FF'},
        2: {0: '--- .. 0C7', 1: '--- .. 820'},
        4: {0: 'D-2 01 ...', 2: '--- .. E8F', 3: '--- .. E1F'},
        6: {0: '--- .. 201', 1: '--- .. 4F2'},
        8: {0: 'C-2 02 ...', 1: 'E-2 02 EA4'},
        12: {0: '--- .. B00'},
    }
    write('mod_clone.mod', mod_file('clone', smp, [0], [q0], empty_loop=0))

    # Fast Tracker 6CHN, with BPM changes
    r0 = {
        0: {0: 'C-2 01 F05', 1: 'E-2 02 ...', 2: 'G-2 03 ...', 3: 'C-3 04 F80', 4: 'E-3 01 8C0', 5: 'G-1 02 840'},
        3: {4: '--- .. 147', 5: '--- .. 283'},
        6: {0: 'D-2 01 E5F', 1: '--- .. 9FF', 2: 'C-2 03 E52'},
        9: {3: '--- .. FA0', 5: 'A-2 04 ...'},
        12: {0: '--- .. D00'},
    }
    write('mod_6chn.mod', mod_file('six channels', smp, [0, 0], [r0], magic=b'6CHN', chn=6))

    # Noisetracker (restart 0x78), an ADPCM sample
    nt = [
        ('nt square', sq, 0, 64, 0, 64),
        ('nt adpcm', s8(sine(200, 50, 100)), 0, 50, 0, 2),
    ]
    n0 = {
        0: {0: 'C-2 01 F04', 1: 'E-2 02 ...'},
        2: {0: '--- .. 703', 1: '--- .. 0A3'},
        4: {0: 'G-2 01 E02', 1: 'C-3 02 4A4'},
        6: {2: 'C-2 02 ...', 3: 'G-2 01 9A0'},
        8: {0: '--- .. D00'},
    }
    write('mod_nt.mod', mod_file('noise', nt, [0, 0], [n0], restart=0x78,
                                 sample_prefix={1: adpcm4(sine(200, 50, 100))}))

    # Octalyser CD81
    o0 = {r: {c: ['C-2 01 ...', 'E-2 02 ...', 'G-2 03 ...', 'C-3 04 ...'][(r + c) % 4]
              for c in range(8) if (r + c) % 3 == 0} for r in range(0, 12, 2)}
    o0[0][0] = 'C-2 01 F03'
    o0[11] = {0: '--- .. D00'}
    write('mod_cd81.mod', mod_file('octalyser', smp, [0], [o0], magic=b'CD81', chn=8))

    # Digital Tracker FA04 (four extra bytes after the magic)
    f0 = {
        0: {0: 'C-2 01 F03', 1: 'E-2 03 ...'},
        2: {2: 'G-2 02 E62', 3: 'C-3 04 ...'},
        4: {0: '--- .. E60', 1: 'D-2 03 ...'},
        6: {0: '--- .. E62'},
        8: {0: '--- .. D00'},
    }
    write('mod_fa04.mod', mod_file('digital', smp, [0], [f0], magic=b'FA04', extra=b'\x00\x40\x00\x00'))

    # VBlank: high Fxx only at the end of a long order list
    v0 = {0: {0: 'C-2 01 F03', 1: 'E-2 02 ...'}, 4: {0: '--- .. D00'}}
    v1 = {0: {0: 'G-2 03 F30', 1: 'C-3 04 ...'}, 4: {0: '--- .. D00'}}
    write('mod_vblank.mod', mod_file('vblank', smp, [0] * 8 + [1], [v0, v1], restart=0))


# ---------------------------------------------------------------------------
# Fast Tracker II XM

def parse_xm_cell(cell):
    """'C-4 01 40 A0F': note ('---', '===' for key off), instrument, volume
    column byte (hex or '..'), effect (type as one hex/alnum char, or '...')."""
    note, ins, vol, fx = cell.split()
    if note == '---':
        n = 0
    elif note == '===':
        n = 97
    else:
        n = note_index(note) + 1
    ins = 0 if ins == '..' else int(ins, 16)
    vol = 0 if vol == '..' else int(vol, 16)
    if fx == '...':
        fxt, fxp = 0, 0
    else:
        fxt = int(fx[0], 36)
        fxp = int(fx[1:], 16)
    return n, ins, vol, fxt, fxp


def xm_pattern(chn, rows, cells, packed=True, version=0x0104):
    data = b''
    for r in range(rows):
        for c in range(chn):
            cell = cells.get(r, {}).get(c)
            n, ins, vol, fxt, fxp = parse_xm_cell(cell) if cell else (0, 0, 0, 0, 0)
            if packed:
                fields = [n, ins, vol, fxt, fxp]
                mask = 0
                for k, v in enumerate(fields):
                    if v:
                        mask |= 1 << k
                if mask == 0x1F:
                    data += bytes(fields)
                else:
                    data += bytes([0x80 | mask] + [v for v in fields if v])
            else:
                data += bytes([n, ins, vol, fxt, fxp])
    if version > 0x0102:
        return struct.pack('<IBHH', 9, 0, rows, len(data)) + data
    return struct.pack('<IBBH', 8, 0, rows - 1, len(data)) + data


def xm_instrument(name, samples, keymap=None, venv=None, penv=None, vib=(0, 0, 0, 0), fadeout=0x200,
                  short_header=False, adpcm=()):
    """samples: (name, data bytes, length in bytes, loop_start, loop_len, volume, finetune, type, pan, relnote,
    reserved). venv/penv: (points [(x, y)], sustain, loop_start, loop_end, type)."""
    if not samples:
        return struct.pack('<I', 29) + pad(name, 22) + b'\0' + struct.pack('<H', 0), b''
    if short_header:
        hdr = struct.pack('<I', 33) + pad(name, 22) + b'\0' + struct.pack('<HI', len(samples), 40)
    else:
        km = bytes(keymap or [0] * 96)
        body = km
        for env in (venv, penv):
            pts = env[0] if env else []
            words = []
            for x, y in pts:
                words += [x, y]
            body += struct.pack('<24H', *(words + [0] * (24 - len(words))))
        body += bytes([len(venv[0]) if venv else 0, len(penv[0]) if penv else 0,
                       venv[1] if venv else 0, venv[2] if venv else 0, venv[3] if venv else 0,
                       penv[1] if penv else 0, penv[2] if penv else 0, penv[3] if penv else 0,
                       venv[4] if venv else 0, penv[4] if penv else 0]) + bytes(vib)
        body += struct.pack('<H', fadeout) + b'\0' * 22
        hdr = struct.pack('<I', 29 + 4 + len(body)) + pad(name, 22) + b'\0' + struct.pack('<HI', len(samples), 40) + body
    sh = b''
    sd = b''
    for (sname, data, length, ls, ll, vol, fin, typ, pan, rel, res) in samples:
        sh += struct.pack('<IIIBbBBbB', length, ls, ll, vol, fin, typ, pan, rel, res) + pad(sname, 22)
        sd += data
    return hdr + sh, sd


def xm_file(name, tracker, instruments, orders, patterns, chn, version=0x0104, flags=1, tempo=6, bpm=125,
            restart=0, headersz=276, extensions=b'', packed=True):
    hdr = b'Extended Module: ' + pad(name, 20) + b'\x1a' + pad(tracker, 20, b' ')
    hdr += struct.pack('<HIHHHHHHHH', version, headersz, len(orders), restart, chn, len(patterns),
                       len(instruments), flags, tempo, bpm)
    hdr += pad(bytes(orders), headersz - 20)
    pats = b''.join(xm_pattern(chn, rows, cells, packed, version) for rows, cells in patterns)
    if version <= 0x0103:
        # (the sample data of XM 1.02 and 1.03 follows the patterns)
        return hdr + b''.join(h for h, _ in instruments) + pats + b''.join(d for _, d in instruments) + extensions
    return hdr + pats + b''.join(h + d for h, d in instruments) + extensions


def xm_files():
    sq = square(64, 32, 90)
    sn16 = sine(512, 128, 20000)
    sw = saw(256, 64, 100)
    venv = ([(0, 64), (4, 48), (12, 56), (24, 20), (40, 0)], 2, 1, 3, 7)
    penv = ([(0, 32), (6, 0), (12, 64), (20, 32)], 1, 0, 3, 7)
    i1 = xm_instrument('envelopes', [('square', delta8(sq), 64, 0, 64, 64, 0, 1, 128, 0, 0)],
                       venv=venv, penv=penv, vib=(1, 8, 4, 2), fadeout=0x300)
    keymap = [0] * 48 + [1] * 48
    i2 = xm_instrument('two samples', [
        ('sine16', delta16le(sn16), 1024, 128, 768, 48, -16, 0x12, 96, 12, 0),
        ('saw', delta8(sw), 256, 0, 0, 56, 20, 0, 200, -5, 0),
    ], keymap=keymap, venv=([(0, 64), (10, 32)], 0, 0, 0, 1))
    i3 = xm_instrument('empty', [])
    p0 = (24, {
        0: {0: 'C-4 01 .. F04', 1: 'E-5 02 40 ...', 2: 'C-4 02 .. 047', 3: 'G-4 01 C8 ...'},
        1: {0: '--- .. 65 ...', 1: '--- .. 74 ...', 2: '--- .. A4 ...', 3: '--- .. D3 ...'},
        2: {0: '--- .. 83 ...', 1: '--- .. 92 ...', 2: '--- .. B6 ...', 3: '--- .. E4 ...'},
        3: {0: 'G-4 .. F4 304', 1: '--- .. .. G20', 3: '--- .. .. P2F'},
        4: {0: '--- .. .. H08', 1: 'A-4 02 .. R43', 2: '=== .. .. ...'},
        5: {0: '--- .. .. H80', 1: '--- .. .. T21', 3: '--- .. .. 8F0'},
        6: {0: 'C-5 01 .. X14', 1: '--- .. .. X22', 2: 'D-4 02 .. K02'},
        7: {0: '--- .. .. 1F2', 1: '--- .. .. 2E3', 2: '--- .. .. L08'},
        8: {0: 'C-4 01 .. 9A0', 1: 'E-4 02 .. E5C', 2: '--- .. .. G40', 3: 'C-3 01 .. 721'},
        9: {0: '--- .. .. E93', 3: '--- .. .. 700'},
        10: {0: 'G-3 01 .. EC3', 1: '--- .. .. 1F1', 2: 'A-5 02 .. 520'},
        11: {0: 'C-4 01 .. ED2', 2: '--- .. .. 510'},
        12: {0: '--- .. .. E61', 1: '=== .. .. ...', 3: 'C-6 03 .. ...'},
        13: {0: '--- .. .. E62', 1: 'C-4 02 .. 604'},
        14: {0: '--- .. .. X90', 3: 'C-4 01 .. X91'},
        15: {0: '--- .. .. X9F', 2: '--- .. .. X9E'},
        16: {0: '--- .. .. EE1', 1: '--- .. .. E4F'},
        18: {0: 'D-4 01 .. F80', 3: '--- .. .. A0F'},
        20: {0: '--- .. .. B01'},
    })
    p1 = (16, {
        0: {0: 'C-4 02 .. 0F0', 1: 'C-3 01 .. 1F8', 2: 'G-5 02 .. 2F8', 3: 'E-4 01 .. ...'},
        2: {0: '--- .. .. E14', 1: '--- .. .. E24', 3: '--- .. .. 341'},
        4: {0: 'C-4 .. .. 400', 2: '=== 02 .. ...', 3: 'C-4 01 .. K00'},
        6: {0: '--- 01 .. ...', 1: 'C-8 02 .. ...', 2: 'C-0 01 .. ...'},
        8: {0: '--- .. .. E3F', 1: 'C-4 01 .. EB4', 3: '--- .. .. EA4'},
        10: {0: '--- .. .. D04'},
    })
    text = b'XM comment\rsecond\x01line \xe9    '
    comment = b'text' + struct.pack('<I', len(text)) + text
    write('xm_ft2.xm', xm_file('FT2 test', 'FastTracker v2.00', [i1, i2, i3], [0, 1, 0], [p0, p1], 4,
                               tempo=4, bpm=140, extensions=comment))

    # Amiga frequencies, OpenMPT: a stereo sample, an ADPCM sample, 256 rows
    st = s8(sine(128, 32, 80)) + s8(saw(128, 16, 80))
    a1 = xm_instrument('stereo', [('stereo', delta8(sine(128, 32, 80)) + delta8(saw(128, 16, 80)), 256, 0, 128, 64, 0,
                                   0x21, 128, 0, 0)])
    ad = adpcm4(sine(300, 60, 90))[5:]
    a2 = xm_instrument('adpcm', [('adpcm', ad, 300, 0, 0, 50, 0, 0, 64, 0, 0xad)])
    rows = {r: {0: 'C-4 01 .. ...' if r % 32 == 0 else None, 1: 'G-4 02 30 ...' if r % 48 == 8 else None}
            for r in range(256)}
    rows = {r: {c: v for c, v in d.items() if v} for r, d in rows.items()}
    rows[0][0] = 'C-4 01 .. F02'
    rows[200] = {0: '--- .. .. B00'}
    del st
    write('xm_amiga.xm', xm_file('amiga', 'OpenMPT 1.28.00.00', [a1, a2], [0], [(256, rows)], 2, flags=0,
                                 tempo=2, bpm=200))

    # XM 1.02 (8-bit row counts, samples after the patterns), MED2XM BPM, no packing
    o1 = xm_instrument('old', [('old', delta8(sq), 64, 0, 64, 60, 0, 1, 128, 0, 0)])
    o0 = (12, {0: {0: 'C-4 01 .. ...', 1: 'G-4 01 20 ...'}, 6: {0: 'D-4 01 .. 102'}, 11: {0: '--- .. .. B00'}})
    write('xm_old.xm', xm_file('old', 'MED2XM by J.Pynnone', [o1], [0], [o0], 2, version=0x0102, tempo=3, bpm=8,
                               packed=False))

    # OXM: an Ogg Vorbis sample; BoobieSqueezer-style instrument headers
    with open(os.path.join(AUDIO, 'ogg_mono.ogg'), 'rb') as f:
        ogg = f.read()
    oggdata = struct.pack('<I', len(ogg)) + ogg
    v1 = xm_instrument('vorbis', [('vorbis', oggdata, len(oggdata), 0, 0, 64, 0, 0x10, 128, 0, 0)],
                       short_header=True)
    v2 = xm_instrument('vorbis8', [('vorbis8', oggdata, len(oggdata), 0, 0, 40, 0, 0x00, 128, 0, 0)],
                       short_header=True)
    v0 = (8, {0: {0: 'C-5 01 .. F03', 1: 'G-5 02 .. ...'}, 4: {0: 'E-5 01 .. ...'}, 7: {0: '--- .. .. B00'}})
    write('xm_oxm.xm', xm_file('vorbis', 'unknown', [v1, v2], [0], [v0], 2))


# ---------------------------------------------------------------------------
# Scream Tracker 3 S3M

S3M_FX = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ'


def parse_s3m_cell(cell):
    """'C-4 01 40 A03': note ('---', '^^^' note off), instrument, volume (decimal or '..'), effect."""
    note, ins, vol, fx = cell.split()
    if note == '---':
        n = None
    elif note == '^^^':
        n = 254
    else:
        n = (int(note[2]) << 4) | NOTES.index(note[:2])
    ins = None if ins == '..' else int(ins, 16)
    vol = None if vol == '..' else int(vol)
    fx = None if fx == '...' else (S3M_FX.index(fx[0]) + 1, int(fx[1:], 16))
    return n, ins, vol, fx


def s3m_pattern(cells, chn):
    data = b''
    for r in range(64):
        for c in range(chn):
            cell = cells.get(r, {}).get(c)
            if not cell:
                continue
            n, ins, vol, fx = parse_s3m_cell(cell)
            b = c
            body = b''
            if n is not None or ins is not None:
                b |= 0x20
                body += bytes([255 if n is None else n, ins or 0])
            if vol is not None:
                b |= 0x40
                body += bytes([vol])
            if fx is not None:
                b |= 0x80
                body += bytes(fx)
            data += bytes([b]) + body
        data += b'\0'
    return struct.pack('<H', len(data) + 2) + data


def s3m_file(name, version, ffi, mv, chset, instruments, orders, patterns, flags=0, gv=64, speed=6, tempo=125,
             dp=0xFC, pans=None, special=0, rsvd2=b'\0' * 8, uc=0):
    """instruments: ('sample', name, data, length, loopbeg, loopend, vol, flags, c2spd, pack) or ('adlib', name)."""
    hdr = pad(name, 28) + bytes([0x1A, 0x10, 0, 0])
    hdr += struct.pack('<HHHHHH', len(orders), len(instruments), len(patterns), flags, version, ffi)
    hdr += b'SCRM' + bytes([gv, speed, tempo, mv, uc, dp]) + pad(rsvd2, 8) + struct.pack('<H', special)
    hdr += pad(bytes(chset), 32, b'\xff')
    hdr += bytes(orders)
    off = len(hdr) + 2 * len(instruments) + 2 * len(patterns) + (32 if dp == 0xFC else 0)
    off = (off + 15) & ~15
    ins_offs, ins_data = [], b''
    for _ in instruments:
        ins_offs.append(off + len(ins_data))
        ins_data += b'\0' * 80
    pat_offs, pat_data = [], b''
    base = off + len(ins_data)
    for p in patterns:
        pat_offs.append(base + len(pat_data))
        pat_data += s3m_pattern(p, 32)
        while len(pat_data) % 16:
            pat_data += b'\0'
    smp_base = base + len(pat_data)
    smp_data = b''
    ins_list = []
    for ins in instruments:
        if ins[0] == 'adlib':
            ins_list.append(bytes([2]) + pad('adlib.ins', 12) + b'\0\0\0'
                            + bytes([0x21, 0x21, 0x10, 0x05, 0xF0, 0xF0, 0x33, 0x33, 0, 0, 0x08, 0])
                            + bytes([48, 0, 0, 0]) + struct.pack('<HH', 8363, 0) + b'\0' * 12 + pad(ins[1], 28)
                            + b'SCRI')
            continue
        _, iname, data, length, lb, le, vol, iflags, c2spd, pack = ins
        seg = (smp_base + len(smp_data)) >> 4
        ins_list.append(bytes([1]) + pad('sample.smp', 12) + bytes([seg >> 16]) + struct.pack('<HIII', seg & 0xFFFF,
                                                                                             length, lb, le)
                        + bytes([vol, 0, pack, iflags]) + struct.pack('<HH', c2spd, 0) + b'\0' * 12
                        + pad(iname, 28) + b'SCRS')
        smp_data += data
        while len(smp_data) % 16:
            smp_data += b'\0'
    ins_data = b''.join(ins_list)
    out = hdr + b''.join(struct.pack('<H', o >> 4) for o in ins_offs) + b''.join(
        struct.pack('<H', o >> 4) for o in pat_offs)
    if dp == 0xFC:
        out += bytes(pans or [0x20 | (3 if i % 2 == 0 else 0xC) for i in range(32)])
    out = pad(out, off)
    return out + ins_data + pat_data + smp_data


def s3m_files():
    sq = square(256, 32, 100)
    sn = sine(512, 64, 25000)
    sw = saw(300, 50, 110)
    ins = [
        ('sample', 'square loop', u8(sq), 256, 0, 256, 64, 1, 8363, 0),
        ('sample', 'sine 16 bit', u16le(sn), 512, 64, 512, 48, 5, 16726, 0),
        ('sample', 'saw', u8(sw), 300, 0, 0, 40, 0, 4000, 0),
        ('adlib', 'fm piano'),
    ]
    p0 = {
        0: {0: 'C-4 01 .. A04', 1: 'E-4 02 .. ...', 2: 'G-4 03 32 ...', 3: 'C-5 04 .. ...'},
        1: {0: '--- .. .. D04', 1: '--- .. .. D40', 2: '--- .. .. DF2', 3: '--- .. .. D2F'},
        2: {0: '--- .. .. D00', 1: '--- .. .. E04', 2: '--- .. .. F04'},
        3: {0: 'G-4 .. .. G08', 1: '--- .. .. EF2', 2: '--- .. .. FE4'},
        4: {0: '--- .. .. G00', 1: 'C-4 02 .. H44', 2: 'E-4 03 .. I21'},
        5: {0: '--- .. .. K02', 1: '--- .. .. U84', 2: '--- .. .. J37'},
        6: {0: 'A-4 01 .. L20', 1: '--- .. .. R44', 2: 'C-4 01 .. O80'},
        7: {0: '--- .. .. Q32', 1: '^^^ .. .. ...', 2: '--- .. .. S81'},
        8: {0: 'C-4 01 .. S31', 1: 'C-5 02 .. S41', 2: '--- .. .. S2F', 3: '--- .. .. T60'},
        9: {0: '--- .. .. SB0', 1: '--- .. .. V20', 2: 'D-4 01 .. SC2'},
        10: {0: '--- .. .. SB1', 1: 'E-4 02 .. SD2', 2: '--- .. .. X40'},
        11: {0: '--- .. .. SE1', 1: '--- .. .. XA4', 2: '--- .. .. X00'},
        12: {0: 'C-4 03 20 S12', 1: '--- .. .. V40', 3: '^^^ .. .. ...'},
        13: {0: '--- .. .. M20', 1: 'C-4 02 .. Z10', 2: '--- .. .. S00'},
        14: {0: '--- .. .. T10', 1: '--- .. .. T05'},
        15: {0: '--- .. .. C00'},
    }
    p1 = {
        0: {0: 'C-4 01 64 ...', 4: 'C-5 02 .. ...', 5: 'G-3 03 .. ...'},
        2: {0: '--- .. .. D0F', 4: '--- .. .. DF0', 5: '--- .. .. E80'},
        4: {0: '--- .. .. B00', 4: '--- .. .. F80'},
    }
    chset = [0, 8, 1, 9, 2, 10]
    write('s3m_st3.s3m', s3m_file('ST3 test', 0x1320, 2, 0xB0, chset, ins, [0, 1, 0xFE, 0, 0xFF], [p0, p1],
                                  speed=3, tempo=150, uc=16))

    # ST3.01 (old flow), signed samples, mono, Amiga limits, ST3.00 volume slides
    ins1 = [
        ('sample', 'signed square', s8(sq), 256, 0, 256, 64, 1, 8363, 0),
        ('sample', 'signed saw', s8(sw), 300, 100, 300, 40, 1, 9000, 0),
    ]
    q0 = {
        0: {0: 'C-4 01 .. A03', 1: 'G-4 02 .. ...'},
        2: {0: '--- .. .. SB0', 1: '--- .. .. D0F'},
        3: {0: '--- .. .. SB1', 1: '--- .. .. DF0'},
        5: {0: 'C-5 01 .. Q01', 1: '--- .. .. K04'},
        7: {0: '--- .. .. C00'},
    }
    write('s3m_st301.s3m', s3m_file('ST3.01', 0x1301, 1, 0x20, [0, 1], ins1, [0], [q0], flags=0x50, dp=0))

    # Schism Tracker (a date in the version), Imago Orpheus
    write('s3m_schism.s3m', s3m_file('schism', 0x4801, 2, 0xB0, chset, ins, [0], [p0], rsvd2=b'\x00\x00' + b'\0' * 6))
    write('s3m_orpheus.s3m', s3m_file('orpheus', 0x2104, 2, 0xB0, chset, ins, [1, 0], [p0, p1], speed=4))


# ---------------------------------------------------------------------------
# Impulse Tracker IT

class BitWriter:
    def __init__(self):
        self.bits = []

    def put(self, value, n):
        for i in range(n):
            self.bits.append((value >> i) & 1)

    def data(self):
        out = bytearray()
        for i in range(0, len(self.bits), 8):
            b = 0
            for k, bit in enumerate(self.bits[i:i + 8]):
                b |= bit << k
            out.append(b)
        return bytes(out)


def it_compress(samples, is16, it215):
    """Impulse Tracker sample compression, choosing bit widths as values need."""
    block_len = 0x4000 if is16 else 0x8000
    top = 17 if is16 else 9
    sbits = 16 if is16 else 8
    mask = (1 << sbits) - 1
    out = b''
    for start in range(0, len(samples), block_len):
        block = samples[start:start + block_len]
        vals = []
        d1 = 0
        prev = 0
        for s in block:
            d = (s - prev) & mask
            prev = s
            if it215:
                dd = (d - d1) & mask
                d1 = d
                d = dd
            if d >= 1 << (sbits - 1):
                d -= 1 << sbits
            vals.append(d)
        w = BitWriter()
        width = top

        def fits(v, wd):
            if wd == top:
                return True
            if wd < 7:
                return -(1 << (wd - 1)) < v < (1 << (wd - 1))
            if is16:
                return -(1 << (wd - 1)) + 8 <= v <= (1 << (wd - 1)) - 9
            return -(1 << (wd - 1)) + 4 <= v <= (1 << (wd - 1)) - 5

        def change(cur, new):
            b = new if new < cur else new - 1
            if cur < 7:
                w.put(1 << (cur - 1), cur)
                w.put(b - 1, 4 if is16 else 3)
            elif cur < top:
                if is16:
                    j = (0xFFFF >> (17 - cur)) + 8 - 16
                else:
                    j = (0xFF >> (9 - cur)) + 4 - 8
                w.put(j + b, cur)
            else:
                w.put((0xFFFF if is16 else 0xFF) + new, cur)

        for i, v in enumerate(vals):
            need = next(wd for wd in range(1, top + 1) if fits(v, wd))
            # widen at once, narrow only when a few values in a row allow it
            if need > width or (need < width - 1 and all(fits(x, width - 1) for x in vals[i:i + 4])):
                new = max(need, 1) if need > width else width - 1
                if new == width:
                    pass
                else:
                    change(width, new)
                    width = new
            # (at the widest, the value's own bits: the top bit marks a width change)
            w.put(v & (mask if width == top else (1 << width) - 1), width)
        data = w.data()
        out += struct.pack('<H', len(data)) + data
    return out


def it_decompress(data, n, is16, it215):
    """A check of it_compress(): libxmp's itsex.c in Python."""
    out = []
    pos = 0
    while len(out) < n:
        blen = struct.unpack_from('<H', data, pos)[0]
        pos += 2
        block = data[pos:pos + blen] + b'\0' * 4
        pos += blen
        bitpos = 0

        def read(k):
            nonlocal bitpos
            v = 0
            for i in range(k):
                v |= ((block[(bitpos + i) >> 3] >> ((bitpos + i) & 7)) & 1) << i
            bitpos += k
            return v
        left = 17 if is16 else 9
        temp = temp2 = 0
        count = min(0x4000 if is16 else 0x8000, n - len(out))
        sbits = 16 if is16 else 8
        mask = (1 << sbits) - 1
        produced = 0
        while produced < count:
            bits = read(left)
            if left < 7:
                if bits == 1 << (left - 1):
                    b = read(4 if is16 else 3) + 1
                    left = b if b < left else b + 1
                    continue
            elif left < (17 if is16 else 9):
                i = ((0xFFFF >> (17 - left)) + 8) if is16 else ((0xFF >> (9 - left)) + 4)
                j = i - (16 if is16 else 8)
                if j < bits <= i:
                    b = bits - j
                    left = b if b < left else b + 1
                    continue
            elif bits >= (0x10000 if is16 else 256):
                left = (bits + 1) & 0xFF
                continue
            if left < sbits:
                if bits & (1 << (left - 1)):
                    bits -= 1 << left
            temp = (temp + bits) & mask
            temp2 = (temp2 + temp) & mask
            v = temp2 if it215 else temp
            out.append(v - (1 << sbits) if v >= 1 << (sbits - 1) else v)
            produced += 1
    return out


def it_envelope(flags, points, lpb=0, lpe=0, slb=0, sle=0):
    b = bytes([flags, len(points), lpb, lpe, slb, sle])
    for x, y in points:
        b += struct.pack('<bH', y, x)
    return pad(b, 82)


def it_instrument_new(name, keymap, nna=0, dct=0, dca=0, fadeout=256, gbv=128, dfp=0x80 | 32, rv=0, rp=0,
                      ifc=0, ifr=0, envs=(None, None, None)):
    b = b'IMPI' + pad('inst.iti', 12) + bytes([0, nna, dct, dca]) + struct.pack('<H', fadeout)
    b += bytes([0, 60, gbv, dfp, rv, rp]) + struct.pack('<HBB', 0x214, 0, 0) + pad(name, 26)
    b += bytes([ifc, ifr, 0, 0]) + struct.pack('<H', 0)
    for n, s in keymap:
        b += bytes([n, s])
    for e in envs:
        b += e if e else it_envelope(0, [])
    return pad(b, 554)


def it_instrument_old(name, keymap, flags=0, vls=0, vle=0, sls=0, sle=0, fadeout=32, nna=0, dnc=0, nodes=()):
    b = b'IMPI' + pad('old.iti', 12) + bytes([0, flags, vls, vle, sls, sle]) + b'\0\0' + struct.pack('<H', fadeout)
    b += bytes([nna, dnc]) + struct.pack('<HBB', 0x100, 0, 0) + pad(name, 26) + b'\0' * 6
    for n, s in keymap:
        b += bytes([n, s])
    b += b'\0' * 200
    en = b''
    for x, y in nodes:
        en += bytes([x, y])
    en += b'\xff\xff'
    b += pad(en, 50, b'\xff')
    return pad(b, 554)


def it_sample_header(name, flags, convert, length, loopbeg=0, loopend=0, c5spd=8363, sloopbeg=0, sloopend=0,
                     vol=64, gvl=64, dfp=0, vib=(0, 0, 0, 0), ptr=0):
    return (b'IMPS' + pad('smp.its', 12) + bytes([0, gvl, flags, vol]) + pad(name, 26) + bytes([convert, dfp])
            + struct.pack('<IIIIIII', length, loopbeg, loopend, c5spd, sloopbeg, sloopend, ptr) + bytes(vib))


def it_note(note):
    if note == '===':
        return 255
    if note == '^^^':
        return 254
    if note == '~~~':
        return 246
    return note_index(note)


def parse_it_cell(cell):
    """'C-4 01 v40 A03': note ('---', '===' off, '^^^' cut, '~~~' fade), instrument, volume column
    (v + raw byte in decimal, or '...'), effect (letter + hex)."""
    note, ins, vol, fx = cell.split()
    n = None if note == '---' else it_note(note)
    i = None if ins == '..' else int(ins, 16)
    v = None if vol == '...' else int(vol[1:])
    f = None if fx == '...' else (ord(fx[0]) - 64, int(fx[1:], 16))
    return n, i, v, f


def it_pattern(rows, cells, chn=64, use_last=True):
    data = b''
    last = {}
    for r in range(rows):
        for c in range(chn):
            cell = cells.get(r, {}).get(c)
            if not cell:
                continue
            n, i, v, f = parse_it_cell(cell)
            mask = 0
            body = b''
            prev = last.get(c, (None, None, None, None))
            if n is not None:
                if use_last and n == prev[0]:
                    mask |= 0x10
                else:
                    mask |= 1
                    body += bytes([n])
            if i is not None:
                if use_last and i == prev[1]:
                    mask |= 0x20
                else:
                    mask |= 2
                    body += bytes([i])
            if v is not None:
                if use_last and v == prev[2]:
                    mask |= 0x40
                else:
                    mask |= 4
                    body += bytes([v])
            if f is not None:
                if use_last and f == prev[3]:
                    mask |= 0x80
                else:
                    mask |= 8
                    body += bytes(f)
            last[c] = (n if n is not None else prev[0], i if i is not None else prev[1],
                       v if v is not None else prev[2], f if f is not None else prev[3])
            data += bytes([(c + 1) | 0x80, mask]) + body
        data += b'\0'
    return struct.pack('<HH', len(data), rows) + b'\0\0\0\0' + data


def it_file(name, cwt, cmwt, flags, special, instruments, samples, orders, patterns, gv=128, mv=48, speed=6,
            tempo=125, sep=128, chpan=None, chvol=None, message=b'', midi=None, rsvd=0, edit_history=None):
    """samples: (header fields dict, data bytes); instruments: bytes (old or new)."""
    hdr = b'IMPM' + pad(name, 26) + bytes([4, 16])
    hdr += struct.pack('<HHHHHHHH', len(orders), len(instruments), len(samples), len(patterns), cwt, cmwt, flags,
                       special)
    hdr += bytes([gv, mv, speed, tempo, sep, 0])
    hdr += struct.pack('<HII', len(message), 0, rsvd)
    hdr += bytes(chpan or [32] * 64) + bytes(chvol or [64] * 64) + bytes(orders)
    ptrs_at = len(hdr)
    n_ptr = len(instruments) + len(samples) + len(patterns)
    extra = b''
    if edit_history is not None:
        extra += struct.pack('<H', len(edit_history)) + b''.join(edit_history)
    if midi is not None:
        extra += midi
    pos = ptrs_at + 4 * n_ptr + len(extra)
    msg_at = pos
    pos += len(message)
    body = b''
    ins_ptrs, smp_ptrs, pat_ptrs = [], [], []
    for ins in instruments:
        ins_ptrs.append(pos + len(body))
        body += ins
    smp_hdr_at = []
    for _ in samples:
        smp_hdr_at.append(pos + len(body))
        smp_ptrs.append(pos + len(body))
        body += b'\0' * 80
    for p in patterns:
        if p is None:
            pat_ptrs.append(0)
            continue
        pat_ptrs.append(pos + len(body))
        body += p
    data_at = pos + len(body)
    sdata = b''
    headers = []
    for (fields, data) in samples:
        f = dict(fields)
        f['ptr'] = data_at + len(sdata)
        headers.append(it_sample_header(**f))
        sdata += data
    body = bytearray(body)
    for at, h in zip(smp_hdr_at, headers):
        o = at - pos
        body[o:o + 80] = h
    hdr = bytearray(hdr)
    msgofs = msg_at if message else 0
    struct.pack_into('<I', hdr, 0x38, msgofs)
    out = bytes(hdr) + b''.join(struct.pack('<I', p) for p in ins_ptrs + smp_ptrs + pat_ptrs) + extra + message
    return out + bytes(body) + sdata


def it_midi_config():
    def m(s):
        return pad(s, 32)
    glob = b''.join(m(s) for s in ['FF', 'FC', '', '9c n v', '9c n 0', '', 'Cc p', '', ''])
    sfx = m('F0F000z') + m('F0F001z') + b''.join(m('') for _ in range(14))
    zxx = b''.join(m('F0F001%02X' % (k * 8)) for k in range(16)) + b''.join(m('') for _ in range(112))
    return glob + sfx + zxx


def it_files():
    sq = square(400, 40, 90)
    sn16 = sine(2000, 100, 24000)
    sw = decay(saw(3000, 60, 110))
    nz16 = noise(1500, 9000, 11)
    c8 = it_compress(sw, False, False)
    assert it_decompress(c8, len(sw), False, False) == sw
    c16 = it_compress(sn16, True, True)
    assert it_decompress(c16, len(sn16), True, True) == sn16
    c8b = it_compress(sq, False, True)
    assert it_decompress(c8b, len(sq), False, True) == sq
    c16b = it_compress(nz16, True, False)
    assert it_decompress(c16b, len(nz16), True, False) == nz16
    samples = [
        (dict(name='square', flags=0x11, convert=1, length=400, loopbeg=0, loopend=400, vol=60, dfp=0x80 | 40,
              vib=(10, 8, 30, 0)), s8(sq)),
        (dict(name='sine 16 IT215', flags=0x0B | 0x10 | 0x40, convert=5, length=2000, loopbeg=100, loopend=2000,
              c5spd=16726, gvl=48), c16),
        (dict(name='saw IT214', flags=0x09 | 0x20, convert=1, length=3000, sloopbeg=500, sloopend=1500,
              c5spd=10000, vol=50), c8),
        (dict(name='noise 16', flags=0x0B, convert=1, length=1500, c5spd=22050, vol=40, dfp=0x80 | 10), c16b),
        (dict(name='square IT215', flags=0x09 | 0x10 | 0xC0, convert=5, length=400, loopbeg=40, loopend=400,
              sloopbeg=100, sloopend=300), c8b),
        (dict(name='unsigned', flags=0x01, convert=0, length=256, c5spd=8363), u8(sine(256, 64, 100))),
        (dict(name='bidi 16', flags=0x03 | 0x50, convert=1, length=600, loopbeg=100, loopend=600),
         s16le(saw(600, 100, 20000))),
        (dict(name='empty', flags=0, convert=0, length=0), b''),
    ]
    venv = it_envelope(0x01 | 0x02 | 0x04, [(0, 64), (5, 40), (10, 60), (20, 30), (30, 0)], 1, 3, 2, 2)
    penv = it_envelope(0x01 | 0x02, [(0, 0), (8, -32), (16, 32), (24, 0)], 0, 3)
    fenv = it_envelope(0x01, [(0, 0), (10, 12), (20, -12)])
    flt = it_envelope(0x01 | 0x80, [(0, 32), (12, -20), (24, 20)])
    km1 = [(k, 1 if k < 60 else 2) for k in range(120)]
    km2 = [(k + (2 if k < 48 else 0), 3 if k < 48 else 4) for k in range(120)]
    km3 = [(k, 5 if k % 2 else 6) for k in range(120)]
    km4 = [(k, 7) for k in range(120)]
    instruments = [
        it_instrument_new('envelopes NNA cont', km1, nna=1, dct=1, dca=2, fadeout=200, envs=(venv, penv, fenv)),
        it_instrument_new('filter NNA fade', km2, nna=3, dct=2, dca=3, fadeout=512, ifc=0x80 | 90, ifr=0x80 | 40,
                          rv=20, rp=10, envs=(venv, None, flt)),
        it_instrument_new('note off', km3, nna=2, dct=3, dca=1, gbv=100, dfp=20),
        it_instrument_new('unsigned', km4, nna=0, dct=0, dca=7),
    ]
    p0 = it_pattern(32, {
        0: {0: 'C-5 01 v64 A03', 1: 'E-5 01 ... T80', 2: 'G-4 02 v40 ...', 3: 'C-4 03 ... ...'},
        1: {0: '--- .. v65 ...', 1: '--- .. v75 ...', 2: '--- .. v85 ...', 3: '--- .. v95 ...'},
        2: {0: 'D-5 01 v105 ...', 1: '--- .. v115 ...', 2: '--- .. v128 ...', 3: '--- .. v192 ...'},
        3: {0: 'E-5 .. v196 ...', 1: '--- .. v205 ...', 2: '--- .. v160 ...', 3: '--- .. ... D0F'},
        4: {0: 'C-5 01 ... ...', 1: 'C-5 01 ... S73', 2: 'G-4 02 ... S71', 3: '--- .. ... DF1'},
        5: {0: 'E-5 01 ... S70', 1: '--- .. ... S77', 2: '--- .. ... S78', 3: '--- .. ... N04'},
        6: {0: '=== .. ... ...', 1: '^^^ .. ... ...', 2: '~~~ .. ... ...', 3: 'C-4 04 ... ...'},
        7: {0: 'C-6 02 ... Z40', 1: 'C-5 02 ... Z90', 2: '--- .. ... SF1', 3: '--- .. ... Z20'},
        8: {0: '--- .. ... \\44', 1: 'D-5 03 ... S74', 2: 'G-5 03 ... S75', 3: '--- .. ... S76'},
        9: {0: '--- .. ... Y44', 1: '--- .. ... S52', 2: '--- .. ... P0F', 3: '--- .. ... PF1'},
        10: {0: 'A-4 01 ... W04', 1: '--- .. ... W40', 2: '--- .. ... V60', 3: '--- .. ... M30'},
        11: {0: '--- .. ... X40', 1: '--- .. ... S91', 2: '--- .. ... S9F', 3: '--- .. ... S9E'},
        12: {0: 'C-5 05 ... SA1', 1: 'C-5 05 ... O80', 2: 'E-5 06 ... SC2', 3: 'G-5 06 ... SD1'},
        13: {0: '--- .. ... SE2', 1: '--- .. ... SB0', 2: '--- .. ... I21', 3: '--- .. ... J37'},
        14: {0: '--- .. ... SB1', 1: '--- .. ... Q83', 2: '--- .. ... R44', 3: '--- .. ... U82'},
        15: {0: 'C-5 07 ... G10', 1: 'G-5 07 ... H34', 2: '--- .. ... K01', 3: '--- .. ... L01'},
        16: {0: 'D-5 .. ... G00', 1: '--- .. ... H00', 2: '--- .. ... S30', 3: '--- .. ... S40'},
        17: {0: '--- .. ... EF1', 1: '--- .. ... FE1', 2: '--- .. ... E08', 3: '--- .. ... F08'},
        18: {0: '--- .. ... T01', 1: '--- .. ... T11', 2: '--- .. ... S00', 3: '--- .. ... S60'},
        20: {0: 'C-5 01 ... S72', 1: 'C-5 01 ... ...', 2: 'C-5 01 ... S7C', 3: 'C-5 01 ... S7A'},
        22: {0: 'C-5 01 ... ...', 1: '--- .. ... S7B', 2: '--- .. ... S79'},
        24: {0: '--- .. ... C00'},
    })
    p1 = it_pattern(20, {
        0: {0: 'C-5 02 ... ...', 1: 'E-5 02 ... ...', 2: 'G-5 02 ... ...', 3: 'C-6 02 ... ...'},
        2: {0: 'C-5 02 ... ...', 1: 'E-5 02 ... ...', 2: 'G-5 02 ... ...', 3: 'C-6 02 ... ...'},
        4: {0: 'D-5 03 ... ...', 1: '--- .. ... SD3', 2: '--- .. ... SC1'},
        6: {0: 'C-5 01 ... V80', 1: '--- .. ... W8F', 2: '--- .. ... WF8'},
        8: {0: '--- .. ... A01', 1: '--- .. ... S8F'},
        12: {0: '--- .. ... B02'},
    })
    msg = b'IT song message\rline two\r\x80\x00end'
    write('it_new.it', it_file('IT new instruments', 0x0214, 0x0214, 0x01 | 0x04 | 0x08 | 0x80, 0x01 | 0x08,
                               instruments, samples, [0, 1, 0xFE, 2, 0xFF],
                               [p0, p1, None], mv=64, speed=3, tempo=140, message=msg, midi=it_midi_config(),
                               chpan=[0, 64, 32, 100] + [32] * 60, chvol=[64, 50, 64, 64] + [64] * 60))

    # old instruments (cmwt 0x100), old effects, linked Gxx, mono, an empty pattern, edit history
    old_ins = [
        it_instrument_old('old envelope', [(k, 1) for k in range(120)], flags=0x01 | 0x02, vls=0, vle=2,
                          fadeout=20, nna=1, dnc=1, nodes=[(0, 64), (5, 32), (10, 64), (20, 0)]),
        it_instrument_old('old plain', [(k, 2 if k < 60 else 3) for k in range(120)], flags=0x04, sls=1, sle=1,
                          nodes=[(0, 64), (10, 10)]),
    ]
    o0 = it_pattern(16, {
        0: {0: 'C-5 01 ... A04', 1: 'G-4 02 ... ...'},
        2: {0: 'E-5 01 ... G08', 1: '--- .. ... I44'},
        4: {0: '--- .. ... L02', 1: 'C-5 02 ... H22'},
        6: {0: 'C-5 02 ... E04', 1: '--- .. ... ...'},
        8: {0: '=== .. ... ...', 1: 'C-6 01 v64 ...'},
        12: {0: '--- .. ... C00'},
    })
    write('it_old.it', it_file('IT old instruments', 0x0202, 0x0100, 0x04 | 0x10 | 0x20, 0x02, old_ins,
                               samples[:3], [0, 1, 0], [o0, None], speed=4, edit_history=[b'\0' * 8] * 2))

    # sample mode, Schism Tracker, stereo pans, a 16-bit stereo sample (MPT)
    st16 = s16le(sine(400, 50, 15000)) + s16le(saw(400, 40, 15000))
    smp_mode = [
        (dict(name='stereo', flags=0x07 | 0x10, convert=1, length=400, loopbeg=0, loopend=400), st16),
        samples[2],
        samples[0],
    ]
    s0 = it_pattern(16, {
        0: {0: 'C-5 01 ... A03', 1: 'E-5 02 ... ...', 2: 'G-5 03 ... ...'},
        4: {0: '--- .. ... S91', 1: 'C-5 02 v170 ...'},
        8: {0: 'C-4 03 ... T40', 2: '^^^ .. ... ...'},
        12: {0: '--- .. ... C00'},
    })
    write('it_smp.it', it_file('IT sample mode', 0x1A20, 0x0214, 0x01 | 0x08, 0x00, [], smp_mode, [0], [s0],
                               sep=64, rsvd=0x05, chpan=[0, 64, 32] + [32] * 61))


# ---------------------------------------------------------------------------
# The other formats libxmp loads. Each gets one or two small modules (a few
# samples, a pattern or two), in the order of upstream's loader list.

def be16(v):
    return struct.pack('>H', v & 0xFFFF)


def be32(v):
    return struct.pack('>I', v & 0xFFFFFFFF)


def le16(v):
    return struct.pack('<H', v & 0xFFFF)


def le32(v):
    return struct.pack('<I', v & 0xFFFFFFFF)


def pt_track(cells, rows=64):
    """One channel of Protracker events: {row: cell}."""
    return mod_pattern(1, rows, {r: {0: c} for r, c in cells.items()})


def iff(cid, data, le=False, align=False):
    out = cid + (le32(len(data)) if le else be32(len(data))) + data
    if align and len(data) & 1:
        out += b'\0'
    return out


SQ8 = s8(square(64, 16, 100))
SW8 = s8(decay(saw(512, 32, 110)))
SN8 = s8(sine(256, 64, 90))
NZ8 = s8(noise(300, 60, 7))


def amiga_samples():
    return [
        ('square', SQ8, 0, 64, 0, 64),
        ('saw down', SW8, 0xE, 48, 0, 2),
        ('sine', SN8, 3, 40, 128, 128),
        ('noise', NZ8, 0, 30, 100, 200),
    ]


def amiga_files():
    smp = amiga_samples()
    # Startrekker FLT8: paired 4 channel patterns
    a = {0: {0: 'C-2 01 F04', 1: 'E-2 02 ...', 2: 'G-2 03 C30', 3: 'C-3 04 ...'},
         2: {0: '--- .. A02', 1: '--- .. 102', 3: '--- .. E11'},
         4: {0: 'D-2 01 ...', 2: '--- .. 404', 3: '--- .. E31'},
         6: {1: '--- .. 0C3', 2: '--- .. ED2'}}
    b = {0: {0: 'G-1 03 ...', 1: 'C-2 04 E62', 2: 'E-2 01 ...', 3: 'B-2 02 ...'},
         3: {0: '--- .. 302', 1: '--- .. E83', 3: '--- .. EC2'},
         7: {0: '--- .. D00'}}
    c = {0: {0: 'C-2 02 ...', 2: 'G-2 04 ...'}, 4: {1: 'E-1 03 F03'}}
    d = {0: {3: 'C-3 01 ...'}, 2: {0: 'D-2 03 ...'}, 5: {0: '--- .. D00'}}
    write('flt_flt8.mod', mod_file('flt8 test', smp, [0, 2], [a, b, c, d], magic=b'FLT8'))

    # Soundtracker 2.x: 15 instruments, no magic
    st = smp[:3] + [('st four', s8(sine(128, 32, 70)), 0, 50, 0, 2)]
    hdr = pad('soundtracker', 20)
    for i in range(15):
        if i < len(st):
            name, data, fin, vol, ls, ll = st[i]
            hdr += pad(name, 22) + struct.pack('>HBBHH', len(data) // 2, 0, vol, ls, ll // 2)
        else:
            hdr += pad('', 22) + struct.pack('>HBBHH', 0, 0, 0, 0, 1)
    p0 = {0: {0: 'C-2 01 F05', 1: 'E-2 02 ...', 2: 'G-2 03 ...', 3: 'C-3 04 ...'},
          2: {0: '--- .. 102', 1: '--- .. 201', 3: '--- .. C20'},
          4: {0: 'D-2 01 ...', 2: '--- .. 037'}, 12: {3: '--- .. B01'}}
    p1 = {0: {0: 'G-1 03 ...', 1: 'C-2 04 E01', 2: 'A-2 01 ...'}, 8: {1: 'C-2 02 ...'}, 16: {0: '--- .. D00'}}
    hdr += bytes([2, 0x78]) + pad(bytes([0, 1]), 128)
    write('st_soundtracker.mod', hdr + mod_pattern(4, 64, p0) + mod_pattern(4, 64, p1) +
          b''.join(s[1] for s in st))

    # Ice Tracker: a track table per order
    hdr = pad('ice tracker', 20)
    for i in range(31):
        if i < len(smp):
            name, data, fin, vol, ls, ll = smp[i]
            hdr += pad(name, 22) + struct.pack('>HBBHH', len(data) // 2, 0, vol, ls // 2, ll // 2)
        else:
            hdr += pad('', 22) + struct.pack('>HBBHH', 0, 0, 0, 0, 1)
    tracks = [
        pt_track({0: 'C-2 01 F44', 8: 'E-2 01 ...', 16: '--- .. D00'}),
        pt_track({0: 'G-2 02 ...', 4: '--- .. 302', 12: 'C-3 03 C20'}),
        pt_track({2: 'C-2 04 ...', 10: 'D-2 04 A02'}),
        pt_track({0: 'C-1 03 F03', 6: 'E-1 03 ...', 20: '--- .. D00'}),
    ]
    order = [[0, 1, 2, 2], [3, 2, 1, 0]]
    hdr += bytes([2, len(tracks)]) + b''.join(bytes(o) for o in order) + b'\0' * (4 * (128 - len(order)))
    write('ice_ice.mod', hdr + b'IT10' + b''.join(tracks) + b''.join(s[1] for s in smp))

    # His Master's Noise: FEST magic, a Mupp chip instrument
    hp0 = {0: {0: 'C-2 01 F04', 1: 'E-2 02 ...', 2: 'C-2 05 ...', 3: 'G-2 03 703'},
           4: {0: '--- .. 047', 2: 'E-2 05 ...'},
           8: {1: 'G-2 02 E01', 2: '--- .. C20'},
           12: {0: '--- .. D00'}}
    mupp = b''.join(s8(saw(32, 32, 100 - 3 * w)) for w in range(22)) + bytes(32 * 6)
    mupp += bytes([(i * 3) % 22 for i in range(64)]) + bytes([64 - (i % 32) for i in range(64)])
    mupp = mupp[:1024]
    hdr = pad('hmn test', 20)
    insts = [('square', SQ8, 0, 64, 0, 64), ('saw down', SW8, 0, 48, 0, 2), ('sine', SN8, 0, 40, 128, 128),
             None, None]
    for i in range(31):
        if i == 4:
            hdr += b'Mupp' + bytes([1, 2, 40]) + pad('', 15) + struct.pack('>HBBHH', 0, 0, 60, 0, 1)
        elif i < len(insts) and insts[i] is not None:
            name, data, fin, vol, ls, ll = insts[i]
            hdr += pad(name, 22) + struct.pack('>HBBHH', len(data) // 2, fin, vol, ls // 2, ll // 2)
        else:
            hdr += pad('', 22) + struct.pack('>HBBHH', 0, 0, 0, 0, 1)
    hdr += bytes([1, 0x7F]) + pad(bytes([0]), 128) + b'FEST'
    write('hmn_fest.mod', hdr + mod_pattern(4, 64, hp0) + mupp + SQ8 + SW8 + SN8)


class Para:
    """A file built up in 16-byte paragraphs (for formats with segment pointers)."""

    def __init__(self, data=b''):
        self.data = bytearray(data)

    def align(self):
        while len(self.data) % 16:
            self.data.append(0)
        return len(self.data) // 16

    def add(self, b):
        p = self.align()
        self.data += b
        return p

    def put(self, at, b):
        self.data[at:at + len(b)] = b


def oct_note(note):
    """'C-4' -> (octave, semitone)."""
    return int(note[2]), NOTES.index(note[:2])


def stm_files():
    # Scream Tracker 2.21: 4 channels, 31 instruments
    smp = [('square.smp', u8(square(64, 16, 100)), 0, 64), ('saw.smp', u8(saw(512, 32, 110)), 0, 0xffff),
           ('sine.smp', u8(sine(256, 64, 90)), 64, 256)]
    f = Para(pad('stm test', 20) + b'!Scream!' + bytes([0x1a, 2, 2, 21, 0x60, 2, 64]) + bytes(13))
    ins_at = len(f.data)
    f.data += bytes(32 * 31)
    orders = [0, 1, 0, 99]
    f.data += pad(bytes(orders), 128, b'\x63')

    def stm_ev(note, ins, vol, fx):
        if note is None:
            b0 = 255
        elif note == '===':
            b0 = 254
        else:
            o, s = oct_note(note)
            b0 = ((o - 3) << 4) | s
        fxt, fxp = (0, 0) if fx is None else (int(fx[0], 36) - 9, int(fx[1:], 16))
        return bytes([b0, ((ins & 0x1f) << 3) | (vol & 7), ((vol & 0x78) << 1) | fxt, fxp])

    def stm_pat(cells):
        out = b''
        for r in range(64):
            for c in range(4):
                cell = cells.get(r, {}).get(c)
                out += stm_ev(*cell) if cell else b'\xfb\x00\x00\x00'
        return out

    p0 = {0: {0: ('C-4', 1, 64, 'A50'), 1: ('E-4', 2, 40, None), 2: ('G-4', 3, 64, 'H44')},
          2: {0: (None, 0, 65, 'D04'), 1: (None, 0, 65, 'E08'), 3: ('C-5', 2, 50, 'J37')},
          4: {0: ('G-3', 1, 64, 'G10'), 2: (None, 0, 65, 'I23')},
          6: {1: ('===', 0, 65, None), 3: (None, 0, 65, 'F10')},
          8: {0: ('C-4', 3, 30, 'A30')}, 12: {0: (None, 0, 65, 'C00')}}
    p1 = {0: {0: ('C-5', 1, 64, None), 1: ('G-4', 3, 64, 'D40')},
          4: {2: ('E-4', 2, 64, 'B02')}}
    f.data += stm_pat(p0) + stm_pat(p1)
    for i in range(31):
        hdr = bytes(32)
        if i < len(smp):
            name, data, ls, le = smp[i]
            seg = f.add(s8([b - 128 for b in data]))
            hdr = pad(name, 13) + b'\0' + struct.pack('<HHHHBBHIH', seg, len(data), ls, le, 48 + 8 * i, 0,
                                                      8363 + 1000 * i, 0, 0)
        f.put(ins_at + 32 * i, hdr)
    write('stm_st221.stm', bytes(f.data))


def stx_files():
    # STMIK 0.2 (STM2STX 1.1): S3M-style packed patterns, paragraph pointers
    smp = [('square', s8(square(64, 16, 100)), 0, 64), ('sine', s8(sine(256, 64, 90)), 64, 256)]
    f = Para(bytes(64))

    def stx_pat(cells):
        out = b''
        for r in range(64):
            for c, (note, ins, vol, fx) in sorted(cells.get(r, {}).items()):
                b = c
                ev = b''
                if note is not None:
                    b |= 0x20
                    o, s = oct_note(note) if note != '===' else (0, 0)
                    ev += bytes([254 if note == '===' else (o << 4) | s, ins])
                if vol is not None:
                    b |= 0x40
                    ev += bytes([vol])
                if fx is not None:
                    b |= 0x80
                    ev += bytes([int(fx[0], 36) - 9, int(fx[1:], 16)])
                out += bytes([b]) + ev
            out += b'\0'
        return out

    p0 = {0: {0: ('C-4', 1, 63, 'A60'), 1: ('G-4', 2, None, 'H34')},
          2: {0: (None, 0, None, 'D02'), 2: ('E-4', 1, 40, None)},
          4: {1: (None, 0, None, 'E04'), 3: ('C-5', 2, 50, 'J47')},
          8: {0: ('D-4', 1, None, 'G08'), 2: ('===', 0, None, None)},
          16: {0: (None, 0, None, 'C00')}}
    p1 = {0: {0: ('E-4', 2, None, None), 1: ('C-4', 1, 20, 'F04')}, 8: {2: (None, 0, None, 'B00')}}
    pats = [stx_pat(p0), stx_pat(p1)]
    orders = [0, 1, 0]
    pp_ins_list = []
    ins_hdrs = []
    pp_pat = f.add(b''.join(le16(0) for _ in pats))
    pp_ins = f.add(b''.join(le16(0) for _ in smp))
    pp_chn = f.add(bytes(32) + b''.join(bytes([o]) + bytes(4) for o in orders))
    for i, (name, data, ls, le) in enumerate(smp):
        ins_hdrs.append(len(f.data))
        pp_ins_list.append(f.add(bytes(80)))
    seg = []
    for p in pats:
        seg.append(f.add(p))
    sdata = b''.join(s[1] for s in smp)
    f.data += sdata
    for i, (name, data, ls, le) in enumerate(smp):
        f.put(ins_hdrs[i], bytes([1]) + pad(name, 13) + struct.pack('<HIIIBBBBHH', 0, len(data), ls, le, 50, 0, 0,
                                                                   0, 8363, 0) + bytes(12) + pad(name, 28) +
              b'SCRS')
    for i, s in enumerate(seg):
        f.put(pp_pat * 16 + 2 * i, le16(s))
    for i, s in enumerate(pp_ins_list):
        f.put(pp_ins * 16 + 2 * i, le16(s))
    hdr = pad('stx test', 20) + b'!Scream!' + struct.pack('<HHHHHHHBBHHHHHHHH', 0, 0, pp_pat, pp_ins, pp_chn, 0, 0,
                                                          64, 0x60, 0, 0, len(pats), len(smp), len(orders), 0, 0,
                                                          0) + b'SCRM'
    f.put(0, hdr)
    write('stx_stm2stx.stx', bytes(f.data))


def mtm_files():
    smp = [('square', u8(square(64, 16, 100)), 0, 64, 0, 64, 0),
           ('sine16', u16le(sine(256, 64, 20000)), 128, 512, 2, 60, 1),
           ('saw', u8(decay(saw(512, 32, 110))), 0, 0, 0xf, 48, 0)]

    def mtm_track(cells):
        out = b''
        for r in range(64):
            c = cells.get(r)
            if c is None:
                out += b'\0\0\0'
                continue
            note, ins, fx = c
            n = 0 if note is None else note_index(note) - 25
            fxt, fxp = (0, 0) if fx is None else (int(fx[0], 16), int(fx[1:], 16))
            out += bytes([(n << 2) | (ins >> 4), ((ins & 0xf) << 4) | fxt, fxp])
        return out

    tracks = [
        mtm_track({0: ('C-4', 1, 'F05'), 4: ('E-4', 1, '0C3'), 8: ('G-4', 2, None), 16: (None, 0, 'D10')}),
        mtm_track({0: ('C-3', 3, 'F80'), 6: (None, 0, 'A04'), 12: ('G-3', 2, 'E88')}),
        mtm_track({2: ('E-5', 2, '447'), 10: ('C-5', 1, '305')}),
        mtm_track({0: ('A-4', 1, 'B02'), 3: (None, 0, 'EC2')}),
    ]
    pats = [[1, 2, 3, 0], [4, 0, 2, 1]]
    comment = pad('MTM comment line', 40) + pad('second line', 40) + bytes(40)
    hdr = b'MTM\x10' + pad('mtm test', 20) + struct.pack('<HBBHBBBB', len(tracks), len(pats) - 1, 2, len(comment),
                                                         len(smp), 0, 64, 4)
    hdr += bytes([0, 15, 7, 8] + [0] * 28)
    for name, data, ls, le, fin, vol, attr in smp:
        hdr += pad(name, 22) + struct.pack('<IIIBBB', len(data), ls, le, fin, vol, attr)
    hdr += pad(bytes([0, 1, 0]), 128)
    body = b''.join(tracks) + b''.join(b''.join(le16(t) for t in p + [0] * 28) for p in pats)
    write('mtm_mtm.mtm', hdr + body + comment + b''.join(s[1] for s in smp))


def imf_files():
    hdr = pad('imf test', 32) + struct.pack('<HHHH', 2, 2, 2, 1) + bytes(8) + bytes([5, 130, 56, 48]) + bytes(8)
    hdr += b'IM10'
    for c in range(32):
        hdr += pad('ch%d' % c, 12) + bytes([0, 0, (c * 80) & 0xff, 0 if c < 4 else 2])
    hdr += pad(bytes([0, 1]), 256, b'\xff')

    def imf_pat(rows, cells):
        out = b''
        for r in range(rows):
            for c, (note, ins, fx, f2) in sorted(cells.get(r, {}).items()):
                b = c
                ev = b''
                if note is not None:
                    b |= 0x20
                    if note == '===':
                        nb = 160
                    else:
                        o, s = oct_note(note)
                        nb = (o << 4) | s
                    ev += bytes([nb, ins])
                if fx is not None:
                    b |= 0x80
                    ev += bytes(fx)
                if f2 is not None:
                    b |= 0x40
                    ev += bytes(f2)
                out += bytes([b]) + ev
            out += b'\0'
        return le16(len(out) + 4) + le16(rows) + out

    p0 = imf_pat(32, {0: {0: ('C-4', 1, (1, 4), (0x0c, 48)), 1: ('E-4', 2, None, None)},
                      2: {0: (None, 0, (0x0d, 0x02), None), 2: ('G-4', 1, (0x05, 0x34), (0x0a, 0x40))},
                      4: {1: (None, 0, (0x21, 0x32), None), 3: ('C-5', 2, (0x09, 0x37), None)},
                      8: {0: ('D-4', 1, (0x03, 0x10), None), 2: ('===', 0, None, None)},
                      10: {1: (None, 0, (0x1b, 0x23), (0x12, 4))},
                      12: {3: (None, 0, (0x1f, 0x30), (0x1a, 0))},
                      16: {0: (None, 0, (0x1e, 0x00), None)}})
    p1 = imf_pat(16, {0: {0: ('E-4', 2, (0x02, 140), None), 1: ('C-4', 1, (0x16, 0x80), (0x17, 0x40))},
                      6: {2: ('G-3', 1, (0x10, 0x14), None), 3: (None, 0, (0x0e, 0x0f), None)},
                      10: {0: (None, 0, (0x1d, 0x01), None)}})

    def imf_ins(name, samples, env=None):
        ii = pad(name, 32) + bytes([0] * 60 + [1] * 60) + bytes(8)
        pts = env[0] if env else []
        words = []
        for x, y in pts:
            words += [x, y]
        ii += struct.pack('<32H', *(words + [0] * (32 - len(words)))) + bytes(128)
        ii += bytes([len(pts), env[1] if env else 0, env[2] if env else 0, env[3] if env else 0,
                     env[4] if env else 0, 0, 0, 0]) + bytes(16)
        ii += struct.pack('<HH', 0x100, len(samples)) + b'II10'
        for sname, data, ls, le, rate, vol, pan, flg in samples:
            n = len(data)
            ii += pad(sname, 13) + bytes(3) + struct.pack('<IIIIBB', n, ls, le, rate, vol, pan) + bytes(14)
            ii += bytes([flg]) + bytes(5) + struct.pack('<HI', 0, 0) + b'IS10' + data
        return ii

    i1 = imf_ins('env', [('square', s8(square(64, 16, 100)), 0, 64, 8363, 64, 0x40, 0x09),
                         ('sine', s8(sine(256, 64, 90)), 0, 256, 16726, 50, 0, 0x03)],
                 env=([(0, 64), (8, 32), (20, 48), (40, 0)], 1, 1, 2, 0x07))
    i2 = imf_ins('saw16', [('saw16', s16le(decay(saw(512, 64, 20000))), 0, 0, 22050, 60, 0, 0x04)])
    write('imf_orpheus.imf', hdr + p0 + p1 + i1 + i2)

    # an instrument without samples, the module's only one
    hdr = pad('no samples', 32) + struct.pack('<HHHH', 1, 1, 1, 1) + bytes(8) + bytes([5, 130, 56, 48]) + bytes(8)
    hdr += b'IM10'
    for c in range(32):
        hdr += pad('ch%d' % c, 12) + bytes([0, 0, (c * 80) & 0xff, 0 if c < 2 else 2])
    hdr += pad(bytes([0]), 256, b'\xff')
    p0 = imf_pat(16, {0: {0: ('C-4', 1, (1, 4), None)}, 8: {1: ('E-4', 1, None, None)}})
    write('imf_nosamples.imf', hdr + p0 + imf_ins('empty', []))


def ptm_files():
    smp = [('square', SQ8, 0, 64, 0x05), ('sine16', s16le(sine(256, 64, 20000)), 0, 512, 0x0d | 0x10),
           ('saw', SW8, 0, 0, 0x01)]
    hdr = pad('ptm test', 28) + bytes([0x1a, 0x03, 0x02, 0]) + struct.pack('<HHHHHH', 3, len(smp), 2, 4, 0, 0)
    hdr += b'PTMF' + bytes(16) + bytes([0, 15, 7, 8] + [7] * 28) + pad(bytes([0, 1, 0]), 256, b'\xff')
    f = Para(hdr + bytes(256) + bytes(80 * len(smp)))

    def ptm_pat(cells):
        out = b''
        for r in range(64):
            for c, (note, ins, fx, vol) in sorted(cells.get(r, {}).items()):
                b = c
                ev = b''
                if note is not None:
                    b |= 0x20
                    ev += bytes([254 if note == '===' else note_index(note) - 11, ins])
                if fx is not None:
                    b |= 0x40
                    ev += bytes(fx)
                if vol is not None:
                    b |= 0x80
                    ev += bytes([vol])
                out += bytes([b]) + ev
            out += b'\0'
        return out

    p0 = {0: {0: ('C-4', 1, (0x0f, 5), 63), 1: ('E-4', 2, None, None), 2: ('G-4', 3, (0x0e, 0x80), None)},
          2: {0: (None, 0, (0x0a, 0x04), None), 3: ('C-5', 1, (0x10, 0x30), 40)},
          4: {1: (None, 0, (0x11, 0x23), None), 2: (None, 0, (0x13, 0x14), None)},
          8: {0: ('D-4', 3, (0x12, 0x44), None), 3: ('===', 0, None, None)},
          12: {1: (None, 0, (0x16, 0x21), None)}, 16: {0: (None, 0, (0x0d, 0x00), None)}}
    p1 = {0: {0: ('E-4', 2, (0x03, 0x08), None), 1: ('C-4', 1, (0x15, 0x22), 20)},
          8: {2: (None, 0, (0x0b, 0x02), None)}}
    segs = [f.add(ptm_pat(p0)), f.add(ptm_pat(p1))]
    ofs = []
    for s in smp:
        f.align()
        ofs.append(len(f.data))
        if s[4] & 0x10:
            f.data += delta16le([int.from_bytes(s[1][i:i + 2], 'little', signed=True) for i in range(0, len(s[1]), 2)])
        else:
            f.data += delta8(list(struct.unpack('%db' % len(s[1]), s[1])))
    for i, s in enumerate(segs):
        f.put(608 - 256 + 2 * i, le16(s))
    for i, (name, data, ls, le, typ) in enumerate(smp):
        f.put(608 + 80 * i, bytes([typ]) + pad(name, 12) + struct.pack('<BHHIIIIIIIBB', 48 + 8 * i, 8363 + 2000 * i,
                                                                         0, ofs[i], len(data), ls, le, 0, 0, 0, 0, 0) +
              pad(name, 28) + b'PTMS')
    write('ptm_ptm.ptm', bytes(f.data))


def ult_files():
    smp = [('square', SQ8, 0, 64, 8), ('sine16', s16le(sine(256, 64, 20000)), 64, 256, 12), ('saw', SW8, 0, 0, 0)]
    hdr = b'MAS_UTrack_V004' + pad('ult test', 32) + bytes([2]) + pad('ULT comment', 32) + pad('line two', 32)
    hdr += bytes([len(smp)])
    for i, (name, data, ls, le, bidi) in enumerate(smp):
        hdr += pad(name, 32) + pad(name, 12) + struct.pack('<IIIIBBHH', ls, le, 0, len(data) >> (1 if bidi & 4 else 0),
                                                           200 - 40 * i, bidi, 8363 + 1000 * i, 0)
    hdr += pad(bytes([0, 1, 0]), 256, b'\xff') + bytes([3, 1]) + bytes([0, 15, 5, 10])

    def ult_chan(cells, npat):
        out = b''
        for j in range(64 * npat):
            c = cells.get(j)
            if c is None:
                out += b'\0' * 5
                continue
            note, ins, fx, fxp, f2p = c
            n = 0 if note is None else note_index(note) - 35
            out += bytes([n, ins, fx, f2p, fxp])
        # pack runs of empty events with the 0xfc repeat code
        packed = b''
        i = 0
        while i < len(out):
            ev = out[i:i + 5]
            k = 1
            while i + 5 * k < len(out) and out[i + 5 * k:i + 5 * k + 5] == ev and k < 255:
                k += 1
            if k >= 3:
                packed += b'\xfc' + bytes([k]) + ev
            else:
                packed += ev * k
            i += 5 * k
        return packed

    chans = [
        {0: ('C-4', 1, 0xf0, 0x03, 0), 4: (None, 0, 0xa0, 0x04, 0), 8: ('G-4', 2, 0x30, 0x10, 0),
         14: (None, 0, 0xd0, 0, 0),
         72: (None, 0, 0xd0, 0, 0)},
        {0: ('E-4', 2, 0xb0, 0x0f, 0), 6: (None, 0, 0x4c, 0x44, 0x30), 64: ('C-5', 3, 0x90, 0x02, 0)},
        {2: ('G-3', 3, 0x1c, 0x04, 0x20), 10: (None, 0, 0xe0, 0xc2, 0), 66: ('A-4', 1, 0xf0, 0x90, 0)},
        {16: ('C-3', 1, 0x50, 0x00, 0), 80: (None, 0, 0x2a, 0x08, 0x30)},
    ]
    write('ult_v004.ult', hdr + b''.join(ult_chan(c, 2) for c in chans) + b''.join(s[1] for s in smp))


class BitsLsb:
    """A little-endian bit stream, least significant bit first."""

    def __init__(self):
        self.out = bytearray()
        self.acc = 0
        self.n = 0

    def put(self, v, bits):
        self.acc |= (v & ((1 << bits) - 1)) << self.n
        self.n += bits
        while self.n >= 8:
            self.out.append(self.acc & 0xFF)
            self.acc >>= 8
            self.n -= 8

    def bytes(self):
        out = bytes(self.out) + (bytes([self.acc & 0xFF]) if self.n else b'')
        return out + bytes(8)


def mdl_pack_byte(bs, delta):
    """One byte of Digitrakker's sample packing (method 1)."""
    b = delta & 0xFF
    s = 0
    if b >= 128:
        s = 1
        b ^= 0xFF
    bs.put(s, 1)
    if b < 8:
        bs.put(1, 1)
        bs.put(b, 3)
    else:
        bs.put(0, 1)
        for _ in range((b - 8) // 16):
            bs.put(0, 1)
        bs.put(1, 1)
        bs.put((b - 8) % 16, 4)


def mdl_pack8(samples):
    bs = BitsLsb()
    prev = 0
    for s in samples:
        mdl_pack_byte(bs, (s & 0xFF) - prev)
        prev = s & 0xFF
    data = bs.bytes()
    return le32(len(data)) + data


def mdl_pack16(samples):
    bs = BitsLsb()
    prev = 0
    for s in samples:
        v = s & 0xFFFF
        bs.put(v & 0xFF, 8)
        mdl_pack_byte(bs, (v >> 8) - prev)
        prev = v >> 8
    data = bs.bytes()
    return le32(len(data)) + data


def mdl_files():
    def chunk(cid, data):
        return cid + le32(len(data)) + data

    orders = [0, 1, 0]
    info = pad('mdl test', 32) + pad('composer', 20) + struct.pack('<HHBBB', len(orders), 0, 255, 5, 135)
    info += bytes([0, 64, 32, 127, 0x80]) + bytes(27) + bytes(orders)

    def mdl_track(cells):
        out = b''
        row = 0
        for r in sorted(cells):
            if r > row:
                out += bytes([(r - row - 1) << 2])
            c = cells[r]
            if c == 'repeat':
                out += bytes([(1 << 2) | 1])
                row = r + 2
                continue
            note, ins, vol, fx1, fx2 = c
            flags = 3
            ev = b''
            if note is not None:
                flags |= 0x04
                ev += bytes([0xff if note == '===' else note_index(note) - 11])
            if ins:
                flags |= 0x08
                ev += bytes([ins])
            if vol is not None:
                flags |= 0x10
                ev += bytes([vol])
            if fx1 or fx2:
                flags |= 0x20 | 0x40 | 0x80
                ev += bytes([(fx1[0] if fx1 else 0) | ((fx2[0] if fx2 else 0) << 4), fx1[1] if fx1 else 0,
                             fx2[1] if fx2 else 0])
            out += bytes([flags]) + ev
            row = r + 1
        return le16(len(out)) + out

    tracks = [
        mdl_track({0: ('C-4', 7, 200, (0xf, 4), (0x1, 0x10)), 4: (None, 0, None, (0x3, 0x10), None),
                   8: ('G-4', 9, None, (0x5, 0x37), (0x2, 0x08)), 20: (None, 0, None, (0xd, 0), None)}),
        mdl_track({2: ('E-4', 9, 128, None, (0x4, 0x44)), 3: 'repeat', 10: ('===', 0, None, None, None),
                   12: ('C-5', 7, None, (0x4, 0x46), (0x3, 0x23))}),
        mdl_track({0: ('G-3', 7, None, (0xe, 0x13), (0x5, 0x32)), 6: (None, 0, None, (0x1, 0x08), (0x7, 150)),
                   16: (None, 0, None, (0xb, 0x01), None)}),
    ]
    pa = bytes([2])
    for idx in ([1, 2, 3, 0], [3, 0, 1, 2]):
        pa += bytes([4, 31]) + pad('pattern', 16) + b''.join(le16(t) for t in idx)
    tr = le16(len(tracks)) + b''.join(tracks)
    ii = bytes([2])

    def sub(sid, mapend, vol, venv, pan, penv, fade, vib, fenv):
        return bytes([sid, mapend, vol, venv, pan, penv]) + le16(fade) + bytes(vib) + bytes([0, fenv])
    ii += bytes([7, 2]) + pad('two samples', 32) + sub(1, 47, 200, 0xc0 | 1, 64, 0x40 | 2, 0x200, (4, 8, 0, 0), 0)
    ii += sub(2, 95, 128, 0x40, 0, 0, 0, (0, 0, 0, 0), 0)
    ii += bytes([9, 1]) + pad('packed', 32) + sub(3, 95, 255, 0x40, 128, 0x40, 0x400, (2, 4, 1, 1), 0x80 | 3)
    sq = square(64, 16, 100)
    sn16 = sine(256, 64, 20000)
    sw = decay(saw(512, 32, 110))
    smp = [(1, 'square', 8363, s8(sq), 0, 64, 0x00, s8(sq)),
           (2, 'sine16', 16726, s16le(sn16), 0, 256, 0x01 | 0x08, mdl_pack16(sn16)),
           (3, 'saw packed', 8363, s8(sw), 32, 64, 0x02 | 0x04, mdl_pack8(sw))]
    is_ = bytes([len(smp)])
    sa = b''
    for idx, name, c5, raw, ls, ll, flg, packed in smp:
        is_ += bytes([idx]) + pad(name, 32) + pad('file', 8) + struct.pack('<IIII', c5, len(raw), ls, ll)
        is_ += bytes([64, flg])
        sa += packed
    ve = bytes([1, 1]) + bytes([1, 64, 4, 32, 8, 48, 10, 0]) + bytes(22) + bytes([0x10 | 1, 0x21])
    pe = bytes([1, 2]) + bytes([1, 0, 10, 127, 10, 64]) + bytes(24) + bytes([0, 0])
    fe = bytes([1, 3]) + bytes([1, 64, 5, 80, 5, 48]) + bytes(24) + bytes([0x20, 0x20])
    body = chunk(b'IN', info) + chunk(b'PA', pa) + chunk(b'TR', tr) + chunk(b'II', ii) + chunk(b'VE', ve)
    body += chunk(b'PE', pe) + chunk(b'FE', fe) + chunk(b'IS', is_) + chunk(b'SA', sa)
    write('mdl_v11.mdl', b'DMDL\x11' + body)


def liq_files():
    chn = 4
    orders = [0, 1, 0]
    hdrsz = 109 + 2 * chn + len(orders)
    hdr = b'Liquid Module:' + pad('liquid test', 30) + pad('author', 20) + b'\x1a' + pad('Liquid Tracker', 20, b' ')
    hdr += struct.pack('<HHHHHHIHHHH', 0x100, 5, 120, 0, 0, chn, 0, 2, 3, len(orders), hdrsz)
    hdr += bytes([0, 64, 32, 66]) + bytes([32, 48, 64, 20]) + bytes(orders)

    def liq_pat(rows, chans):
        data = b''
        for c in range(chn):
            cells = chans.get(c, {})
            row = 0
            for r in sorted(cells):
                gap = r - row
                if gap == 1:
                    data += b'\x80'
                elif gap > 1:
                    data += bytes([0xe0, gap - 1])
                note, ins, vol, fx = cells[r]
                flags = 0
                ev = b''
                if note is not None:
                    flags |= 1
                    ev += bytes([0xfe if note == '===' else note_index(note) - 37 + 12])
                if ins:
                    flags |= 2
                    ev += bytes([ins - 1])
                if vol is not None:
                    flags |= 4
                    ev += bytes([vol])
                if fx is not None:
                    flags |= 8 | 0x10
                    ev += fx[0].encode() + bytes([fx[1]])
                data += bytes([0xc0 | flags]) + ev
                row = r + 1
            if c < chn - 1:
                data += b'\xa0'
        data += b'\xc0'
        return b'LP\0\0' + pad('pattern', 30) + struct.pack('<HII', rows, len(data), 0) + data

    p0 = liq_pat(32, {0: {0: ('C-4', 1, 63, ('S', 0x04)), 4: (None, 0, None, ('L', 0x0f)), 8: ('E-4', 2, 40, None),
                          12: (None, 0, None, ('N', 0x08)), 16: ('G-4', 1, None, ('C', 0x00))},
                      1: {0: ('G-3', 2, None, ('V', 0x44)), 6: ('===', 0, None, None), 10: ('C-4', 3, 50, ('P', 0x32))},
                      2: {2: ('E-5', 3, None, ('G', 0x48)), 9: (None, 0, None, ('M', 0xc3))},
                      3: {1: ('C-3', 1, 30, ('R', 0x03)), 5: (None, 0, None, ('T', 0x24))}})
    p1 = liq_pat(16, {0: {0: ('D-4', 2, None, ('U', 0x08)), 4: (None, 0, None, ('D', 0x04))},
                      2: {0: ('A-4', 1, None, ('X', 0x02)), 8: (None, 0, None, ('B', 140))}})

    def ldss(name, data, ls, le, c2spd, vol, flags, pan, loop_type, version=0x101):
        h = b'LDSS' + struct.pack('<H', version) + pad(name, 30) + pad('editor', 20) + pad('author', 20) + b'\0'
        h += struct.pack('<IIIIBBBBBBHHIBB', len(data), ls, le, c2spd, vol, flags, pan, 0, 48, 0, 0x90, 0, 0, 0,
                         loop_type)
        return h + bytes(10) + pad('file.smp', 25) + data

    ins = ldss('square', SQ8, 0, 64, 8363, 64, 0x04, 32, 0)
    ins += ldss('sine16', s16le(sine(256, 64, 20000)), 64, 512, 16726, 48, 0x05, 255, 1)
    ins += ldss('stereo', s8(saw(128, 32, 90)) + s8(sine(128, 32, 90)), 0, 0, 8363, 56, 0x06, 64, 0, version=0x100)
    write('liq_liquid.liq', hdr + p0 + p1 + ins)


def no_files():
    hdr = b'NO\0\0' + bytes([7]) + pad('no test', 29) + bytes([2, 0, 4, 0]) + struct.pack('<HH', 0, 0) + b'\0'
    hdr += pad(bytes([0, 1, 0, 0xff]), 256)
    smp = [('square', u8(square(64, 16, 100)), 8363, 0, 64, 64), ('saw', u8(decay(saw(512, 32, 110))), 8363, 0, 0, 48),
           ('sine', u8(sine(256, 64, 90)), 16726, 64, 256, 40)]
    for i in range(63):
        if i < len(smp):
            name, data, c2, ls, le, vol = smp[i]
            hdr += bytes([len(name)]) + pad(name, 30) + bytes([vol]) + struct.pack('<HIII', c2, len(data), ls, le)
        else:
            hdr += bytes(46)

    def no_pat(cells):
        out = b''
        for r in range(64):
            for c in range(4):
                cell = cells.get(r, {}).get(c)
                if cell is None:
                    note, ins, vol, fxt, fxp = 0x3f, 0x7f, 0x7f, 0xf, 0
                else:
                    note, ins, vol, fx = cell
                    note = 0x3f if note is None else note_index(note) - 37 + 12
                    ins = 0x7f if ins == 0 else ins - 1
                    vol = 0x7f if vol is None else vol - 1
                    fxt, fxp = (0xf, 0) if fx is None else fx
                out += le32(note | (ins << 6) | (vol << 13) | (fxt << 20) | (fxp << 24))
        return out

    p0 = {0: {0: ('C-4', 1, 64, (0x0, 0x04)), 1: ('E-4', 2, None, (0x1, 0x44)), 2: ('G-4', 3, 40, None)},
          4: {0: (None, 0, None, (0xb, 0x08)), 3: ('C-5', 1, None, (0x7, 0x32))},
          8: {1: ('G-3', 3, None, (0xc, 0x03)), 2: (None, 0, None, (0x8, 0x04))},
          12: {0: ('D-4', 2, None, (0xd, 0x10)), 3: (None, 0, None, (0xc, 0x21))},
          16: {0: (None, 0, None, (0x2, 0x00))}}
    p1 = {0: {0: ('E-4', 1, None, (0x4, 0x08)), 2: ('A-3', 3, None, (0x6, 0x37))},
          4: {1: (None, 0, None, (0xc, 0x82))}, 8: {0: (None, 0, None, (0x2, 0x00))}}
    write('no_liquid.no', hdr + no_pat(p0) + no_pat(p1) + b''.join(s[1] for s in smp))


def masi_files():
    def ch(cid, data):
        return iff(cid, data, le=True)

    def epic_note(note):
        o, s = oct_note(note)
        return ((o - 1) << 4) | s

    def pbod(name, rows, cells):
        body = name + le16(rows)
        for r in range(rows):
            row = b''
            for c, (note, ins, vol, fx) in sorted(cells.get(r, {}).items()):
                flag = 0
                ev = b''
                if note is not None:
                    flag |= 0x80
                    ev += bytes([epic_note(note)])
                if ins:
                    flag |= 0x40
                    ev += bytes([ins - 1])
                if vol is not None:
                    flag |= 0x20
                    ev += bytes([vol])
                if fx is not None:
                    flag |= 0x10
                    ev += bytes(fx)
                row += bytes([flag, c]) + ev
            body += le16(len(row) + 2) + row
        return ch(b'PBOD', le32(len(body) + 4) + body)

    def dsmp(flags, name, data, ls, le, vol, rate):
        d = bytes([flags]) + pad('songname', 8) + pad(name[:4], 4) + pad(name, 31) + bytes(8) + bytes([0, 0])
        d += struct.pack('<IIIHBIH', len(data), ls, le, 0, vol, 0, rate) + bytes(16)
        return ch(b'DSMP', d + delta8(list(struct.unpack('%db' % len(data), data))))

    oplh = bytes([0x07, 5, 0x08, 130]) + b''.join(bytes([1]) + n for n in (b'P0  ', b'P1  ', b'P0  '))
    oplh += bytes([0x0d, 0, 0x40, 0, 0x0d, 1, 0xc0, 0, 0x0d, 2, 0, 2, 0x0e, 3, 128]) + b'\0'
    oplh = le16(10) + oplh
    song = pad('MAINSONG', 9) + bytes([1, 4]) + iff(b'DATE', b'941004', le=True) + iff(b'OPLH', oplh, le=True)
    p0 = pbod(b'P0  ', 32, {0: {0: ('C-4', 1, 128, (0x3d, 4)), 1: ('E-4', 2, None, None),
                                2: ('G-4', 3, 64, (0x15, 0x34))},
                            2: {0: (None, 0, None, (0x02, 0x08)), 3: ('C-5', 1, 100, (0x49, 0x0c))},
                            4: {1: (None, 0, None, (0x0c, 0x10)), 2: (None, 0, None, (0x04, 0x06))},
                            8: {0: ('D-4', 3, None, (0x0f, 0x20)), 3: (None, 0, None, (0x2b, 0x03))},
                            10: {1: ('A-4', 2, None, (0x29, 0x00, 0x01, 0x00))},
                            12: {2: (None, 0, None, (0x47, 0x37))}})
    p1 = pbod(b'P1  ', 16, {0: {0: ('E-4', 2, None, (0x0e, 0x03)), 1: ('C-4', 1, 64, (0x1f, 0x44))},
                            8: {2: ('G-3', 3, None, (0x33, 0x00, 0x00))}})
    smp = dsmp(0x80, 'square', SQ8, 0, 64, 255, 8363) + dsmp(0, 'saw', SW8, 0, 0, 200, 8363)
    smp += dsmp(0x80, 'sine', SN8, 64, 256, 180, 16726)
    body = ch(b'SDFT', b'MAINSONG') + ch(b'TITL', b'masi test') + ch(b'SONG', song) + smp + p0 + p1
    write('masi_psm.psm', b'PSM ' + le32(len(body) + 4) + b'FILE' + body)


def masi16_files():
    chn = 4
    orders = [0, 1, 0]

    def pat(rows, cells):
        out = b''
        for r in range(rows):
            for c, (note, ins, vol, fx) in sorted(cells.get(r, {}).items()):
                b = c
                ev = b''
                if note is not None:
                    b |= 0x80
                    ev += bytes([note_index(note) - 36 + 13, ins])
                if vol is not None:
                    b |= 0x40
                    ev += bytes([vol - 1])
                if fx is not None:
                    b |= 0x20
                    ev += bytes(fx)
                out += bytes([b]) + ev
            out += b'\0'
        return le16(len(out) + 4) + bytes([rows, chn]) + out

    pats = pat(32, {0: {0: ('C-4', 1, 64, (60, 4)), 1: ('E-4', 2, None, None), 2: ('G-4', 3, 40, (20, 0x34))},
                    2: {0: (None, 0, None, (2, 0x08)), 3: ('C-5', 1, 50, (72, 0x0c))},
                    4: {1: (None, 0, None, (11, 0x10)), 2: (None, 0, None, (4, 0x06))},
                    8: {0: ('D-4', 3, None, (14, 0x20)), 3: (None, 0, None, (42, 0x03))},
                    12: {2: (None, 0, None, (70, 0x37)), 1: (None, 0, None, (61, 140))},
                    16: {0: (None, 0, None, (51, 0x00))}})
    pats += pat(16, {0: {0: ('E-4', 2, None, (13, 0x03)), 1: ('C-4', 1, 64, (30, 0x44))},
                     8: {2: ('G-3', 3, None, (22, 0x04))}})
    smp = [('square', SQ8, 0, 64, 0x80), ('saw', SW8, 0, 0, 0), ('sine', SN8, 64, 256, 0xa0)]
    p_ord = 98
    p_chn = p_ord + len(orders)
    p_ins = p_chn + 16
    p_pat = p_ins + 64 * len(smp)
    p_smp = p_pat + len(pats)
    hdr = b'PSM\xfe' + pad('masi16 test', 60) + bytes([0, 0x10, 0, 5, 125, 64])
    hdr += struct.pack('<HHHHHHIIII', 0, len(orders), 2, len(smp), chn, chn, p_ord, p_chn, p_pat, p_ins)
    hdr += bytes(orders) + bytes([0, 15, 7, 8] + [7] * 12)
    sdata = b''
    for i, (name, data, ls, le, flags) in enumerate(smp):
        hdr += pad('file%d.smp' % i, 13) + pad(name, 24) + struct.pack('<IIH', p_smp + len(sdata), 0, i + 1)
        hdr += bytes([flags]) + struct.pack('<III', len(data), ls, le) + bytes([0x70 + i, 60 - 8 * i]) + le16(8363)
        sdata += delta8(list(struct.unpack('%db' % len(data), data)))
    write('masi16_psm16.psm', hdr + pats + sdata)


def gal_patt(num, rows, cells, fx_first=True):
    data = bytes([rows - 1])
    for r in range(rows):
        for c, (note, ins, vol, fx) in sorted(cells.get(r, {}).items()):
            flag = c
            ev = b''
            if fx is not None:
                flag |= 0x80
                ev += bytes([fx[1], fx[0]])
            if note is not None:
                flag |= 0x40
                ev += bytes([ins, 128 if note == '===' else note_index(note) + 1])
            if vol is not None:
                flag |= 0x20
                ev += bytes([vol])
            data += bytes([flag]) + ev
        data += b'\0'
    return bytes([num]) + le32(len(data)) + data


GAL_P0 = {0: {0: ('C-4', 1, 128, (0x14, 4)), 1: ('E-4', 2, None, None), 2: ('G-4', 1, 64, (0x04, 0x34))},
          2: {0: (None, 0, None, (0x0a, 0x08)), 3: ('C-5', 2, 100, (0x08, 0xc0))},
          4: {1: (None, 0, None, (0x01, 0x10)), 2: ('===', 0, None, None)},
          8: {0: ('D-4', 2, None, (0x03, 0x20)), 3: (None, 0, None, (0x0e, 0xc3))},
          16: {0: (None, 0, None, (0x0d, 0x00))}}
GAL_P1 = {0: {0: ('E-4', 2, None, (0x02, 0x03)), 1: ('C-4', 1, 64, (0x07, 0x44))},
          8: {2: ('G-3', 1, None, (0x0b, 0x00))}}


def gal5_file():
    def ch(cid, data):
        return iff(cid, data, le=True, align=True)

    init = pad('gal5 test', 64) + bytes([0, 4, 5, 125]) + le16(0x1c5) + le16(0xff00) + b'\x80'
    init += bytes([0, 64, 32, 127] + [64] * 60)
    ordr = bytes([2, 0, 1, 0])

    def inst(num, name, data, ls, le, flags, rate, vol):
        samp = le32(0x40000000) + pad(name, 28) + le32(0) + b'\0' + bytes([vol])
        samp += struct.pack('<HHHIIIIII', 0x7fff, flags, 0x80, len(data) >> (1 if flags & 4 else 0),
                            ls, le, rate, 0, 0) + data
        riff = b'RIFF' + le32(len(samp) + 12) + b'AS  ' + b'SAMP' + le32(len(samp)) + samp
        return ch(b'INST', le32(0x142) + bytes([0, num]) + pad(name, 28) + bytes(290) + le16(1) + riff)

    body = ch(b'INIT', init) + ch(b'ORDR', ordr) + ch(b'PATT', gal_patt(0, 32, GAL_P0))
    body += ch(b'PATT', gal_patt(1, 16, GAL_P1))
    body += inst(0, 'square', SQ8, 0, 64, 0x08 | 0x80, 8363, 64)
    body += inst(1, 'sine16', s16le(sine(256, 64, 20000)), 0, 256, 0x04 | 0x10 | 0x80, 16726, 48)
    return b'RIFF' + le32(len(body) + 4) + b'AM  ' + body


def gal4_file(envloop=False):
    def ch(cid, data):
        return iff(cid, data, le=True)

    main = pad('gal4 env loops' if envloop else 'gal4 test', 64) + bytes([0, 4, 5, 125]) + le16(0x1c5) + le16(0xff00) + b'\x80'
    ordr = bytes([2, 0, 1, 0])

    def env(points):
        b = b''
        for x, y in points:
            b += le16(x * 16) + bytes([y])
        return b + bytes(30 - len(b))

    def inst(num, name, samples, envflags=0, venv=(), penv=(), loops=(0x11, 0x00, 0x22)):
        d = bytes([0, num]) + pad(name, 28) + bytes([len(samples)]) + bytes([0] * 54 + [len(samples) - 1] * 54)
        d += bytes(11) + bytes([0, 4, 0, 0, 8]) + le16(16 * 4) + b'\0'
        d += bytes([envflags, (max(len(venv), 1) - 1) | ((max(len(penv), 1) - 1) << 4)]) + bytes(loops)
        d += env(venv) + env(penv) + b'\x02\0'
        for sname, data, ls, le, flags, rate, vol, pan in samples:
            s = pad(sname, 28) + bytes([pan, vol, flags, 0x80])
            s += struct.pack('<IIIIII', len(data) >> (1 if flags & 4 else 0), ls, le, rate, 0, 0) + data
            d += b'SAMP' + le32(len(s)) + s
        return ch(b'INST', d)

    if envloop:
        # volume and pan envelopes looping from their point 1 to 3 (the
        # sustain point 0 left alone)
        body = ch(b'MAIN', main) + ch(b'ORDR', bytes([0, 0]))
        body += ch(b'PATT', gal_patt(0, 32, {0: {0: ('C-4', 1, 64, None)}, 24: {0: ('E-4', 1, None, None)}}))
        body += inst(0, 'looped envs', [('sine16', s16le(sine(256, 64, 20000)), 0, 256, 0x04 | 0x18, 16726, 64, 0)],
                     envflags=0x55, venv=[(0, 64), (4, 8), (10, 60), (16, 20), (30, 0)],
                     penv=[(0, 32), (6, 0), (12, 64), (18, 16)], loops=(0x00, 0x11, 0x33))
        return b'RIFF' + le32(len(body) + 4) + b'AMFF' + body
    body = ch(b'MAIN', main) + ch(b'ORDR', ordr) + ch(b'PATT', gal_patt(0, 32, GAL_P0))
    body += ch(b'PATT', gal_patt(1, 16, GAL_P1))
    body += inst(0, 'two samples', [('square', SQ8, 0, 64, 0x08, 8363, 64, 0),
                                    ('saw', SW8, 0, 0, 0x20, 8363, 50, 48)],
                 envflags=0x13, venv=[(0, 64), (8, 40), (20, 50), (40, 0)], penv=[(0, 32), (10, 0), (20, 64)])
    body += inst(1, 'sine16', [('sine16', s16le(sine(256, 64, 20000)), 0, 256, 0x04 | 0x18, 16726, 48, 0)])
    return b'RIFF' + le32(len(body) + 4) + b'AMFF' + body


def gal_files():
    import zlib
    write('gal5_j2b.j2b', gal5_file())
    g4 = gal4_file()
    write('gal4_j2b.j2b', g4)
    write('gal4_envloop.j2b', gal4_file(envloop=True))
    write('muse_gal4.j2b', b'MUSE' + be32(0xdeadbeaf) + le32(len(g4)) + bytes(12) + zlib.compress(g4, 9))


def amf_files():
    # DSMI AMF 1.4: tracks of (row, command, parameter) triples
    chn = 4
    smp = [('square', u8(square(64, 16, 100)), 8363, 64, 0, 64), ('saw', u8(decay(saw(512, 32, 110))), 8363, 48, 0, 0),
           ('sine', u8(sine(256, 64, 90)), 16726, 40, 64, 256)]

    def track(evs):
        data = b''
        for r, cmds in sorted(evs.items()):
            for cmd, par in cmds:
                data += bytes([r, cmd, par & 0xff])
        return le16(len(data) // 3) + b'\0' + data

    def note(n, vol=0xff):
        return (note_index(n) - 1, vol)

    tracks = [
        track({0: [(0x80, 0), note('C-4', 64), (0x81, 5)], 4: [(0x82, 0xfc)], 8: [(0x80, 1), note('E-4')],
               12: [(0x86, 0x10)], 20: [(0x8c, 0)]}),
        track({0: [(0x80, 2), note('G-4', 40), (0x89, 0x44)], 6: [(0x84, 0x08)], 10: [(0x97, 0x20)],
               14: [(0x80, 0), note('C-5'), (0x88, 0x37)]}),
        track({2: [(0x80, 1), note('E-3'), (0x87, 0x24)], 8: [(0x83, 30)], 12: [(0x94, 0x02)],
               16: [(0x8f, 0x03)]}),
        track({0: [(0x80, 0), note('A-4'), (0x84, 0xf8)], 4: [(0x91, 0x02)], 8: [(0x95, 140)],
               10: [(0x8d, 0)]}),
    ]
    pats = [[1, 2, 3, 0], [4, 3, 0, 2], [1, 0, 2, 3]]

    def amf(title, tracks, pats):
        hdr = b'AMF\x0e' + pad(title, 32) + bytes([len(smp), len(pats)]) + le16(len(tracks)) + bytes([chn])
        hdr += bytes([0xe0, 0x20, 0x00, 0x10] + [0] * 28) + bytes([125, 6])
        for p in pats:
            hdr += le16(32) + b''.join(le16(t) for t in p)
        for name, data, c2, vol, ls, le in smp:
            hdr += b'\x01' + pad(name, 32) + pad('file.smp', 13) + le32(0) + le32(len(data)) + le16(c2)
            hdr += bytes([vol]) + le32(ls) + le32(le)
        hdr += b''.join(le16(i + 1) for i in range(len(tracks)))
        return hdr + b''.join(tracks) + b''.join(s[1] for s in smp)

    write('amf_dsmi.amf', amf('amf test', tracks, pats))
    # fine (0x92) and extra fine (0x96) pitch slides, down (> 0) and up
    fine = [
        track({0: [(0x80, 2), note('C-5')], 2: [(0x92, 0x03)], 4: [(0x92, 0x05)], 6: [(0x92, 0xfd)],
               8: [(0x92, 0xfa)], 10: [(0x92, 0x00)]}),
        track({0: [(0x80, 2), note('G-4')], 2: [(0x96, 0x06)], 4: [(0x96, 0x0f)], 6: [(0x96, 0xfc)],
               8: [(0x96, 0xf1)]}),
    ]
    write('amf_fineslide.amf', amf('fine slides', fine, [[1, 2, 0, 0]]))


def asylum_files():
    orders = [0, 1, 0]
    hdr = pad(b'ASYLUM Music Format V1.0', 32) + bytes([5, 125, 3, 2, len(orders), 0]) + pad(bytes(orders), 256)
    smp = [('square', SQ8, 0, 64, 0, 0, 64), ('saw', SW8, 2, 48, 12, 0, 0), ('sine', SN8, 0, 40, -12, 64, 192)]
    for i in range(64):
        if i < len(smp):
            name, data, fin, vol, xpo, ls, ll = smp[i]
            hdr += pad(name, 22) + bytes([fin, vol, xpo & 0xff]) + struct.pack('<III', len(data), ls, ll)
        else:
            hdr += bytes(37)

    def pat(cells):
        out = b''
        for r in range(64):
            for c in range(8):
                cell = cells.get(r, {}).get(c)
                if cell is None:
                    out += bytes(4)
                    continue
                note, ins, fxt, fxp = cell
                out += bytes([0 if note is None else note_index(note) - 12, ins, fxt, fxp])
        return out

    p0 = {0: {0: ('C-4', 1, 0x0f, 5), 1: ('E-4', 2, 0, 0), 4: ('G-4', 3, 0x04, 0x34), 7: ('C-5', 1, 0x0c, 0x20)},
          4: {0: (None, 0, 0x0a, 0x04), 2: ('C-3', 3, 0x03, 0x10)},
          8: {1: (None, 0, 0x01, 0x08), 5: ('E-3', 2, 0x1b, 0x23)},
          16: {0: (None, 0, 0x0d, 0x00)}}
    p1 = {0: {0: ('D-4', 2, 0x0e, 0x91), 3: ('A-3', 1, 0x08, 0x80)}, 8: {6: (None, 0, 0x0b, 0x02)}}
    write('asylum_amf.amf', hdr + pat(p0) + pat(p1) + b''.join(s[1] for s in smp))


def gdm_files():
    smp = [('square', u8(square(64, 16, 100)), 0, 64, 0x01, 8363, 64, 255),
           ('saw', u8(decay(saw(512, 32, 110))), 0, 0, 0, 8363, 50, 4),
           ('sine16', u16le(sine(256, 64, 20000)), 128, 512, 0x03, 16726, 60, 12)]

    def gdm_note(n):
        o, s = oct_note(n)
        return (o << 4) | s

    def pat(cells):
        out = b''
        for r in range(64):
            for c, (note, ins, fxs) in sorted(cells.get(r, {}).items()):
                b = c
                ev = b''
                if note is not None:
                    b |= 0x20
                    ev += bytes([gdm_note(note), ins])
                if fxs:
                    b |= 0x40
                    for k, (slot, fxt, fxp) in enumerate(fxs):
                        more = 0x20 if k < len(fxs) - 1 else 0
                        ev += bytes([(slot << 6) | more | fxt, fxp])
                out += bytes([b]) + ev
            out += b'\0'
        return le16(len(out) + 2) + out

    pats = pat({0: {0: ('C-4', 1, [(0, 0x0f, 4), (1, 0x1f, 130)]), 1: ('E-4', 2, []), 2: ('G-4', 3, [(0, 0x04, 0x34)])},
                2: {0: (None, 0, [(0, 0x0a, 0x04)]), 3: ('C-5', 1, [(0, 0x1e, 0x84)])},
                4: {1: (None, 0, [(0, 0x0e, 0x13)]), 2: (None, 0, [(0, 0x08, 0x23)])},
                8: {0: ('D-4', 3, [(0, 0x03, 0x20), (1, 0x13, 48)]), 3: (None, 0, [(0, 0x12, 0x33)])},
                12: {2: (None, 0, [(0, 0x10, 0x37), (2, 0, 0)])}, 16: {0: (None, 0, [(0, 0x0d, 0x00)])}})
    pats += pat({0: {0: ('E-4', 2, [(0, 0x02, 0x03)]), 1: ('C-4', 1, [(0, 0x14, 0x44)])},
                 8: {2: ('G-3', 3, [(0, 0x0e, 0xa2)])}, 12: {1: (None, 0, [(0, 0x0b, 0x00)])}})
    orders = [0, 1, 0]
    ord_ofs = 160
    ins_ofs = ord_ofs + len(orders)
    pat_ofs = ins_ofs + 62 * len(smp)
    smp_ofs = pat_ofs + len(pats)
    hdr = b'GDM\xfe' + pad('gdm test', 32) + pad('author', 32) + b'\r\n\x1aGMFS' + bytes([1, 0]) + le16(0)
    hdr += bytes([1, 10]) + bytes([0, 15, 8, 255] + [8] * 28) + bytes([64, 5, 125]) + le16(1)
    hdr += le32(ord_ofs) + bytes([len(orders) - 1]) + le32(pat_ofs) + bytes([1]) + le32(ins_ofs) + le32(smp_ofs)
    hdr += bytes([len(smp) - 1])
    hdr = pad(hdr, ord_ofs) + bytes(orders)
    for name, data, ls, le, flg, c4, vol, pan in smp:
        hdr += pad(name, 32) + pad('file.smp', 12) + b'\0' + struct.pack('<IIIBHBB', len(data), ls, le, flg, c4,
                                                                         vol, pan)
    write('gdm_2gdm.gdm', hdr + pats + b''.join(s[1] for s in smp))


class Blob:
    """A big-endian file with pointers patched in once things are placed."""

    def __init__(self):
        self.data = bytearray()

    def add(self, b, align=2):
        while len(self.data) % align:
            self.data.append(0)
        at = len(self.data)
        self.data += b
        return at

    def put32(self, at, v):
        self.data[at:at + 4] = be32(v)


def med_synth(wforms, voltbl, wftbl, decay=0, volspeed=2, wfspeed=3):
    """A MED synthetic instrument (type -1) with its waveforms."""
    hdr = be16(0xffff) + bytes([decay, 0, 0, 0]) + be16(0) + be16(0) + be16(len(voltbl)) + be16(len(wftbl))
    hdr += bytes([volspeed, wfspeed]) + be16(len(wforms)) + pad(bytes(voltbl), 128) + pad(bytes(wftbl), 128)
    offs = []
    body = b''
    base = 4 + len(hdr) + 4 * len(wforms)
    for w in wforms:
        offs.append(base + len(body))
        body += be16(len(w) // 2) + w
    data = hdr + b''.join(be32(o) for o in offs) + body
    return be32(len(data) - 2) + data


def med_sample(data, typ=0):
    return be32(len(data)) + be16(typ) + data


def mmd_file(ident, song_samples, blocks, orders, instruments, deftempo=125, tempo2=6, flags=0, flags2=0,
             names=(), exp_smp=(), exp_size=4, annotation=b'', mmdinfo=b'', trackvols=None, trackpans=None,
             songname=b''):
    """MMD0-3. song_samples: (rep, replen, svol, strans) per instrument (in words); blocks: (tracks, lines,
    {row: {track: (note, ins, cmd, param)}}); instruments: the encoded instrument data or None."""
    ver = ident[3] - ord('0')
    f = Blob()
    f.add(bytes(52))
    smp = b''
    for i in range(63):
        rep, replen, svol, strans = song_samples[i] if i < len(song_samples) else (0, 0, 0, 0)
        smp += be16(rep) + be16(replen) + bytes([0, 0, svol, strans & 0xff])
    if ver < 2:
        song = smp + be16(len(blocks)) + be16(len(orders)) + pad(bytes(orders), 256)
        song += be16(deftempo) + bytes([0, flags, flags2, tempo2]) + bytes(trackvols or [64] * 16)
        song += bytes([64, len(song_samples)])
        song_at = f.add(song)
        seq_fix = None
    else:
        song = smp + be16(len(blocks)) + be16(0) + be32(0) + be32(0) + be32(0) + be16(0)
        song += be16(1) + be32(0) + be32(0) + be16(0) + be16(0) + bytes([0, 0]) + be16(0) + b'\0' + bytes(223)
        song += be16(deftempo) + bytes([0, flags, flags2, tempo2]) + bytes(16) + bytes([64, len(song_samples)])
        song_at = f.add(song)
        seq_fix = song_at + 504 + 4
    f.put32(4, 0)
    f.put32(8, song_at)
    if seq_fix is not None:
        playseq = f.add(pad('sequence', 32) + be32(0) + be32(0) + be16(len(orders)) + b''.join(be16(o) for o in orders))
        seqtable = f.add(be32(playseq))
        f.put32(seq_fix, seqtable)
        tv = f.add(bytes(trackvols or [64] * 8))
        f.put32(seq_fix + 8, tv)
        f.data[seq_fix + 12:seq_fix + 14] = be16(len(trackvols or [64] * 8))
        if trackpans is not None:
            tp = f.add(bytes(p & 0xff for p in trackpans))
            f.put32(seq_fix + 16, tp)
    barr = f.add(bytes(4 * len(blocks)))
    f.put32(16, barr)
    sarr = f.add(bytes(4 * len(song_samples)))
    f.put32(24, sarr)
    exp = f.add(bytes(64))
    f.put32(32, exp)
    if exp_smp:
        at = f.add(b''.join(pad(e, exp_size) for e in exp_smp))
        f.put32(exp + 4, at)
        f.data[exp + 8:exp + 12] = be16(len(exp_smp)) + be16(exp_size)
    if annotation:
        at = f.add(annotation)
        f.put32(exp + 12, at)
        f.put32(exp + 16, len(annotation))
    if names:
        at = f.add(b''.join(pad(n, 40) for n in names))
        f.put32(exp + 20, at)
        f.data[exp + 24:exp + 28] = be16(len(names)) + be16(40)
    if songname:
        at = f.add(songname + b'\0')
        f.put32(exp + 44, at)
        f.put32(exp + 48, len(songname) + 1)
    if mmdinfo:
        at = f.add(be32(0) + be16(0) + be16(1) + be32(len(mmdinfo)) + mmdinfo)
        f.put32(exp + 56, at)
    for b, (tracks, lines, cells) in enumerate(blocks):
        if ver == 0:
            data = bytes([tracks, lines - 1])
        else:
            data = be16(tracks) + be16(lines - 1) + be32(0)
        for r in range(lines):
            for t in range(tracks):
                c = cells.get(r, {}).get(t)
                note, ins, cmd, par = (None, 0, 0, 0) if c is None else c
                n = 0 if note is None else note_index(note) - 23
                if ver == 0:
                    data += bytes([n | ((ins & 0x10) << 3) | ((ins & 0x20) << 1), ((ins & 0xf) << 4) | cmd, par])
                else:
                    data += bytes([n, ins, cmd, par])
        f.put32(barr + 4 * b, f.add(data))
    for i, ins in enumerate(instruments):
        if ins is not None:
            f.put32(sarr + 4 * i, f.add(ins))
    f.data[0:4] = ident
    f.put32(4, len(f.data))
    return bytes(f.data)


def med_files():
    wf = [s8(saw(32, 32, 100)), s8(square(64, 16, 90)), s8(sine(32, 32, 100))]
    synth = med_synth(wf, [64, 60, 0xf1, 2, 50, 40, 0xff], [0xf0, 2, 0, 1, 2, 0xfe, 2])
    sq = med_sample(SQ8)
    sw = med_sample(SW8)
    b0 = (4, 32, {0: {0: ('C-3', 1, 0x0c, 0x40), 1: ('E-3', 2, 0, 0), 2: ('G-3', 3, 0x04, 0x34)},
                  2: {0: (None, 0, 0x0d, 0x04), 3: ('C-4', 1, 0x01, 0x08)},
                  4: {1: (None, 0, 0x03, 0x10), 2: ('C-2', 2, 0x09, 0x03)},
                  8: {0: ('D-3', 3, 0x00, 0x37), 3: (None, 0, 0x0f, 0xf1)},
                  12: {2: ('A-3', 1, 0x1f, 0x03)}, 16: {0: (None, 0, 0x0f, 0x00)}})
    b1 = (4, 16, {0: {0: ('E-3', 2, 0x02, 0x04), 1: ('C-3', 3, 0x0f, 0x50)},
                  8: {2: ('G-2', 1, 0x0b, 0x00)}})
    write('mmd0_med.med', mmd_file(b'MMD0', [(0, 32, 64, 0), (0, 0, 48, 0), (0, 0, 64, 12)], [b0, b1], [0, 1, 0],
                                   [sq, sw, synth], tempo2=4, names=[b'square', b'saw', b'synth'],
                                   exp_smp=[b'\x02\x04\x00\x00', b'\x00\x00\x00\x03', b''],
                                   annotation=b'MED annotation', songname=b'mmd0 test'))

    # MMD1: BPM mode, a 16-bit sample, a two-octave IFFOCT instrument, 8 channel mode off, hex volumes
    s16 = med_sample(b''.join(struct.pack('>h', v) for v in sine(256, 64, 20000)), typ=0x10)
    oct2 = med_sample(s8(saw(96, 32, 100)) + s8(saw(192, 64, 100)), typ=3)
    c0 = (6, 32, {0: {0: ('C-3', 1, 0x0c, 0x30), 1: ('E-3', 2, 0x19, 0x01), 4: ('G-3', 3, 0x16, 0x02)},
                  4: {0: (None, 0, 0x11, 0x02), 5: ('C-4', 4, 0x14, 0x24)},
                  8: {1: (None, 0, 0x1a, 0x03), 2: ('C-2', 3, 0x18, 0x02), 3: ('E-4', 1, 0x2e, 0x08)},
                  12: {0: ('D-3', 2, 0x1d, 0x00)}})
    c1 = (6, 16, {0: {0: ('E-3', 4, 0x0f, 0x80), 2: ('C-3', 1, 0x1b, 0x02)}, 10: {5: ('G-2', 3, 0x0f, 0xff)}})
    write('mmd1_octamed.med', mmd_file(b'MMD1', [(0, 32, 64, 0), (32, 96, 48, 0), (0, 0, 64, 0), (0, 0, 64, 0)],
                                       [c0, c1], [0, 1], [sq, s16, oct2, synth], deftempo=130, tempo2=6,
                                       flags=0x10 | 0x20, flags2=0x20 | 3, songname=b'mmd1 test'))

    # MMD2 and MMD3: the sequence table, track volumes and pans, OctaMED V7 extended sample info
    d0 = (8, 32, {0: {0: ('C-3', 1, 0x0c, 0x40), 1: ('E-3', 2, 0, 0), 5: ('G-3', 3, 0x04, 0x34), 7: ('C-4', 2, 0, 0)},
                  4: {2: ('C-2', 1, 0x0d, 0x02), 6: ('E-2', 3, 0x1f, 0x12)},
                  8: {0: (None, 0, 0x03, 0x08), 4: ('G-2', 2, 0x0f, 0xf2)},
                  16: {0: (None, 0, 0x0f, 0x00)}})
    d1 = (8, 16, {0: {0: ('E-3', 2, 0x0f, 0x60), 3: ('C-3', 1, 0x00, 0x47)}, 8: {1: ('D-3', 3, 0x0b, 0x00)}})
    ext = [b'\x00\x00\x00\x00\x00\x01\x00\x00' + bytes(2) + be32(0) + be32(64),
           b'\x00\x00\x00\x02\x31\x08\x00\x00' + bytes(2) + be32(32) + be32(128), b'']
    for ident in (b'MMD2', b'MMD3'):
        write('%s_octamed.med' % ident.decode().lower(),
              mmd_file(ident, [(0, 32, 64, 0), (0, 0, 48, 0), (0, 0, 64, 0)], [d0, d1], [0, 1, 0],
                       [sq, s16, synth], tempo2=5, flags=0x40, exp_smp=ext, exp_size=18,
                       mmdinfo=b'info text', trackvols=[64, 60, 50, 40, 64, 64, 64, 64],
                       trackpans=[-16, 16, 0, 8, -8, 0, 15, -15], songname=ident + b' test'))


class Nibbles:
    def __init__(self):
        self.n = []

    def put(self, v, count):
        for k in reversed(range(count)):
            self.n.append((v >> (4 * k)) & 0xf)

    def bytes(self):
        n = self.n + [0] * (len(self.n) & 1)
        return bytes((n[i] << 4) | n[i + 1] for i in range(0, len(n), 2))


def oldmed_cells():
    p0 = {0: {0: ('C-2', 1, 0x0c, 0x40), 1: ('E-2', 2, 0, 0), 2: ('G-2', 3, 0x04, 0x34)},
          2: {0: (None, 0, 0x0d, 0x04), 3: ('C-3', 1, 0x01, 0x08)},
          4: {1: (None, 0, 0x03, 0x10), 2: ('C-1', 2, 0x0f, 0xf1)},
          8: {0: ('D-2', 3, 0x00, 0x37), 3: (None, 0, 0x0f, 0x60)},
          12: {2: ('A-2', 1, 0x02, 0x03)}, 16: {0: (None, 0, 0x0f, 0x00)}}
    p1 = {0: {0: ('E-2', 2, 0x0f, 0xf2), 1: ('C-2', 3, 0x0c, 0x20)}, 8: {2: ('G-1', 1, 0x0f, 0xff)},
          12: {3: ('C-2', 2, 0x0f, 0x00)}}
    return p0, p1


def oldmed_files():
    p0, p1 = oldmed_cells()
    # MED 1.12 MED2: the samples are separate files, which a module loaded from memory doesn't find
    names = ['square', 'saw', 'sine']
    hdr = b'MED\x02' + bytes(40) + b''.join(pad(names[i] if i < 3 else '', 40) for i in range(31))
    hdr += b'\0' + bytes([64, 48, 40] + [0] * 28) + be16(0) + b''.join(be16(v) for v in [0, 0, 32] + [0] * 28)
    hdr += be16(0) + b''.join(be16(v) for v in [32, 0, 64] + [0] * 28)
    hdr += be16(2) + pad(bytes([0]), 100) + be16(1) + be16(130) + be16(0) + be16(6) + be32(0) + bytes(16)
    pats = b''
    for cells in (p0, p1):
        pats += be32(0)
        for r in range(64):
            for c in range(4):
                note, ins, fxt, fxp = cells.get(r, {}).get(c) or (None, 0, 0, 0)
                pats += be16(mod_period(note) if note else 0) + bytes([(ins << 4) | fxt, fxp])
    write('med2_med112.med', hdr + pats)

    # MED 2.00 MED3: masked header fields, packed blocks, the samples inside (FLAG_INSTRSATT)
    smp = [SQ8, SW8, SN8]
    hdr = b'MED\x03' + b''.join((names[i].encode() if i < 3 else b'') + b'\0' for i in range(32))
    hdr += be32(0xe0000000) + bytes([64, 48, 40])
    hdr += be32(0x20000000) + be16(32)
    hdr += be32(0xe0000000) + be16(32) + be16(0) + be16(64)
    hdr += be16(2) + be16(3) + bytes([0, 1, 0]) + be16(5) + bytes([0, 0x08]) + be16(6) + be32(0) + bytes(16)
    hdr += be32(0) + be32(0)

    def med3_block(cells):
        n = Nibbles()
        for r in range(64):
            row = [cells.get(r, {}).get(c) for c in range(4)]
            lm = sum(8 >> c for c in range(4) if row[c] and (row[c][0] or row[c][1]))
            n.put(lm, 1)
            for c in range(4):
                if lm & (8 >> c):
                    note, ins = row[c][0], row[c][1]
                    n.put(note_index(note) - 23 if note else 0, 2)
                    n.put(max(ins - 1, 0), 1)
            fm = sum(8 >> c for c in range(4) if row[c] and (row[c][2] or row[c][3]))
            n.put(fm, 1)
            for c in range(4):
                if fm & (8 >> c):
                    n.put(row[c][2], 1)
                    n.put(row[c][3], 2)
        data = n.bytes()
        return bytes([4, 0x0f]) + be16(len(data)) + data

    pats = med3_block(p0) + med3_block(p1)
    samples = be32(0xe0000000) + b''.join(be32(len(s)) + be16(0) + s for s in smp)
    write('med3_med200.med', hdr + pats + samples)

    # MED 2.10 MED4: a synthetic instrument, two-pass sample mask, MEDV and ANNO chunks
    insts = [('square', 0, 64, 64, 0), ('saw', 0, 0, 48, 0), ('synth', 0, 0, 64, -12)]
    hdr = b'MED\x04' + bytes([0x80, 0xe0])
    for name, ls, ll, vol, tr in insts:
        hdr += bytes([0x0c, len(name)]) + name.encode() + be16(ls >> 1) + be16(ll >> 1) + bytes([vol, tr & 0xff])
    hdr += be16(2) + be16(3) + bytes([0, 1, 0]) + be16(140) + bytes([0, 0x08 | 0x20]) + be16(5) + bytes(20)
    hdr += bytes([64] * 16) + bytes([64])

    def med4_block(cells, rows=64):
        n = Nibbles()
        for r in range(rows):
            row = [cells.get(r, {}).get(c) for c in range(4)]
            lm = sum(8 >> c for c in range(4) if row[c] and (row[c][0] or row[c][1]))
            n.put(lm, 1)
            for c in range(4):
                if lm & (8 >> c):
                    n.put(((note_index(row[c][0]) - 23 if row[c][0] else 0) << 4) | row[c][1], 3)
            fm = sum(8 >> c for c in range(4) if row[c] and (row[c][2] or row[c][3]))
            n.put(fm, 1)
            for c in range(4):
                if fm & (8 >> c):
                    n.put((row[c][2] << 8) | row[c][3], 3)
        data = n.bytes()
        ctl = b'\xaa'
        block = bytes([4, rows - 1]) + be16(len(data)) + ctl
        return bytes([len(block)]) + block + data

    pats = med4_block(p0) + med4_block(p1)
    wf = [s8(saw(32, 32, 100)), s8(square(64, 16, 90))]
    vt = [64, 56, 0xf1, 2, 40, 0xff]
    wt = [0xf0, 2, 0, 1, 0xfe, 2]
    msh = b'MSH\0' + be16(0xffff) + be16(0) + be16(0) + be16(0) + be16(0) + be16(len(vt)) + be16(len(wt))
    msh += bytes([2, 3]) + be16(len(wf)) + bytes(vt) + bytes(wt)
    base = len(msh) + 4 * len(wf)
    body = b''
    offs = []
    for w in wf:
        offs.append(base + len(body))
        body += be16(len(w) // 2) + w
    msh += b''.join(be32(o) for o in offs) + body
    samples = be32(0x70000000) + be32(0)
    samples += be32(len(SQ8)) + be16(0) + SQ8 + be32(len(SW8)) + be16(0) + SW8
    samples += be32(len(msh)) + be16(0xffff) + msh
    tail = b'MEDV' + be32(4) + be32(0x0210) + b'ANNO' + be32(14) + b'MED4 comment\0\0'
    write('med4_med210.med', hdr + pats + samples + tail)


def chip_files():
    smp = amiga_samples()
    hdr = pad('chiptracker', 20) + be16(0)
    for i in range(31):
        if i < len(smp):
            name, data, fin, vol, ls, ll = smp[i]
            hdr += pad(name, 22) + struct.pack('>HBBHH', len(data) // 2, fin & 0xf, vol, ls, ll // 2)
        else:
            hdr += pad('', 22) + struct.pack('>HBBHH', 0, 0, 0, 0, 1)
    tracks = [
        pt_track({0: 'C-2 01 F04', 4: '--- .. A02', 8: 'E-2 01 ...', 16: '--- .. D00'}),
        pt_track({0: 'G-2 02 ...', 6: '--- .. 302', 12: 'C-3 03 C20'}),
        pt_track({2: 'C-2 04 ...', 10: 'D-2 04 E91'}),
    ]

    def chip_track(t):
        out = b''
        for i in range(0, len(t), 4):
            ev = t[i:i + 4]
            p = ((ev[0] & 0x0f) << 8) | ev[1]
            n = 0 if p == 0 else 2 * (PT_PERIODS.index(p) + 12 - 13 + 1 + 12)
            out += bytes([n, (ev[0] & 0xf0) | (ev[2] >> 4), ev[2] & 0x0f, ev[3]])
        return out

    tidx = b''
    for o in ([0, 1, 2, 1], [2, 0, 1, 2]):
        tidx += b''.join(bytes([t, 0]) for t in o)
    hdr += b'KRIS' + bytes([2, 0]) + pad(tidx, 1024) + be16(0)
    write('chip_kris.mod', hdr + b''.join(chip_track(t) for t in tracks) + b''.join(s[1] for s in smp))


def rtm_files():
    def obj(ident, name, header_size, version=0x110):
        return ident + b'\x20' + pad(name, 32) + b'\x1a' + le16(version) + le16(header_size)

    chn = 4
    orders = [0, 1, 0]
    head = pad('Real Tracker 1.10', 20) + pad('composer', 32) + le16(1) + bytes([chn, 2]) + le16(len(orders))
    head += le16(2) + bytes([5, 125]) + bytes([0xe0 & 0xff, 32, 0, 16] + [0] * 28) + le32(2 * len(orders))
    out = obj(b'RTMM', 'rtm test', len(head)) + head + b''.join(le16(o) for o in orders)

    def pat(rows, cells):
        data = b''
        for r in range(rows):
            last = -1
            for c, (note, ins, fx, f2) in sorted(cells.get(r, {}).items()):
                flags = 0
                ev = b''
                if c != last + 1:
                    flags |= 1
                    ev += bytes([c])
                if note is not None:
                    flags |= 2
                    ev += bytes([0xfe if note == '===' else note_index(note) - 12])
                if ins:
                    flags |= 4
                    ev += bytes([ins])
                if fx:
                    flags |= 0x18
                    ev += bytes(fx)
                if f2:
                    flags |= 0x60
                    ev += bytes(f2)
                data += bytes([flags]) + ev
                last = c
            data += b'\0'
        head = le16(1) + bytes([chn]) + le16(rows) + le32(len(data))
        return obj(b'RTND', 'pattern', len(head)) + head + data

    out += pat(32, {0: {0: ('C-4', 1, (15, 4), (12, 48)), 1: ('E-4', 2, None, None), 3: ('G-4', 1, (4, 0x34), None)},
                    2: {0: (None, 0, (10, 0x08), None), 2: ('C-5', 2, (8, 0x40), (20, 8))},
                    4: {1: (None, 0, (1, 0x10), (16, 48)), 3: ('===', 0, None, None)},
                    8: {0: ('D-4', 2, (3, 0x20), (17, 0x02)), 2: (None, 0, (27, 0x33), None)},
                    12: {1: (None, 0, (33, 0x13), None), 3: (None, 0, (36, 0x0f), None)},
                    16: {0: (None, 0, (13, 0), None)}})
    out += pat(16, {0: {0: ('E-4', 2, (37, 0x08), None), 1: ('C-4', 1, (40, 3), (25, 0x20))},
                    6: {2: ('G-3', 1, (29, 0x22), (28, 0xa0))}, 10: {3: (None, 0, (11, 2), None)}})

    def env(points, sus=0, ls=0, le=0, flags=0):
        b = bytes([len(points)])
        for i in range(12):
            x, y = points[i] if i < len(points) else (0, 0)
            b += le32(x) + le32(y)
        return b + bytes([sus, ls, le]) + le16(flags)

    def inst(name, samples, table, venv, penv, flags=1):
        h = bytes([len(samples)]) + le16(flags) + bytes(table) + venv + penv + bytes([1, 4, 8, 6]) + le16(0x100)
        h += bytes([0, 0, 0, 0])
        o = obj(b'RTIN', name, len(h)) + h
        for sname, data, ls, le, loop, flg, freq, note, pan, vol in samples:
            sh = le16(flg) + bytes([64, vol]) + le32(len(data)) + le32(loop) + le32(ls) + le32(le) + le32(freq)
            sh += bytes([note, pan & 0xff])
            o += obj(b'RTSM', sname, len(sh)) + sh + data
        return o

    out += inst('envelopes', [('square', delta8(square(64, 16, 100)), 0, 64, 1, 4, 8363, 48, -20, 64),
                              ('sine16', delta16le(sine(256, 64, 20000)), 0, 512, 2, 6, 16726, 60, 30, 48)],
                [0] * 60 + [1] * 60, env([(0, 128), (8, 64), (20, 100), (40, 0)], 1, 1, 2, 7),
                env([(0, 0), (10, -64), (20, 64)], 0, 0, 0, 1))
    out += inst('saw', [('saw', delta8(decay(saw(512, 32, 110))), 0, 0, 0, 4, 8363, 48, 0, 56)], [0] * 120,
                env([]), env([]), flags=2)
    write('rtm_rtm110.rtm', out)


def pt3_files():
    smp = amiga_samples()
    p0 = {0: {0: 'C-2 01 F04', 1: 'E-2 02 ...', 2: 'G-2 03 C30', 3: 'C-3 04 ...'},
          4: {0: '--- .. A02', 1: '--- .. 102', 3: '--- .. E11'},
          8: {0: 'D-2 01 ...', 2: '--- .. 404', 3: '--- .. E93'},
          16: {0: '--- .. D00'}}
    p1 = {0: {0: 'G-1 03 ...', 1: 'C-2 04 E62', 2: 'E-2 01 ...'}, 8: {1: '--- .. E63', 3: 'C-2 02 B00'}}
    mod = mod_file('', smp, [0, 1], [p0, p1])
    info = pad('pt3 test', 32) + b''.join(be16(v) for v in [31, 2, 1, 64, 125, 1, 1, 1, 94, 12, 0, 0, 0, 1, 0])
    body = b'MODL' + b'VERS' + be32(18) + bytes(4) + b'PT3.61'
    body += b'INFO' + be32(len(info) + 8) + info + b'CMNT' + be32(8 + 8) + b'comment!'
    body += b'PTDT' + be32(len(mod)) + mod
    write('pt3_iffmodl.pt36', b'FORM' + be32(len(body)) + body)


def dt_files():
    def ch(cid, data):
        return cid + be32(len(data)) + data

    name = b'dt test'
    out = b'D.T.' + be32(14 + len(name)) + be16(0) + bytes([0xff, 0]) + be16(0) + be16(5) + be16(125)
    out += be32(8400) + name
    out += ch(b'S.Q.', be16(3) + be16(0) + be32(0) + pad(bytes([0, 1, 0]), 128))
    out += ch(b'VERS', be32(19))
    sv19 = be16(24) + be32(0x80000000) + b''.join(be16(v) for v in [0, 160, 90, 45] + [90] * 28) + bytes(16)
    out += ch(b'SV19', sv19)
    out += ch(b'PATT', be16(4) + be16(2) + b'2.04')
    smp = [('square', s8(square(64, 16, 100)), 0, 64, 0, 64, 8, 0),
           ('stereo', s8(saw(128, 32, 90)) + s8(sine(128, 32, 90)), 0, 0, 2, 48, 8, 1),
           ('sine16', b''.join(struct.pack('>h', v) for v in sine(256, 64, 20000)), 0, 512, 0xe, 56, 16, 0)]
    inst = be16(len(smp))
    for sname, data, ls, ll, fin, vol, depth, stereo in smp:
        inst += be32(0) + be32(len(data)) + bytes([fin, vol]) + be32(ls) + be32(ll) + pad(sname, 22)
        inst += bytes([stereo, depth]) + be16(60) + be16(0) + be32(8363)
    out += ch(b'INST', inst)

    def dapt(pat, rows, cells):
        d = be32(0xffffffff) + be16(pat) + be16(rows)
        for r in range(rows):
            for c in range(4):
                cell = cells.get(r, {}).get(c)
                if cell is None:
                    d += bytes(4)
                    continue
                note, ins, vol, fxt, fxp = cell
                o, s = oct_note(note) if note else (0, 0)
                nb = (o << 4) | s if note else 0
                d += bytes([nb, (vol << 2) | (ins >> 4), ((ins & 0xf) << 4) | fxt, fxp])
        return ch(b'DAPT', d)

    out += dapt(0, 32, {0: {0: ('C-4', 1, 63, 0xf, 5), 1: ('E-4', 2, 40, 0, 0), 2: ('G-4', 3, 50, 0x4, 0x34)},
                        4: {0: (None, 0, 0, 0xa, 0x04), 3: ('C-5', 1, 0, 0x8, 0x40)},
                        8: {1: (None, 0, 0, 0x1, 0x10), 2: ('C-3', 2, 0, 0xe, 0x93)},
                        16: {0: (None, 0, 0, 0xd, 0x00)}})
    out += dapt(1, 16, {0: {0: ('D-4', 2, 0, 0x3, 0x10), 3: ('A-3', 3, 30, 0x0, 0x37)},
                        8: {2: (None, 0, 0, 0xb, 0x02)}})
    for s in smp:
        out += ch(b'DAIT', s[1])
    text = b'DT text'
    out += ch(b'TEXT', be16(0) + be32(len(text)) + be16(0) + be16(0) + be16(0xffff) + b'\0' + text)
    write('dt_dtm19.dtm', out)


def mgt_files():
    chn = 4
    f = Blob()
    f.add(bytes(58))
    song = pad('mgt test', 32)
    song_at = f.add(song + be32(0) + be16(3) + be16(0) + bytes([125, 5]) + be16(64) + bytes([64, 64]) +
                    b''.join(be16(0) for _ in range(chn)))
    seq = f.add(b''.join(be16(o) for o in [0, 1, 0]))
    f.put32(song_at + 32, seq)
    smp = [('square', SQ8, 0, 64, 1), ('saw', SW8, 0, 0, 0), ('sine', SN8, 64, 192, 2)]
    ins_at = f.add(bytes(80 * len(smp)))

    def track(cells, rows=32):
        out = be16(rows)
        r = 0
        while r < rows:
            c = cells.get(r)
            if c is None:
                skip = 0
                while r + skip + 1 < rows and skip < 3 and cells.get(r + skip) is None and \
                        cells.get(r + skip + 1) is None:
                    skip += 1
                out += bytes([skip])
                r += skip + 1
                continue
            note, ins, vol, fxt, fxp = c
            b = 0
            ev = b''
            if note:
                b |= 0x04
                ev += bytes([1 if note == '===' else note_index(note) - 1])
            if ins:
                b |= 0x08
                ev += bytes([ins])
            if vol:
                b |= 0x10
                ev += bytes([vol])
            if fxt or fxp:
                b |= 0x60
                ev += bytes([fxt, fxp])
            out += bytes([b]) + ev
            r += 1
        return out

    tracks = [
        track({0: ('C-4', 1, 0x50, 0x0f, 5), 4: (None, 0, 0x62, 0, 0), 8: ('E-4', 2, 0, 0x0c, 0x30),
               16: (None, 0, 0, 0x0d, 0)}),
        track({0: ('G-4', 3, 0xb4, 0, 0), 6: ('===', 0, 0, 0, 0), 10: ('C-5', 1, 0xc4, 0x01, 0x08)}),
        track({2: ('E-3', 2, 0x30, 0x14, 0x02), 12: (None, 0, 0xf4, 0x03, 0x10)}),
        track({0: ('A-3', 1, 0x84, 0, 0), 8: (None, 0, 0x93, 0x1d, 0x02)}, rows=16),
    ]
    trk_at = f.add(bytes(4 * (len(tracks) + 1)))
    for i, t in enumerate(tracks):
        f.put32(trk_at + 4 * (i + 1), f.add(t, align=1))
    pat_at = f.add(be16(32) + b''.join(be16(t + 1) for t in [1, 2, 3, 0]) + be16(16) +
                   b''.join(be16(t + 1) for t in [4, 3, 2, 1]))
    for i, (name, data, ls, ll, flags) in enumerate(smp):
        at = f.add(data)
        f.data[ins_at + 80 * i:ins_at + 80 * (i + 1)] = (pad(name, 32) + be32(at) + be32(len(data)) + be32(ls) +
                                                         be32(ll) + be32(0) + be32(0) + be32(8363) +
                                                         be16(0x400 - 0x80 * i) + bytes([64, 64, flags, 0]) +
                                                         bytes(14))
    hdr = b'MGT\x11\xbdMCS' + b''.join(be16(v) for v in [chn, 1, 3, 2, len(tracks) + 1, len(smp), 0]) + be32(0)
    hdr += be32(song_at) + be32(seq) + be32(ins_at) + be32(pat_at) + be32(trk_at) + be32(0) + be32(0) + be32(0)
    f.data[0:58] = hdr
    write('mgt_megatracker.mgt', bytes(f.data))


def arch_files():
    def ch(cid, data):
        return iff(cid, data, le=True)

    out = b''
    out += ch(b'TINF', bytes([0x94, 0x19, 0x10, 0x04]))
    out += ch(b'MVOX', le32(4)) + ch(b'STER', bytes([1, 7, 3, 5, 0, 0, 0, 0]))
    out += ch(b'MNAM', pad('arch test', 32)) + ch(b'ANAM', pad('author', 32)) + ch(b'MLEN', le32(3))
    out += ch(b'PNUM', le32(2)) + ch(b'PLEN', pad(bytes([32, 16]), 64)) + ch(b'SEQU', pad(bytes([0, 1, 0]), 128))

    def patt(rows, cells):
        d = b''
        for r in range(rows):
            for c in range(4):
                note, ins, fxt, fxp = cells.get(r, {}).get(c) or (None, 0, 0, 0)
                d += bytes([fxp, fxt, ins, note_index(note) - 35 if note else 0])
        return ch(b'PATT', d)

    out += patt(32, {0: {0: ('C-4', 1, 0x1c, 5), 1: ('E-4', 2, 0x0c, 40), 2: ('G-4', 3, 0x0e, 3)},
                     4: {0: (None, 0, 0x10, 4), 3: ('C-5', 1, 0x01, 8)},
                     8: {1: (None, 0, 0x11, 2), 2: ('C-3', 2, 0x03, 0x10)},
                     12: {3: (None, 0, 0x15, 0x14)},
                     16: {0: (None, 0, 0x0b, 0)}})
    out += patt(16, {0: {0: ('D-4', 2, 0x00, 0x37), 3: ('A-3', 3, 0x1f, 0x30)}, 8: {2: (None, 0, 0x13, 0)}})

    def samp(name, data, vol, ls, ll):
        d = b'SNAM' + le32(len(name)) + name.encode() + b'SVOL' + le32(4) + le32(vol) + b'SLEN' + le32(4)
        d += le32(len(data)) + b'ROFS' + le32(4) + le32(ls) + b'RLEN' + le32(4) + le32(ll)
        d += b'SDAT' + le32(len(data)) + le32(0) + data
        return ch(b'SAMP', d)

    vidc = bytes((i * 37) & 0xff for i in range(256))
    out += samp('vidc saw', bytes(((i % 32) * 8) & 0xfe for i in range(512)), 0xff, 0, 0)
    out += samp('vidc loop', vidc, 0xc0, 0, 256) + samp('short', vidc[:64], 0x80, 16, 2)
    write('arch_musx.mod', b'MUSX' + le32(len(out)) + out)


def sym_lzw(data):
    """Digital Symphony's LZW: 9 to 13 bit codes (256 clears, 257 ends the stream), least significant bit
    first, the stream padded to 4 bytes."""
    out = BitsLsb()
    dic = {bytes([i]): i for i in range(256)}
    nxt = 258
    state = {'length': 258, 'bits': 9, 'maxlength': 512, 'first': True, 'inc': False}

    def emit(code):
        out.put(code, state['bits'])
        state['inc'] = False
        if state['first']:
            state['first'] = False
            return
        if state['length'] < 8192:
            state['length'] += 1
            if state['length'] >= state['maxlength'] and state['length'] < 8192:
                state['maxlength'] <<= 1
                state['bits'] += 1
                state['inc'] = True

    w = b''
    for c in data:
        wc = w + bytes([c])
        if wc in dic:
            w = wc
            continue
        emit(dic[w])
        if nxt < 8192:
            dic[wc] = nxt
            nxt += 1
        w = bytes([c])
    if w:
        emit(dic[w])
    out.put(257, state['bits'] - 1 if state['inc'] else state['bits'])
    b = bytes(out.out) + (bytes([out.acc & 0xff]) if out.n else b'')
    while len(b) % 4:
        b += b'\0'
    return b


def sym_sigma_delta(data, max_runlength=8):
    """Digital Symphony's sigma-delta sample packing."""
    out = BitsLsb()
    bits = 8
    run = 0
    out.put(data[0], 8)
    acc = data[0]
    for v in data[1:]:
        d = (v - acc) & 0xff
        if d >= 128:
            value = ((256 - d) << 1) | 1
        elif d == 0:
            value = 1
        else:
            value = d << 1
        while value >= (1 << bits):
            out.put(0, bits)
            bits += 1
            run = 0
        out.put(value, bits)
        acc = v
        if value >> (bits - 1):
            run = 0
            continue
        run += 1
        if run >= max_runlength:
            if bits > 1:
                bits -= 1
            run = 0
    b = bytes(out.out) + (bytes([out.acc & 0xff]) if out.n else b'')
    while len(b) % 4:
        b += b'\0'
    return bytes([max_runlength]) + b


def sym_files():
    chn = 4
    tracks = []

    def trk(cells):
        d = b''
        for r in range(64):
            note, ins, fxt, parm = cells.get(r) or (None, 0, 0, 0)
            n = note_index(note) - 35 if note else 0
            d += le32(n | (ins << 6) | (fxt << 14) | (parm << 20))
        return d

    tracks.append(trk({0: ('C-4', 1, 0x0f, 4), 4: (None, 0, 0x0c, 40), 8: ('E-4', 2, 0x04, 0x34),
                       12: (None, 0, 0x2f, 1000), 20: (None, 0, 0x0d, 0)}))
    tracks.append(trk({0: ('G-4', 3, 0x30, 0x003), 6: (None, 0, 0x01, 0x208), 10: ('C-5', 4, 0x19, 3),
                       14: (None, 0, 0x1c, 2)}))
    tracks.append(trk({2: ('E-3', 5, 0x11, 0x302), 8: ('C-3', 2, 0x20, 0x137), 16: (None, 0, 0x32, 0)}))
    seq = [[0, 1, 2, 0x1000], [2, 0, 0x1000, 1], [1, 2, 0, 0x1000]]
    sqb = b''.join(b''.join(le16(t) for t in o) for o in seq)
    saw = bytes((i * 4) & 0xff for i in range(512))
    sine16 = s16le(sine(256, 64, 20000))
    smooth = u8(sine(512, 128, 100))
    vidc = bytes((i * 37) & 0xff for i in range(256))
    samples = [('vidc', vidc, 0, 0, 0), ('lzw delta', bytes(delta8(list(struct.unpack('512b', saw)))), 1, 0, 64),
               ('raw8', s8(square(64, 16, 90)), 2, 0, 32), ('raw16', sine16, 3, 64, 128),
               ('sigma', smooth, 4, 0, 0)]
    hdr = b'\x02\x01\x13\x13\x14\x12\x01\x0b' + bytes([1, chn]) + le16(len(seq)) + le16(len(tracks))
    comment = b'Digital Symphony info text ' * 4
    hdr += le32(len(comment))[:3]
    for i in range(63):
        if i < len(samples):
            name, data, typ, ls, ll = samples[i]
            n = len(data) // (2 if typ == 3 else 1)
            hdr += bytes([len(name)]) + le32(n >> 1)[:3]
        else:
            hdr += b'\x80'
    hdr += bytes([8]) + b'sym test' + b'\xff' * 8
    hdr += b'\x00' + sqb
    hdr += b'\x01' + sym_lzw(b''.join(tracks))
    for i in range(63):
        if i >= len(samples):
            continue
        name, data, typ, ls, ll = samples[i]
        hdr += name.encode() + le32(ls >> 1)[:3] + le32(ll >> 1)[:3] + bytes([64 - 4 * i, i])
        if typ == 0:
            hdr += b'\0' + data
        elif typ == 1:
            hdr += b'\1' + sym_lzw(data)
        elif typ == 2:
            hdr += b'\2' + data
        elif typ == 3:
            hdr += b'\3' + data
        else:
            hdr += b'\4' + sym_sigma_delta(data)
    hdr += b'\1' + sym_lzw(comment)
    write('sym_symphony.dsym', hdr)


def digi_files():
    smp = amiga_samples()
    chn = 4
    hdr = pad(b'DIGI Booster module', 20) + b'V1.6' + bytes([0x16, chn, 1]) + bytes(19) + bytes([1, 2])
    hdr += pad(bytes([0, 1, 0]), 128)
    for field in range(3):
        for i in range(31):
            if i < len(smp):
                name, data, fin, vol, ls, ll = smp[i]
                hdr += be32([len(data), ls, ll][field])
            else:
                hdr += be32(0)
    hdr += bytes([s[3] for s in smp] + [0] * (31 - len(smp)))
    hdr += bytes([s[2] & 0xff for s in smp] + [0] * (31 - len(smp)))
    hdr += pad('digi test', 32) + b''.join(pad(smp[i][0] if i < len(smp) else '', 30) for i in range(31))

    def pat(cells):
        table = b''
        events = b''
        for r in range(64):
            mask = 0
            for c in range(chn):
                cell = cells.get(r, {}).get(c)
                if cell:
                    mask |= 0x80 >> c
                    events += mod_pattern(1, 1, {0: {0: cell}})
            table += bytes([mask])
        return be16(len(events) + 64) + table + events

    p0 = {0: {0: 'C-2 01 F04', 1: 'E-2 02 ...', 2: 'G-2 03 C30', 3: 'C-3 04 ...'},
          4: {0: '--- .. A02', 1: '--- .. 102', 3: '--- .. E41'},
          8: {0: 'D-2 01 ...', 2: '--- .. 404', 3: '--- .. 820'},
          16: {0: '--- .. D00'}}
    p1 = {0: {0: 'G-1 03 ...', 1: 'C-2 04 E03', 2: 'E-2 01 ...'}, 8: {1: '--- .. E93', 3: 'C-2 02 B00'}}
    write('digi_booster.digi', hdr + pat(p0) + pat(p1) + b''.join(s[1] for s in smp))


def dbm_files():
    def ch(cid, data):
        return cid + be32(len(data)) + data

    chn = 4
    smp = [(SQ8, 1), (b''.join(struct.pack('>h', v) for v in sine(256, 64, 20000)), 2), (SW8, 1)]
    out = b'DBM0' + be16(0x0300) + be16(0) + b'NAME' + be32(44) + pad('dbm test', 44)
    out += ch(b'INFO', be16(3) + be16(len(smp)) + be16(1) + be16(2) + be16(chn))
    out += ch(b'SONG', pad('song', 44) + be16(3) + be16(0) + be16(1) + be16(0))
    inst = b''
    for i, (name, snum, vol, c2, ls, ll, pan, flags) in enumerate([
            ('square', 1, 64, 8363, 0, 64, -32, 1), ('sine16', 2, 48, 16726, 0, 256, 32, 2),
            ('saw', 3, 56, 8363, 0, 0, 0, 0)]):
        inst += pad(name, 30) + be16(snum) + be16(vol) + be32(c2) + be32(ls) + be32(ll) + be16(pan) + be16(flags)
    out += ch(b'INST', inst)

    def pat(rows, cells):
        d = b''
        for r in range(rows):
            for c, (note, ins, fx, f2) in sorted(cells.get(r, {}).items()):
                n = 0
                ev = b''
                if note is not None:
                    n |= 1
                    if note == '===':
                        ev += b'\x1f'
                    else:
                        o, s = oct_note(note)
                        ev += bytes([(o << 4) | s])
                if ins:
                    n |= 2
                    ev += bytes([ins])
                if fx:
                    n |= 0x0c
                    ev += bytes(fx)
                if f2:
                    n |= 0x30
                    ev += bytes(f2)
                d += bytes([c + 1, n]) + ev
            d += b'\0'
        return be16(rows) + be32(len(d)) + d

    out += ch(b'PATT', pat(32, {0: {0: ('C-4', 1, (0x0f, 5), (0x0c, 48)), 1: ('E-4', 2, None, None),
                                    2: ('G-4', 3, (0x04, 0x34), None)},
                                4: {0: (None, 0, (0x0a, 0x04), None), 3: ('C-5', 1, (0x1c, 140), None)},
                                8: {1: (None, 0, (0x01, 0x10), (0x0e, 0x30)), 2: ('===', 0, None, None)},
                                12: {0: ('D-4', 2, (0x0e, 0x40), None), 3: (None, 0, (0x0e, 0x51), None)},
                                16: {0: (None, 0, (0x0d, 0), None)}}) +
              pat(16, {0: {0: ('E-4', 3, (0x03, 0x08), None), 1: ('C-4', 1, (0x08, 0x20), None)},
                       8: {2: ('G-3', 2, (0x0b, 0x02), None)}}))
    out += ch(b'SMPL', b''.join(be32(fl) + be32(len(d) // fl) + d for d, fl in smp))

    def env(ins, flags, points, sus=0, lps=0, lpe=0):
        e = be16(ins) + bytes([flags, len(points) - 1, sus, lps, lpe, 0])
        for i in range(32):
            p, v = points[i] if i < len(points) else (0, 0)
            e += be16(p) + be16(v)
        return e

    out += ch(b'VENV', be16(1) + env(1, 0x07, [(0, 64), (8, 40), (20, 50), (40, 0)], 1, 1, 2))
    out += ch(b'PENV', be16(1) + env(2, 0x01, [(0, 0), (10, -128), (20, 128)]))
    write('dbm_digibooster.dbm', out)


def emod_files():
    smp = [('square', SQ8, 0, 64, 1, 0), ('saw', SW8, 0, 0, 0, 2), ('sine', SN8, 64, 128, 1, 0)]
    emic = be16(1) + pad('emod test', 20) + pad('composer', 20) + bytes([125, len(smp)])
    for i, (name, data, ls, ll, loop, fin) in enumerate(smp):
        emic += bytes([i + 1, 64 - 8 * i]) + be16(len(data) // 2) + pad(name, 20) + bytes([loop, fin])
        emic += be16(ls // 2) + be16(ll // 2) + be32(0)
    emic += bytes([0, 2]) + bytes([0, 31]) + pad('p0', 20) + be32(0) + bytes([1, 15]) + pad('p1', 20) + be32(0)
    emic += bytes([3, 0, 1, 0])

    def pat(rows, cells):
        d = b''
        for r in range(rows):
            for c in range(4):
                note, ins, fxt, fxp = cells.get(r, {}).get(c) or (None, 0, 0, 0)
                d += bytes([ins, (note_index(note) - 49 + 12) if note else 0xff, fxt, fxp])
        return d

    patt = pat(32, {0: {0: ('C-4', 1, 0x0f, 4), 1: ('E-4', 2, 0x0c, 40), 2: ('G-4', 3, 0x04, 0x34)},
                    4: {0: (None, 0, 0x0a, 0x04), 3: ('C-5', 1, 0x01, 8)},
                    8: {1: (None, 0, 0x09, 0x10), 2: ('C-4', 2, 0x03, 0x10)},
                    16: {0: (None, 0, 0x0b, 2)}})
    patt += pat(16, {0: {0: ('D-4', 2, 0x00, 0x37), 3: ('A-3', 3, 0x0e, 0x91)}, 8: {2: (None, 0, 0x0d, 0)}})
    body = b'EMOD' + iff(b'EMIC', emic) + iff(b'PATT', patt) + iff(b'8SMP', b''.join(s[1] for s in smp))
    write('emod_quadra.emod', b'FORM' + be32(len(body)) + body)

    # sample offsets (in 512 bytes) into a long sample, a pattern break to
    # row 20 and a position jump to order 2 (both binary)
    long = s8(decay(saw(4096, 50, 100), 1.0, 0.3))
    emic = be16(1) + pad('emod jumps', 20) + pad('composer', 20) + bytes([125, 1])
    emic += bytes([1, 64]) + be16(len(long) // 2) + pad('long', 20) + bytes([0, 0]) + be16(0) + be16(0) + be32(0)
    emic += bytes([0, 3])
    emic += b''.join(bytes([i, 31 if i == 1 else 15]) + pad('p%d' % i, 20) + be32(0) for i in range(3))
    emic += bytes([3, 0, 1, 2])
    patt = pat(16, {0: {0: ('C-4', 1, 0x09, 0x01)}, 2: {0: ('C-4', 1, 0x09, 0x03)}, 4: {0: ('C-4', 1, 0x09, 0x05)},
                    6: {0: (None, 0, 0x0d, 20)}})
    patt += pat(32, {0: {1: ('C-5', 1, 0, 0)}, 14: {1: ('A-4', 1, 0, 0)}, 20: {1: ('G-4', 1, 0x09, 0x02)},
                     22: {1: (None, 0, 0x0b, 2)}})
    patt += pat(16, {0: {2: ('E-4', 1, 0, 0)}, 4: {2: ('C-4', 1, 0x09, 0x04)}})
    body = b'EMOD' + iff(b'EMIC', emic) + iff(b'PATT', patt) + iff(b'8SMP', long)
    write('emod_jumps.emod', b'FORM' + be32(len(body)) + body)


def okt_files():
    smp = [('square', SQ8, 0, 64, 64, 0), ('saw', SW8, 0, 0, 48, 1), ('sine', SN8, 64, 128, 40, 2)]
    samp = b''
    for i in range(36):
        if i < len(smp):
            name, data, ls, ll, vol, mode = smp[i]
            samp += pad(name, 20) + be32(len(data)) + be16(ls // 2) + be16(ll // 2) + be16(vol) + be16(mode)
        else:
            samp += bytes(32)

    def pbod(rows, cells):
        d = be16(rows)
        for r in range(rows):
            for c in range(6):
                note, ins, fxt, fxp = cells.get(r, {}).get(c) or (None, 0, 0, 0)
                d += bytes([note_index(note) - 23 if note else 0, max(ins - 1, 0), fxt, fxp])
        return iff(b'PBOD', d)

    body = b'OKTASONG' + iff(b'CMOD', be16(0) + be16(1) + be16(0) + be16(1)) + iff(b'SAMP', samp)
    body += iff(b'SPEE', be16(5)) + iff(b'SLEN', be16(2)) + iff(b'PLEN', be16(3)) + iff(b'PATT', bytes([0, 1, 0]))
    body += pbod(32, {0: {0: ('C-3', 1, 28, 4), 1: ('E-3', 2, 31, 0x40), 2: ('G-3', 3, 10, 0x34)},
                      4: {1: (None, 0, 1, 0x08), 3: ('C-2', 1, 31, 0x48), 5: ('E-2', 2, 11, 0x23)},
                      8: {0: ('D-3', 2, 13, 0x02), 4: (None, 0, 31, 0x55)},
                      12: {2: (None, 0, 21, 3), 5: (None, 0, 31, 0x65)}, 16: {0: (None, 0, 25, 1)}})
    body += pbod(16, {0: {0: ('E-3', 3, 2, 0x04), 1: ('C-3', 1, 17, 2)}, 8: {2: (None, 0, 30, 1)}})
    body += b''.join(iff(b'SBOD', s[1]) for s in smp)
    write('okt_oktalyzer.okt', body)


def sfx_files():
    smp = amiga_samples()[:3]
    hdr = b''.join(be32(len(s[1])) for s in smp) + bytes(4 * (15 - len(smp)))
    hdr += b'SONG' + be16(7000) + bytes(14)
    for i in range(15):
        if i < len(smp):
            name, data, fin, vol, ls, ll = smp[i]
            hdr += pad(name, 22) + be16(len(data) // 2) + bytes([fin & 0xf, vol]) + be16(ls) + be16(ll // 2)
        else:
            hdr += pad('', 22) + be16(0) + bytes([0, 0]) + be16(0) + be16(1)
    hdr += bytes([1, 0]) + pad(bytes([0, 0, 1]), 128)

    def pat(cells):
        out = b''
        for r in range(64):
            for c in range(4):
                note, ins, fxt, fxp = cells.get(r, {}).get(c) or (None, 0, 0, 0)
                p = mod_period(note) if note else 0
                out += bytes([(ins & 0xf0) | (p >> 8), p & 0xff, ((ins & 0xf) << 4) | fxt, fxp])
        return out

    p0 = {0: {0: ('C-2', 1, 1, 0x37), 1: ('E-2', 2, 0, 0), 2: ('G-2', 3, 5, 0x10)},
          4: {0: (None, 0, 2, 0x20), 3: ('C-3', 1, 7, 0x02)},
          8: {1: (None, 0, 6, 0x08), 2: ('D-2', 2, 8, 0x01)},
          12: {0: (None, 0, 2, 0x03), 3: (None, 0, 3, 0)}}
    p1 = {0: {0: ('E-2', 3, 1, 0x47), 1: ('C-2', 1, 0, 0)}, 8: {2: ('G-1', 2, 5, 0x20)}}
    write('sfx_soundfx13.sfx', hdr + pat(p0) + pat(p1) + b''.join(s[1] for s in smp))


def far_files():
    name = pad('far test', 40)
    text = pad('FAR comment text', 132) + pad('second line\0\0', 140)
    smp = [('square', SQ8, 0, 64, 0, 8), ('sine16', s16le(sine(256, 64, 20000)), 0, 512, 1, 8),
           ('saw', SW8, 0, 0, 0, 0)]
    orders = [0, 1, 0]

    def pat(rows, brk, cells):
        d = bytes([brk, 0])
        for r in range(rows):
            for c in range(16):
                note, ins, vol, fx = cells.get(r, {}).get(c) or (None, 0, 0, 0)
                d += bytes([note_index(note) - 35 if note else 0, max(ins - 1, 0), vol, fx])
        return d

    pats = [pat(32, 10, {0: {0: ('C-4', 1, 0x10, 0xf4), 1: ('E-4', 2, 0x0c, 0x00), 2: ('G-4', 3, 0x08, 0x63)},
                         2: {0: (None, 0, 0, 0x52), 3: ('C-5', 1, 0x0f, 0xb3)},
                         4: {1: (None, 0, 0, 0x14), 2: (None, 0, 0x06, 0x81)},
                         8: {0: ('D-4', 2, 0, 0x34), 3: (None, 0, 0x0a, 0xa4)},
                         12: {1: (None, 0, 0, 0x43), 2: (None, 0, 0, 0xd2)},
                         16: {0: (None, 0, 0, 0x05), 3: (None, 0, 0, 0x93)}}),
            pat(16, 6, {0: {0: ('E-4', 3, 0x0f, 0x24), 1: ('C-4', 1, 0x0d, 0x72)},
                         8: {2: ('G-3', 2, 0x05, 0xe2), 3: (None, 0, 0, 0x03)}})]
    hdr = b'FAR\xfe' + name + b'\r\n\x1a' + le16(869 + len(text)) + bytes([0x10]) + bytes([1] * 15 + [0])
    hdr += bytes(9) + bytes([4]) + bytes([0, 15, 7, 8, 3, 12] + [7] * 10) + bytes(4) + le16(len(text))
    hdr += text + pad(bytes(orders), 256) + bytes([len(pats), len(orders), 0])
    hdr += b''.join(le16(len(p)) for p in pats) + bytes(2 * (256 - len(pats)))
    smap = bytes([0x07] + [0] * 7)
    ins = b''
    for name, data, ls, le, typ, loop in smp:
        ins += pad(name, 32) + le32(len(data)) + bytes([0, 0]) + le32(ls) + le32(le) + bytes([typ, loop]) + data
    write('far_farandole.far', hdr + b''.join(pats) + smap + ins)


def umx_files():
    # an Unreal package around one of the S3M modules
    with open(os.path.join(OUT, 's3m_st3.s3m'), 'rb') as f:
        music = f.read()
    names = [b'None', b'Music', b's3m', b'Core', b'Package', b'Song']
    name_tab = b''.join(n + b'\0' + le32(0) for n in names)
    hdr_len = 36 + 32
    name_ofs = hdr_len
    import_ofs = name_ofs + len(name_tab)
    imports = b'\x03\x04\x00\x05'
    export_ofs = import_ofs + len(imports)
    serial_ofs = export_ofs + 64

    def fci(v):
        if v < 0x40:
            return bytes([v])
        out = bytes([0x40 | (v & 0x3f)])
        v >>= 6
        while v >= 0x80:
            out += bytes([0x80 | (v & 0x7f)])
            v >>= 7
        return out + bytes([v])

    obj = fci(0) + fci(2) + fci(len(music)) + music
    export = fci(1) + fci(0) + le32(0) + fci(5) + le32(0x70004) + fci(len(obj)) + fci(serial_ofs)
    export = pad(export, 64)
    hdr = le32(0x9e2a83c1) + le32(61) + le32(1) + le32(len(names)) + le32(name_ofs) + le32(1) + le32(export_ofs)
    hdr += le32(1) + le32(import_ofs) + bytes(hdr_len - 36)
    write('umx_unreal.umx', hdr + name_tab + imports + export + obj)


def stim_files():
    smp = amiga_samples()[:3]
    pats = []

    def pat(cells):
        tr = []
        for c in range(4):
            out = b''
            r = 0
            while r < 64:
                cell = cells.get(r, {}).get(c)
                if cell is None:
                    k = 0
                    while r + k + 1 < 64 and cells.get(r + k + 1, {}).get(c) is None and k < 127:
                        k += 1
                    out += bytes([0x80 | k])
                    r += k + 1
                    continue
                note, ins, fxt, fxp = cell
                n = note_index(note) - 35 if note else 0
                out += bytes([((fxt & 3) << 5) | ins, ((fxt & 0xc) << 4) | n, fxp])
                r += 1
            tr.append(out)
        offs = [8 + 0]
        for t in tr[:-1]:
            offs.append(offs[-1] + len(t))
        return b''.join(be16(o) for o in offs) + b''.join(tr)

    pats.append(pat({0: {0: ('C-4', 1, 0x0f, 4), 1: ('E-4', 2, 0x0c, 40), 2: ('G-4', 3, 0x04, 0x34)},
                     4: {0: (None, 0, 0x0a, 0x04), 3: ('C-5', 1, 0x01, 8)},
                     8: {1: (None, 0, 0x02, 0x10), 2: ('C-4', 2, 0x03, 0x10)},
                     16: {0: (None, 0, 0x0d, 0)}}))
    pats.append(pat({0: {0: ('D-4', 2, 0x00, 0x37), 3: ('A-3', 3, 0x0e, 0x91)}, 8: {2: (None, 0, 0x0b, 0)}}))
    orders = [0, 1, 0]
    pat_base = 22 + 128 + 4 * 64
    pataddr = []
    blob = b''
    for p in pats:
        pataddr.append(pat_base + len(blob) - 0x0c)
        blob += p
    smpaddr = pat_base + len(blob)
    hdr = b'STIM' + be32(smpaddr) + bytes(8) + be16(len(smp)) + be16(len(orders)) + be16(len(pats))
    hdr += pad(bytes(orders), 128) + b''.join(be32(a) for a in pataddr) + bytes(4 * (64 - len(pataddr)))
    sinfo = bytes(4 * len(smp))
    for name, data, fin, vol, ls, ll in smp:
        sinfo += be16(len(data) // 2) + bytes([fin & 0xf, vol]) + be16(ls // 2) + be16(ll // 2) + data
    write('stim_slamtilt.stim', hdr + blob + sinfo)


def coco_files():
    smp = [('square\r', bytes(((i // 8) % 2) * 0xfe for i in range(64)), 0, 64, 0),
           ('vidc\r', bytes((i * 37) & 0xff for i in range(256)), 0, 0, 0x40)]
    chn = 4
    orders = [0, 1, 0]
    n_ins = len(smp)
    seq_ofs = 32 + 28 * n_ins
    pat_ofs = seq_ofs + len(orders) + 1
    pat_ofs = (pat_ofs + 3) & ~3
    patsz = 64 * chn * 4
    smp_ofs = pat_ofs + 2 * patsz
    hdr = bytes([0x80 | chn]) + pad('coco test\r', 20) + bytes([n_ins, len(orders), 2]) + le32(seq_ofs)
    hdr += le32(pat_ofs)
    off = smp_ofs
    for name, data, ls, lsz, vol in smp:
        hdr += le32(off) + le32(len(data)) + le32(vol) + le32(ls) + le32(lsz) + pad(name, 11) + b'\0'
        off += len(data)
    hdr = pad(hdr, seq_ofs) + bytes(orders) + b'\xff'
    hdr = pad(hdr, pat_ofs)

    def pat(cells):
        out = b''
        for r in range(64):
            for c in range(chn):
                note, ins, fxt, fxp = cells.get(r, {}).get(c) or (None, 0, 0, 0)
                out += bytes([fxp, fxt, ins, note_index(note) - 23 if note else 0])
        return out

    p0 = pat({0: {0: ('C-4', 1, 0x0f, 4), 1: ('E-4', 2, 0x0c, 0x40), 2: ('G-4', 1, 0x07, 3)},
              4: {0: (None, 0, 0x13, 0x04), 3: ('C-5', 2, 0x01, 8)},
              8: {1: (None, 0, 0x14, 0x02), 2: ('C-4', 2, 0x03, 0x10)},
              16: {0: (None, 0, 0x0d, 0)}})
    p1 = pat({0: {0: ('D-4', 2, 0x00, 0x37), 3: ('A-3', 1, 0x05, 0x04)}, 8: {2: (None, 0, 0x0e, 0)}})
    write('coco_coconizer.coco', hdr + p0 + p1 + b''.join(s[1] for s in smp))


def ims_files():
    smp = amiga_samples()
    hdr = pad('ims test', 20)
    for i in range(31):
        if i < len(smp):
            name, data, fin, vol, ls, ll = smp[i]
            hdr += pad(name, 22) + be16(len(data) // 2) + bytes([0, vol]) + be16(ls // 2) + be16(ll // 2)
        else:
            hdr += pad('', 22) + be16(0) + bytes([0, 0]) + be16(0) + be16(1)
    orders = [0, 1, 0]
    hdr += bytes([len(orders), 0]) + pad(bytes(orders), 128) + be32(2 * 0x300 + 0x43c)

    def pat(cells):
        out = b''
        for r in range(64):
            for c in range(4):
                cell = cells.get(r, {}).get(c)
                if cell is None:
                    out += bytes([0x3f, 0, 0])
                    continue
                note, ins, fxt, fxp = cell
                n = 0x3f if note is None else note_index(note) - 33
                out += bytes([n | ((ins & 0x10) << 2), ((ins & 0xf) << 4) | fxt, fxp])
        return out

    p0 = {0: {0: ('C-4', 1, 0xf, 4), 1: ('E-4', 2, 0xc, 40), 2: ('G-4', 3, 0x4, 0x34)},
          4: {0: (None, 0, 0xa, 0x04), 3: ('C-5', 4, 0x1, 8)},
          8: {1: (None, 0, 0x2, 0x10), 2: ('C-4', 2, 0x3, 0x10)},
          12: {0: (None, 0, 0xd, 0x00)}}
    p1 = {0: {0: ('D-4', 2, 0x0, 0x37), 3: ('A-3', 3, 0xe, 0x91)}, 8: {2: (None, 0, 0xb, 0)}}
    write('ims_images.ims', hdr + pat(p0) + pat(p1) + b''.join(s[1] for s in smp))


def c669_files():
    smp = [('square.smp', u8(square(64, 16, 100)), 0, 64), ('saw.smp', u8(decay(saw(512, 32, 110))), 0, 0xfffff),
           ('sine.smp', u8(sine(256, 64, 90)), 64, 256)]
    orders = [0, 1, 0]
    hdr = b'if' + pad('Composer 669 test', 108) + bytes([len(smp), 2, 0]) + pad(bytes(orders), 128, b'\xff')
    hdr += pad(bytes([2, 3]), 128) + pad(bytes([15, 15]), 128)
    for name, data, ls, le in smp:
        hdr += pad(name, 13) + le32(len(data)) + le32(ls) + le32(le)

    def pat(cells):
        out = b''
        for r in range(64):
            for c in range(8):
                cell = cells.get(r, {}).get(c)
                if cell is None:
                    out += b'\xff\x00\xff'
                    continue
                note, ins, vol, fx = cell
                if note is None:
                    b0, b1 = 0xfe, vol & 0xf
                else:
                    n = note_index(note) - 36
                    b0, b1 = (n << 2) | ((ins - 1) >> 4), (((ins - 1) & 0xf) << 4) | (vol & 0xf)
                out += bytes([b0, b1, 0xff if fx is None else fx])
        return out

    p0 = pat({0: {0: ('C-4', 1, 15, 0x52), 1: ('E-4', 2, 12, None), 2: ('G-4', 3, 10, 0x43)},
              4: {0: (None, 0, 13, 0x04), 3: ('C-5', 1, 15, 0x23), 5: ('E-3', 2, 8, None)},
              8: {1: (None, 0, 15, 0x14), 2: ('C-4', 2, 9, 0x31)},
              12: {6: ('G-3', 3, 15, None), 7: ('C-4', 1, 6, None)}})
    p1 = pat({0: {0: ('D-4', 2, 15, 0x02), 3: ('A-3', 3, 11, None)}, 8: {2: (None, 0, 7, 0x58)}})
    write('669_composer.669', hdr + p0 + p1 + b''.join(s[1] for s in smp))


def fnk_files():
    smp = [('square', SQ8, 0, 255, 0x80), ('saw', SW8, 0xffffffff, 200, 0x40), ('sine', SN8, 64, 220, 0xc0)]
    orders = [0, 1, 0]
    hdr = b'Funk' + bytes([0x21, 0x1e, 0x12, 0x07])
    size_at = len(hdr)
    hdr += le32(0) + b'Fk04' + bytes([0]) + pad(bytes(orders), 256, b'\xff') + pad(bytes([15, 7]), 128)
    for i in range(64):
        if i < len(smp):
            name, data, ls, vol, pan = smp[i]
            hdr += pad(name, 19) + le32(ls) + le32(len(data)) + bytes([vol, pan, 0, 0, 0])
        else:
            hdr += pad('', 19) + le32(0xffffffff) + le32(0) + bytes(5)

    def pat(cells):
        out = b''
        for r in range(64):
            for c in range(4):
                cell = cells.get(r, {}).get(c)
                if cell is None:
                    out += bytes([0xfc, 0x0f, 0])
                    continue
                note, ins, fx, fxp = cell
                n = 0x3f if note is None else note_index(note) - 36
                i = max(ins - 1, 0)
                out += bytes([(n << 2) | (i >> 4), ((i & 0xf) << 4) | fx, fxp])
        return out

    p0 = pat({0: {0: ('C-4', 1, 0xe, 0xf4), 1: ('E-4', 2, 0xd, 0x40), 2: ('G-4', 3, 0x3, 0x34)},
              4: {0: (None, 0, 0x6, 0x04), 3: ('C-5', 1, 0x0, 0x08)},
              8: {1: (None, 0, 0x1, 0x10), 2: ('C-4', 2, 0x2, 0x10)},
              12: {3: (None, 0, 0xe, 0xe8), 1: (None, 0, 0xe, 0x13)}})
    p1 = pat({0: {0: ('D-4', 2, 0xb, 0x37), 3: ('A-3', 3, 0x7, 0x04)}, 8: {2: (None, 0, 0xe, 0xd2)}})
    data = hdr + p0 + p1 + b''.join(s[1] for s in smp)
    data = data[:size_at] + le32(len(data)) + data[size_at + 4:]
    write('fnk_funktracker.fnk', data)


def abk_files():
    # AMOS Music Bank: tracks of commands and notes, instruments, songs
    smp = [('square', SQ8, 64), ('saw', SW8, 0), ('sine', SN8, 256)]

    def track(events):
        d = b''
        for ev in events:
            d += be16(ev)
        return d + be16(0x8000)

    def note(period, delay=0):
        return [0x4000 | delay, period]

    tracks = [
        track([0x8900, 0x8800 | 0x21] + note(428, 4) + [0x8a00 | 0x37] + note(381, 4) + [0x8d04, 0x9000 | 4] +
              [0x8300 | 40] + note(339, 8)),
        track([0x8901] + note(320, 2) + [0x8c00 | 0x34, 0x9000 | 6] + note(285, 6) + [0x8b00 | 0x10] +
              note(254, 8)),
        track([0x8902] + note(214, 8) + [0x8100 | 0x08] + note(240, 4) + [0x8500 | 1] + note(226, 4)),
        track([0x8900, 0x8600, 0x8700] + note(170, 16) + [0x9100 | 0]),
    ]
    # two patterns: track offsets per channel
    pat_tab = be16(2)
    hdr_len = 2 + 2 * 4 * 2
    offsets = []
    acc = hdr_len
    for t in tracks:
        offsets.append(acc)
        acc += len(t)
    pat_tab += b''.join(be16(offsets[i]) for i in [0, 1, 2, 3]) + b''.join(be16(offsets[i]) for i in [2, 3, 0, 1])
    pat_section = pat_tab + b''.join(tracks)
    # instruments
    ins = be16(len(smp))
    sdata = b''
    hdr_size = 2 + 32 * len(smp)
    for name, data, rep in smp:
        so = hdr_size + len(sdata)
        ins += be32(so) + be32(so) + be16(len(data) // 2) + be16(rep // 2)
        ins += be16(48) + be16(len(data) // 2) + pad(name, 16)
        sdata += data
    ins_section = ins + sdata
    # songs: one song, four playlists
    song = be16(1) + be32(6)
    playlist = b''.join(be16(p) for p in [0, 1, 0]) + be16(0xfffe)
    song_body = b''.join(be16(28 + k * len(playlist)) for k in range(4))
    song_body += be16(6) + be16(0) + pad('abk song', 16) + playlist * 4
    song_section = song + song_body
    songs_ofs = len(ins_section)
    pats_ofs = songs_ofs + len(song_section)
    body = be32(0x0c) + be32(0x0c + songs_ofs) + be32(0x0c + pats_ofs)
    out = b'AmBk' + be16(3) + be16(0) + be32(0) + b'Music   ' + body + ins_section + song_section + pat_section
    write('abk_amos.abk', out)


def xmf_files():
    chn = 4
    smp = [(SQ8, 0, 64, 0x08, 64, 8363), (b''.join(struct.pack('<h', v) for v in sine(256, 64, 20000)), 0, 512,
                                         0x04 | 0x08, 48, 16726), (SW8, 0, 0, 0, 56, 8363)]
    tab = b''
    pos = 0
    for data, ls, le, flags, vol, rate in smp:
        tab += le32(ls)[:3] + le32(le)[:3] + le32(pos)[:3] + le32(pos + len(data))[:3] + bytes([vol, flags])
        tab += le16(rate)
        pos += len(data)
    tab = pad(tab, 16 * 256)
    orders = [0, 1, 0]
    hdr = b'\x03' + tab + pad(bytes(orders), 256, b'\xff') + bytes([chn - 1, 1]) + bytes([0, 15, 7, 10])

    def pat(cells):
        out = b''
        for r in range(64):
            for c in range(chn):
                note, ins, fx1, fx2 = cells.get(r, {}).get(c) or (None, 0, (0, 0), (0, 0))
                out += bytes([note_index(note) - 35 if note else 0, ins, fx1[0], fx2[0], fx2[1], fx1[1]])
        return out

    p0 = pat({0: {0: ('C-4', 1, (0x0f, 4), (0x0c, 48)), 1: ('E-4', 2, (0, 0), (0, 0)),
                  2: ('G-4', 3, (0x04, 0x34), (0, 0))},
              4: {0: (None, 0, (0x0a, 0x04), (0, 0)), 3: ('C-5', 1, (0x10, 0x0c), (0x05, 0x02))},
              8: {1: (None, 0, (0x01, 0x10), (0x0e, 0xa2)), 2: ('C-4', 2, (0x03, 0x10), (0x11, 0x03))},
              16: {0: (None, 0, (0x0d, 0), (0, 0))}})
    p1 = pat({0: {0: ('D-4', 2, (0x00, 0x37), (0x06, 0x20)), 3: ('A-3', 3, (0x0e, 0x91), (0x0e, 0x42))},
              8: {2: (None, 0, (0x0b, 0), (0, 0))}})
    write('xmf_astroidea.xmf', hdr + p0 + p1 + b''.join(s[0] for s in smp))


def main():
    os.makedirs(OUT, exist_ok=True)
    mod_files()
    xm_files()
    s3m_files()
    it_files()
    amiga_files()
    stm_files()
    stx_files()
    mtm_files()
    imf_files()
    ptm_files()
    mdl_files()
    ult_files()
    liq_files()
    no_files()
    masi_files()
    masi16_files()
    gal_files()
    amf_files()
    asylum_files()
    gdm_files()
    med_files()
    oldmed_files()
    chip_files()
    rtm_files()
    pt3_files()
    dt_files()
    mgt_files()
    arch_files()
    sym_files()
    digi_files()
    dbm_files()
    emod_files()
    okt_files()
    sfx_files()
    far_files()
    umx_files()
    stim_files()
    coco_files()
    ims_files()
    c669_files()
    fnk_files()
    abk_files()
    xmf_files()


if __name__ == '__main__':
    main()
