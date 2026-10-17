#!/usr/bin/env python3
"""Generate sdl3-mixer's tracker module test files (sdl3-mixer/src/testdata/modules/).

The modules are written byte by byte here, from synthetic samples (square,
saw and sine waves and LCG noise), so there's nothing copyrighted in them
and they stay small (a second or two of music each). Each one exercises a
loader of libxmp (translated in sdl3-mixer/src/xmp/) and a set of its
effects, envelopes, loops and instrument features; the IT files carry
samples compressed the way Impulse Tracker 2.14 and 2.15 compress them,
and the OXM file an Ogg Vorbis sample (testdata/audio/ogg_mono.ogg).

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
    hm = smp[:2] + [None, None]
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
    del hm
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
    smp = [('square', u8(square(64, 16, 100)), 0, 64, 0, 64, 0), ('sine16', u16le(sine(256, 64, 20000)), 128, 512, 2, 60, 1),
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
        {0: ('C-4', 1, 0xf0, 0x03, 0), 4: (None, 0, 0xa0, 0x04, 0), 8: ('G-4', 2, 0x30, 0x10, 0), 14: (None, 0, 0xd0, 0, 0),
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


if __name__ == '__main__':
    main()
