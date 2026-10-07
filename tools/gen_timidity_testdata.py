#!/usr/bin/env python3
"""Generate sdl3-mixer-timidity's test data (sdl3-mixer-timidity/testdata/).

Everything here is synthetic, made by this script: small Gravis Ultrasound
patches (.pat) with computed waveforms, a small SoundFont 2, TiMidity
configuration files, and MIDI files exercising TiMidity's events, banks,
drum sets and file formats, along with deliberately broken patches,
configurations and MIDI files. No instrument data or configuration comes
from anywhere else.

The output is deterministic (integer arithmetic and a fixed LCG), so
regenerating the files gives the same bytes. If they change, the
reference output (`testdata/reference.txt`, and sdl3-mixer's
`testdata/reference.txt`), made by a C program built from upstream
SDL_mixer's TiMidity and SDL3, must be regenerated too.

usage: tools/gen_timidity_testdata.py [OUTDIR]
"""

import os
import struct
import sys

OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
    os.path.dirname(os.path.abspath(__file__)), '..', 'sdl3-mixer-timidity', 'testdata')


class Lcg:
    def __init__(self, seed):
        self.s = seed & 0xFFFFFFFF

    def next(self):
        self.s = (self.s * 1664525 + 1013904223) & 0xFFFFFFFF
        return self.s


def write(rel, data):
    path = os.path.join(OUT, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    if isinstance(data, str):
        data = data.encode('latin-1')
    with open(path, 'wb') as f:
        f.write(data)


# A sine table in integers (0..32767 amplitude), so nothing depends on the
# platform's math library.
SINE = [0] * 1024


def make_sine():
    # Bhaskara-free: a parabola approximation, good enough for test tones
    # and exactly reproducible.
    for i in range(1024):
        x = i % 512
        y = (x * (512 - x) * 4 * 32767) // (512 * 512)
        SINE[i] = y if i < 512 else -y


make_sine()


def wave(kind, n, period, amp, seed=1):
    """n 16-bit samples of a waveform with the given period (in samples)."""
    rng = Lcg(seed)
    out = []
    for i in range(n):
        ph = (i * 1024 // period) % 1024
        if kind == 'sine':
            v = SINE[ph]
        elif kind == 'square':
            v = 24000 if ph < 512 else -24000
        elif kind == 'saw':
            v = (ph - 512) * 60
        elif kind == 'tri':
            v = (ph * 128 - 32768) if ph < 512 else (32767 - (ph - 512) * 128)
        elif kind == 'noise':
            v = (rng.next() >> 16) - 32768
        else:
            raise ValueError(kind)
        # a decay over the sample, so non-looping samples fade out
        if kind == 'noise' or kind == 'decay':
            pass
        out.append(max(-32768, min(32767, v * amp // 100)))
    return out


def decaying(samples, half):
    """Apply an exponential-ish integer decay with the given half life."""
    out = []
    for i, v in enumerate(samples):
        k = i // half
        f = (half - (i % half)) * 1000 // half  # 1000..1
        scale = (1000 >> min(k, 15)) * (500 + f // 2) // 1000
        out.append(v * scale // 1000)
    return out


# --- GUS patches ----------------------------------------------------------

MODES_16BIT = 1
MODES_UNSIGNED = 2
MODES_LOOPING = 4
MODES_PINGPONG = 8
MODES_REVERSE = 16
MODES_SUSTAIN = 32
MODES_ENVELOPE = 64


def gus_sample(data, *, bits=16, unsigned=False, loop=None, rate=22050, low=0, high=12543854,
               root=261626, pan=7, env_rates=(0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F),
               env_offsets=(245, 245, 245, 10, 10, 10), tremolo=(0, 0, 0), vibrato=(0, 0, 0),
               modes=0, fractions=0, name=b'wave', data_length=None):
    """One sample: its 96-byte header and its data."""
    if bits == 16:
        modes |= MODES_16BIT
        if unsigned:
            raw = b''.join(struct.pack('<H', (v + 32768) & 0xFFFF) for v in data)
        else:
            raw = b''.join(struct.pack('<h', v) for v in data)
        mult = 2
    else:
        if unsigned:
            raw = bytes(((v >> 8) + 128) & 0xFF for v in data)
        else:
            raw = bytes((v >> 8) & 0xFF for v in data)
        mult = 1
    if unsigned:
        modes |= MODES_UNSIGNED
    if loop is not None:
        ls, le = loop[0] * mult, loop[1] * mult
    else:
        ls, le = 0, len(raw)
    h = name[:7].ljust(7, b'\0')
    h += bytes([fractions])
    h += struct.pack('<iii', len(raw) if data_length is None else data_length, ls, le)
    h += struct.pack('<H', rate)
    h += struct.pack('<iii', low, high, root)
    h += struct.pack('<h', 512)  # tune
    h += bytes([pan])
    h += bytes(env_rates) + bytes(env_offsets)
    h += bytes(tremolo) + bytes(vibrato)
    h += bytes([modes])
    h += struct.pack('<hh', 2, 1024)
    h += b'\0' * 36
    assert len(h) == 96
    return h + raw


def gus_patch(samples, *, magic=b'GF1PATCH110\0ID#000002\0', instruments=1, layers=1,
              nsamples=None):
    hdr = magic + b'synthetic test patch'.ljust(60, b'\0')
    hdr += bytes([instruments, 14, 0]) + struct.pack('<H', len(samples)) + struct.pack('<H', 127)
    hdr += struct.pack('<I', sum(len(s) - 96 for s in samples)) + b'\0' * 36
    assert len(hdr) == 129
    hdr += struct.pack('<H', 0) + b'test'.ljust(16, b'\0') + struct.pack('<I', 0) + bytes([layers])
    hdr += b'\0' * 40
    assert len(hdr) == 192
    hdr += bytes([0, 0]) + struct.pack('<I', 0)
    hdr += bytes([len(samples) if nsamples is None else nsamples]) + b'\0' * 40
    assert len(hdr) == 239
    return hdr + b''.join(samples)


def patches():
    # A two-sample "piano": looped, with an envelope, split at 300 Hz.
    lo = decaying(wave('saw', 3000, 84, 90), 1500)
    hi = decaying(wave('tri', 3000, 42, 80), 1500)
    env = dict(env_rates=(0x30, 0x2A, 0x20, 0x1C, 0x18, 0x10),
               env_offsets=(240, 230, 200, 40, 20, 10))
    write('patches/piano.pat', gus_patch([
        gus_sample(lo, loop=(1000, 2008), low=0, high=300000, root=261626, fractions=0x31,
                   modes=MODES_LOOPING | MODES_ENVELOPE, **env),
        gus_sample(hi, loop=(1000, 2008), low=300001, high=12543854, root=523251, pan=10,
                   modes=MODES_LOOPING | MODES_ENVELOPE | MODES_SUSTAIN, **env),
    ]))
    # 8-bit unsigned square, looped, with tremolo, no envelope.
    sq = wave('square', 2000, 50, 70)
    write('patches/square8.pat', gus_patch([
        gus_sample(sq, bits=8, unsigned=True, loop=(500, 1500), rate=11025, root=220000,
                   tremolo=(10, 40, 60), modes=MODES_LOOPING, pan=3),
    ]))
    # Vibrato with a sweep, an envelope, looped.
    vb = wave('sine', 4000, 100, 90)
    write('patches/vib.pat', gus_patch([
        gus_sample(vb, loop=(1000, 3000), root=220500, vibrato=(20, 50, 80),
                   tremolo=(0, 30, 40), modes=MODES_LOOPING | MODES_ENVELOPE | MODES_SUSTAIN,
                   env_rates=(0x28, 0x28, 0x20, 0x14, 0x14, 0x14),
                   env_offsets=(230, 220, 210, 30, 20, 10)),
    ]))
    # Bidirectional loop.
    pp = wave('tri', 4000, 120, 85)
    write('patches/pingpong.pat', gus_patch([
        gus_sample(pp, loop=(800, 3200), root=183750, fractions=0x80,
                   modes=MODES_LOOPING | MODES_PINGPONG | MODES_ENVELOPE | MODES_SUSTAIN,
                   env_rates=(0x30, 0x30, 0x30, 0x18, 0x18, 0x18),
                   env_offsets=(240, 235, 230, 20, 10, 10)),
    ]))
    # Not looped, played at a fixed pitch (note=60 in the configuration):
    # pre-resampled at load time.
    fx = decaying(wave('sine', 6000, 60, 95), 2000)
    write('patches/fixed.pat', gus_patch([
        gus_sample(fx, root=367500, rate=22050),
    ]))
    # A reverse loop, which TiMidity plays reversed.
    rv = decaying(wave('saw', 3000, 70, 80), 1200)
    write('patches/reverse.pat', gus_patch([
        gus_sample(rv, loop=(600, 2400), root=315000,
                   modes=MODES_LOOPING | MODES_REVERSE | MODES_ENVELOPE | MODES_SUSTAIN,
                   env_rates=(0x30, 0x30, 0x30, 0x20, 0x20, 0x20),
                   env_offsets=(240, 240, 240, 20, 10, 10)),
    ]))
    # 16-bit unsigned, with the older "GF1PATCH100" header.
    us = wave('sine', 2500, 80, 60)
    write('patches/unsigned16.pat', gus_patch([
        gus_sample(us, unsigned=True, loop=(400, 2000), root=275625,
                   modes=MODES_LOOPING),
    ], magic=b'GF1PATCH100\0ID#000002\0', instruments=0, layers=0))
    # A patch under a name with a space (the configuration quotes it).
    qt = wave('square', 2000, 64, 50)
    write('patches/quoted name.pat', gus_patch([
        gus_sample(qt, loop=(200, 1800), root=344531, modes=MODES_LOOPING),
    ]))
    # Drums: not looped; drums play at their own note (pre-resampled).
    write('patches/kick.pat', gus_patch([
        gus_sample(decaying(wave('sine', 5000, 300, 100), 900), root=65406, rate=22050),
    ]))
    write('patches/snare.pat', gus_patch([
        gus_sample(decaying(wave('noise', 4000, 1, 70, seed=7), 700), root=146832,
                   rate=22050, pan=9),
    ]))
    write('patches/hat.pat', gus_patch([
        gus_sample(decaying(wave('noise', 2000, 1, 40, seed=9), 300), bits=8,
                   root=185000, rate=11025, pan=4, loop=(100, 1500),
                   modes=MODES_LOOPING | MODES_ENVELOPE),
    ]))
    # A drum played through a loop with an envelope (keep=loop keep=env).
    write('patches/tom.pat', gus_patch([
        gus_sample(decaying(wave('tri', 4000, 150, 90), 1500), root=98000, loop=(1000, 3000),
                   modes=MODES_LOOPING | MODES_ENVELOPE,
                   env_rates=(0x30, 0x30, 0x30, 0x1C, 0x1C, 0x1C),
                   env_offsets=(240, 230, 220, 30, 20, 10)),
    ]))

    # Broken patches.
    good = gus_patch([gus_sample(wave('sine', 1000, 50, 80), loop=(100, 900), root=440000,
                                 modes=MODES_LOOPING)])
    write('patches/truncated.pat', good[:239 + 96 + 500])           # data cut short
    write('patches/shortheader.pat', good[:200])                      # header cut short
    write('patches/badmagic.pat', b'GF1PATCH999' + good[11:])          # not a patch
    write('patches/twoinstruments.pat', good[:82] + b'\x02' + good[83:])
    write('patches/twolayers.pat', good[:151] + b'\x02' + good[152:])
    write('patches/negsamples.pat', good[:198] + b'\x90' + good[199:])  # (signed char) -112
    # Two samples said, one there.
    write('patches/missingsample.pat', good[:198] + b'\x02' + good[199:])
    # A sample said to be 2 GB long.
    big = gus_patch([gus_sample(wave('sine', 100, 50, 80), root=440000, data_length=0x7FFFFFF0)])
    write('patches/hugesample.pat', big)


# --- configuration files ------------------------------------------------------

def configs():
    write('timidity.cfg', '''\
# TiMidity configuration for sdl3-mixer-timidity's tests.
# (comments, blank lines and TiMidity++'s extensions are skipped)

dir patches
#extension comm "a comment"
opt -EFreverb=0
timeout 10 20
map gs 0 0 0 0

bank 0
0 patches/piano
1 patches/square8 pan=-100 amp=120
2 patches/vib.pat
3 patches/pingpong strip=tail
4 patches/fixed note=60 pan=-50
5 patches/reverse keep=env
6 "patches/quoted name" pan=right
7 patches/missing
8 patches/truncated
9 patches/badmagic
10 patches/negsamples
11 patches/unsigned16 pan=center amp=80
12 patches/twolayers
13 patches/shortheader
14 patches/missingsample
15 patches/hugesample
16 patches/piano strip=loop
17 patches/vib strip=env
18 patches/piano keep=loop keep=env amp=0
19 patches/twoinstruments
drumset 0
36 patches/kick
38 patches/snare
42 patches/hat strip=env keep=loop
45 patches/tom keep=loop keep=env
46 patches/hat strip=loop
drumset 1
36 patches/snare
38 patches/kick note=40
bank 1
0 patches/square8
4 patches/vib
source extra.cfg
''')
    write('extra.cfg', '''\
# Sourced by timidity.cfg: its own bank.
comm program second
HTTPproxy example:80
FTPproxy example:21
mailaddr nobody
copydrumset 0
copybank 0
undef 3
altassign 1 2 3
progbase 1
bank 2
0 patches/vib pan=100
1 'patches/square8' strip=env
''')
    # A configuration whose default instrument plays every channel
    # without a program change. Its `dir` is relative to the current
    # directory, which the tests make sdl3-mixer-timidity/ (the other
    # configurations name their patches relative to themselves).
    write('default.cfg', '''\
dir testdata/patches
default unsigned16
bank 0
0 piano
drumset 0
36 kick
''')
    # Broken configurations (each must fail to load).
    bad = {
        'bad_syntax.cfg': 'bank 0\npiano 0\n',
        'bad_nobank.cfg': '0 piano\n',
        'bad_program.cfg': 'bank 0\n128 piano\n',
        'bad_banknum.cfg': 'bank 128\n',
        'bad_drumnum.cfg': 'drumset -1\n',
        'bad_option.cfg': 'bank 0\n0 piano loud=1\n',
        'bad_noequals.cfg': 'bank 0\n0 piano amp\n',
        'bad_amp.cfg': 'bank 0\n0 piano amp=900\n',
        'bad_note.cfg': 'bank 0\n0 piano note=x\n',
        # (upstream accepts a word: it parses as 63)
        'pan_word.cfg': 'bank 0\n0 piano pan=wide\n',
        # (upstream rejects "left": it parses as 0, and isn't a number)
        'bad_panleft.cfg': 'bank 0\n0 piano pan=left\n',
        'bad_keep.cfg': 'bank 0\n0 piano keep=tail\n',
        'bad_strip.cfg': 'bank 0\n0 piano strip=all\n',
        'bad_quote.cfg': 'bank 0\n0 "piano\n',
        'bad_quote2.cfg': 'bank 0\n0 "pia\'no"\n',
        'bad_quote3.cfg': 'bank 0\n0 "piano"x\n',
        'bad_quote4.cfg': 'bank 0\n0 pi"ano"\n',
        'bad_dir.cfg': 'dir\n',
        'bad_source.cfg': 'source\n',
        'bad_source_missing.cfg': 'source nowhere.cfg\n',
        'bad_source_loop.cfg': 'source bad_source_loop.cfg\n',
        'bad_default.cfg': 'default a b\n',
        'bad_soundfont.cfg': 'soundfont\n',
        'bad_soundfont_missing.cfg': 'soundfont nowhere.sf2\n',
        'bad_font.cfg': 'font\n',
        'bad_font_exclude.cfg': 'font exclude\n',
        'bad_font_order.cfg': 'font order 1\n',
    }
    for name, text in bad.items():
        write(name, text)
    # SoundFont configurations.
    write('sf2.cfg', '''\
dir patches
soundfont test.sf2 order=0
font exclude 0 5
font order 1 0 7
bank 0
1 patches/square8
''')
    write('sf2_badorder.cfg', 'soundfont test.sf2 order=x\n')
    write('sf2_badoption.cfg', 'soundfont test.sf2 orderly\n')


# --- SoundFont 2 ----------------------------------------------------------------

def riff_chunk(cid, data):
    if len(data) % 2:
        data += b'\0'
    return cid + struct.pack('<I', len(data)) + data


def soundfont():
    # Two samples: a looped sine and a decaying noise burst.
    s0 = wave('sine', 3000, 100, 80)
    s1 = decaying(wave('noise', 2000, 1, 60, seed=3), 500)
    smpl = b''
    shdr = b''

    def add_sample(name, data, loop, rate, pitch, corr, stype=1):
        nonlocal smpl, shdr
        start = len(smpl) // 2
        smpl += b''.join(struct.pack('<h', v) for v in data) + b'\0' * 92  # 46 zero samples
        end = start + len(data)
        shdr += name.ljust(20, b'\0') + struct.pack('<IIIII', start, end, start + loop[0],
                                                     start + loop[1], rate)
        shdr += bytes([pitch, corr]) + struct.pack('<HH', 0, stype)

    add_sample(b'sine', s0, (500, 2500), 22050, 57, 0)
    add_sample(b'noise', s1, (0, 1999), 22050, 60, 0)
    add_sample(b'rom', s1, (0, 1999), 22050, 60, 0, 0x8001)
    shdr += b'EOS'.ljust(20, b'\0') + b'\0' * 26

    def gen(oper, amount):
        return struct.pack('<Hh', oper, amount)

    def gen_range(oper, lo, hi):
        return struct.pack('<HBB', oper, lo, hi)

    # Instrument zones.
    igen = b''
    ibag = b''
    inst = b''
    nigen = 0

    def zone(gens):
        nonlocal igen, ibag, nigen
        ibag += struct.pack('<HH', nigen, 0)
        for g in gens:
            igen += g
            nigen += 1

    def instrument(name, zones):
        nonlocal inst
        inst += name.ljust(20, b'\0') + struct.pack('<H', len(ibag) // 4)
        for z in zones:
            zone(z)

    instrument(b'Sine', [
        # a global zone
        [gen(48, 30), gen(17, -200)],
        [gen_range(43, 0, 70), gen(34, -3000), gen(35, -4000), gen(36, -1200), gen(37, 200),
         gen(38, -2400), gen(6, 40), gen(24, -800), gen(13, 30), gen(51, 0), gen(52, 10),
         gen(54, 1), gen(53, 0)],
        [gen_range(43, 71, 127), gen(58, 69), gen(51, -1), gen(54, 3), gen(56, 50),
         gen(8, 9000), gen(9, 100), gen(53, 0)],
    ])
    instrument(b'Kit', [
        [gen_range(43, 36, 36), gen(53, 1)],
        [gen_range(43, 38, 38), gen(51, 5), gen(53, 1)],
        [gen_range(43, 40, 40), gen(53, 2)],    # a ROM sample: skipped
    ])
    instrument(b'Fifth', [
        [gen(51, 7), gen(54, 1), gen(53, 0)],
    ])
    inst += b'EOI'.ljust(20, b'\0') + struct.pack('<H', len(ibag) // 4)
    ibag += struct.pack('<HH', nigen, 0)
    igen += gen(0, 0)

    # Presets.
    pgen = b''
    pbag = b''
    phdr = b''
    npgen = 0

    def preset(name, prog, bank, zones):
        nonlocal pgen, pbag, phdr, npgen
        phdr += name.ljust(20, b'\0') + struct.pack('<HHH', prog, bank, len(pbag) // 4)
        phdr += b'\0' * 12
        for z in zones:
            pbag += struct.pack('<HH', npgen, 0)
            for g in z:
                pgen += g
                npgen += 1

    preset(b'Sine', 0, 0, [[gen(51, 0)], [gen(41, 0)]])
    preset(b'Drums', 0, 128, [[gen(41, 1)]])
    preset(b'Fifth', 2, 0, [[gen(41, 2), gen(52, -20)]])
    preset(b'Excluded', 5, 0, [[gen(41, 2)]])
    preset(b'Ordered', 7, 0, [[gen(41, 0)]])
    phdr += b'EOP'.ljust(20, b'\0') + struct.pack('<HHH', 0, 0, len(pbag) // 4) + b'\0' * 12
    pbag += struct.pack('<HH', npgen, 0)
    pgen += gen(0, 0)

    info = riff_chunk(b'ifil', struct.pack('<HH', 2, 1)) + riff_chunk(b'INAM', b'test\0')
    info += riff_chunk(b'ICMT', b'synthetic\0')
    sdta = riff_chunk(b'smpl', smpl)
    pdta = (riff_chunk(b'phdr', phdr) + riff_chunk(b'pbag', pbag)
            + riff_chunk(b'pmod', b'\0' * 10) + riff_chunk(b'pgen', pgen)
            + riff_chunk(b'inst', inst) + riff_chunk(b'ibag', ibag)
            + riff_chunk(b'imod', b'\0' * 10) + riff_chunk(b'igen', igen)
            + riff_chunk(b'shdr', shdr))
    body = (b'sfbk' + riff_chunk(b'LIST', b'INFO' + info) + riff_chunk(b'LIST', b'sdta' + sdta)
            + riff_chunk(b'LIST', b'pdta' + pdta))
    sf2 = b'RIFF' + struct.pack('<I', len(body)) + body
    write('test.sf2', sf2)
    # Broken SoundFonts.
    write('sf2_truncated.sf2', sf2[:len(sf2) // 2])
    write('sf2_badsize.sf2', sf2[:4] + struct.pack('<I', len(body) + 2) + sf2[8:])
    v3 = sf2.replace(b'ifil' + struct.pack('<IHH', 4, 2, 1), b'ifil' + struct.pack('<IHH', 4, 3, 0))
    write('sf2_version3.sf2', v3)


# --- MIDI files -----------------------------------------------------------------

def vlq(n):
    out = [n & 0x7F]
    n >>= 7
    while n:
        out.append(0x80 | (n & 0x7F))
        n >>= 7
    return bytes(reversed(out))


def track(events, eot=True):
    """events: (delta, bytes) pairs."""
    data = b''.join(vlq(d) + e for d, e in events)
    if eot:
        data += b'\x00\xFF\x2F\x00'
    return b'MTrk' + struct.pack('>I', len(data)) + data


def midi(fmt, tracks, division=96, header_extra=b''):
    hdr = struct.pack('>hhh', fmt, len(tracks), division) + header_extra
    return b'MThd' + struct.pack('>I', len(hdr)) + hdr + b''.join(tracks)


def on(ch, note, vel=100):
    return bytes([0x90 | ch, note, vel])


def off(ch, note, vel=64):
    return bytes([0x80 | ch, note, vel])


def cc(ch, c, v):
    return bytes([0xB0 | ch, c, v])


def prog(ch, p):
    return bytes([0xC0 | ch, p])


def bend(ch, v):
    return bytes([0xE0 | ch, v & 0x7F, v >> 7])


def tempo(us):
    return b'\xFF\x51\x03' + struct.pack('>I', us)[1:]


def meta(t, data):
    return bytes([0xFF, t]) + vlq(len(data)) + data


def notes(ch, seq, dur, vel=100, gap=0):
    """A melody: (delta, event) pairs, each note dur ticks long."""
    ev = []
    first = True
    for n in seq:
        ev.append((0 if first else gap, on(ch, n, vel)))
        ev.append((dur, off(ch, n)))
        first = False
    return ev


def basic_events():
    ev = [
        (0, tempo(500000)),
        (0, meta(0x03, b'basic')),
        (0, prog(0, 0)), (0, prog(1, 1)), (0, prog(2, 2)), (0, prog(3, 3)),
        (0, cc(0, 7, 100)), (0, cc(0, 10, 30)), (0, cc(1, 10, 100)), (0, cc(2, 11, 90)),
        # pitch bend sensitivity: 12 semitones on channel 0
        (0, cc(0, 101, 0)), (0, cc(0, 100, 0)), (0, cc(0, 6, 12)),
        # an NRPN data entry, ignored
        (0, cc(1, 99, 1)), (0, cc(1, 98, 2)), (0, cc(1, 6, 40)),
        (24, on(0, 60)), (0, on(0, 64)), (0, on(0, 67)), (0, on(9, 36, 120)),
        (48, on(9, 42, 90)), (0, on(1, 57, 80)),
        (24, bend(0, 0x2000 + 2000)), (12, bend(0, 0x2000 - 3000)), (12, bend(0, 0x2000)),
        (0, off(9, 36)), (0, off(9, 42)),
        (24, off(0, 60)), (0, off(0, 64)), (0, off(0, 67)),
        (0, cc(0, 64, 127)), (0, on(0, 72)), (0, on(2, 55)), (0, on(9, 38, 100)),
        (48, off(0, 72)), (0, bytes([0xA2, 55, 60])),  # key pressure
        (24, cc(0, 64, 0)), (0, off(9, 38)),
        (0, on(3, 48)), (0, on(3, 79)), (0, on(1, 45, 0)),  # velocity 0: note off
        (48, cc(2, 7, 60)), (0, cc(1, 123, 0)),  # all notes off
        (24, off(2, 55)), (0, tempo(400000)),
        (0, cc(3, 120, 0)),  # all sounds off
        (0, cc(0, 121, 0)),  # reset controllers
        (0, cc(0, 101, 0x7F)), (0, cc(0, 100, 0x7F)), (0, cc(0, 6, 0)),  # RPN reset
        (24, on(0, 62, 110)), (0, on(9, 46, 100)), (96, off(0, 62)), (0, off(9, 46)),
        (48, cc(0, 10, 64)), (0, on(0, 50)), (0, on(9, 45)), (120, off(0, 50)),
        (0, off(9, 45)), (96, bytes([0xF0, 0x05, 0x7E, 0x7F, 0x09, 0x01, 0xF7])),
    ]
    return ev


def midi_files():
    write('midi/basic.mid', midi(0, [track(basic_events())]))

    # Format 1: a tempo map, a melody with running status, drums.
    t0 = track([(0, tempo(600000)), (0, meta(0x58, b'\x04\x02\x18\x08')),
                (192, tempo(450000)), (192, meta(0x01, b'text'))])
    mel = [(0, prog(0, 5)), (0, prog(1, 6)), (0, prog(2, 4)), (0, prog(3, 11))]
    run = bytes([0x90, 60, 90])  # running status after this
    mel += [(0, run), (48, bytes([64, 90])), (48, bytes([67, 90])), (48, bytes([60, 0])),
            (0, bytes([64, 0])), (0, bytes([67, 0]))]
    mel += notes(1, [62, 66, 69], 40, 100, 8)
    mel += [(0, on(2, 60)), (96, off(2, 60)), (0, on(3, 55)), (0, on(3, 67)), (96, off(3, 55)),
            (0, off(3, 67)), (0, bytes([0xD3, 40]))]  # channel pressure: ignored
    t1 = track(mel)
    dr = []
    for i in range(8):
        dr += [(0 if i == 0 else 24, on(9, 36 if i % 2 == 0 else 38, 100)), (24, off(9, 36 if i % 2 == 0 else 38))]
    dr += [(0, bytes([0xF7, 0x02, 0x01, 0x02]))]  # escaped SysEx
    t2 = track(dr)
    write('midi/format1.mid', midi(1, [t0, t1, t2], 120))

    # Format 2: tracks played one after the other.
    a = track([(0, prog(0, 0))] + notes(0, [60, 62, 64, 65], 48))
    b = track([(0, prog(1, 2))] + notes(1, [67, 65, 64, 62], 48))
    write('midi/format2.mid', midi(2, [a, b]))

    # RMID: a Standard MIDI File in a RIFF container.
    smf = midi(0, [track([(0, prog(0, 1))] + notes(0, [57, 60, 64, 69], 60))])
    rmid = b'RMID' + b'data' + struct.pack('<I', len(smf)) + smf
    write('midi/rmid.rmi', b'RIFF' + struct.pack('<I', len(rmid)) + rmid)

    # Banks and drum sets: bank 1 (defined), bank 5 (not: bank 0), drum
    # set 1, a program bank 1 doesn't have (falls back to bank 0's).
    ev = [(0, cc(0, 0, 1)), (0, prog(0, 0)), (0, on(0, 60)), (96, off(0, 60)),
          (0, prog(0, 4)), (0, on(0, 64)), (96, off(0, 64)),
          (0, prog(0, 2)), (0, on(0, 67)), (96, off(0, 67)),
          (0, cc(0, 0, 5)), (0, prog(0, 1)), (0, on(0, 62)), (96, off(0, 62)),
          (0, cc(1, 0, 2)), (0, prog(1, 0)), (0, on(1, 69)), (0, prog(1, 1)), (96, off(1, 69)),
          (0, on(1, 71)), (96, off(1, 71)),
          (0, prog(9, 1)), (0, on(9, 36)), (48, off(9, 36)), (0, on(9, 38)), (48, off(9, 38)),
          (0, prog(9, 9)), (0, on(9, 36)), (48, off(9, 36)),
          (0, cc(9, 0, 3)), (0, cc(0, 32, 1)), (0, cc(0, 32, 0)),
          (0, prog(2, 7)), (0, on(2, 60)), (0, prog(3, 8)), (0, on(3, 60)),
          (0, prog(4, 9)), (0, on(4, 60)), (0, prog(5, 12)), (0, on(5, 60)),
          (0, prog(6, 13)), (0, on(6, 60)), (0, prog(7, 14)), (0, on(7, 60)),
          (0, prog(8, 15)), (0, on(8, 60)), (0, prog(10, 16)), (0, on(10, 64)),
          (0, prog(11, 17)), (0, on(11, 64)), (0, prog(12, 18)), (0, on(12, 64)),
          (0, prog(13, 19)), (0, on(13, 60)), (48, off(13, 60)),
          (96, off(2, 60)), (0, off(3, 60)), (0, off(10, 64)), (0, off(11, 64)), (0, off(12, 64))]
    write('midi/banks.mid', midi(0, [track(ev)]))

    # A long note through each looping patch, with vibrato and tremolo
    # running, released at different times; high and low notes.
    ev = [(0, prog(0, 2)), (0, prog(1, 1)), (0, prog(2, 3)), (0, prog(3, 5)), (0, prog(4, 0)),
          (0, cc(4, 10, 0)), (0, cc(2, 10, 127)), (0, cc(3, 10, 64))]
    ev += [(0, on(0, 57)), (0, on(1, 45)), (0, on(2, 72)), (0, on(3, 50)), (0, on(4, 36)),
           (0, on(4, 96))]
    ev += [(192, off(1, 45)), (96, off(0, 57)), (48, off(2, 72)), (96, off(3, 50)),
           (0, off(4, 36)), (0, off(4, 96)), (0, on(2, 30)), (0, on(0, 90)), (192, off(2, 30)),
           (0, off(0, 90)), (192, on(9, 42))]
    write('midi/sustained.mid', midi(0, [track(ev)], 96))

    # More notes than voices: voices are cut, then lost.
    ev = [(0, prog(c, 0)) for c in range(9)] + [(0, cc(0, 64, 127))]
    for k in range(300):
        c = k % 9
        ev.append((0, on(c, 30 + (k * 7) % 70, 40 + k % 80)))
    ev.append((96, cc(0, 64, 0)))
    for k in range(300):
        c = k % 9
        ev.append((0, off(c, 30 + (k * 7) % 70)))
    ev.append((48, on(0, 60)))
    ev.append((48, off(0, 60)))
    write('midi/polyphony.mid', midi(0, [track(ev)]))

    # SMPTE time division (25 fps, 40 ticks per frame), and a header
    # longer than 6 bytes.
    write('midi/smpte.mid', midi(0, [track([(0, prog(0, 1))] + notes(0, [60, 67, 72], 400))],
                                 division=(-25 << 8) | 40, header_extra=b'\x00\x01'))
    # No notes at all.
    write('midi/empty.mid', midi(0, [track([(0, meta(0x03, b'empty'))])]))
    # Silence before the first note is skipped; a tempo change in it.
    write('midi/lead_in.mid', midi(0, [track([(0, tempo(1000000)), (480, prog(0, 6)),
                                              (480, tempo(300000))]
                                             + notes(0, [60, 63], 96))]))
    # A note on an instrument whose patch says it has -112 samples (an
    # allocation that fails: the song fails to load).
    write('midi/oom.mid', midi(0, [track([(0, prog(0, 10))] + notes(0, [60], 48))]))
    # The configuration's default instrument (default.cfg).
    write('midi/default.mid', midi(0, [track(notes(0, [60, 64], 96) + notes(1, [55], 96)
                                             + [(0, prog(2, 0))] + notes(2, [72], 96)
                                             + notes(9, [36], 48))]))
    # The SoundFont's instruments (sf2.cfg).
    write('midi/sf2.mid', midi(0, [track([(0, prog(0, 0)), (0, prog(1, 1)), (0, prog(2, 2)),
                                          (0, prog(3, 5)), (0, prog(4, 7))]
                                         + [(0, on(0, 60)), (0, on(0, 80)), (0, on(2, 50)),
                                            (0, on(3, 64)), (0, on(4, 62)), (0, on(9, 36)),
                                            (0, on(9, 38)), (0, on(9, 40)), (0, on(1, 67))]
                                         + [(192, off(0, 60)), (0, off(0, 80)), (0, off(2, 50)),
                                            (0, off(3, 64)), (0, off(4, 62)), (0, off(1, 67)),
                                            (96, off(9, 36)), (0, off(9, 38))])]))

    # Broken files.
    good = midi(0, [track(notes(0, [60, 62], 48))])
    write('midi/bad_track_magic.mid', good[:14] + b'MTrx' + good[18:])
    write('midi/bad_format.mid', good[:8] + struct.pack('>h', 3) + good[10:])
    write('midi/bad_tracks0.mid', good[:10] + struct.pack('>h', 0) + good[12:])
    write('midi/bad_format0_2tracks.mid', midi(0, [track(notes(0, [60], 48))] * 2))
    write('midi/bad_header_len.mid', good[:4] + struct.pack('>I', 5) + good[8:])
    write('midi/bad_missing_track.mid', midi(1, [track(notes(0, [60], 48))])[:8]
          + struct.pack('>hhh', 1, 2, 96) + midi(1, [track(notes(0, [60], 48))])[14:])
    write('midi/bad_no_eot.mid', midi(0, [track(notes(0, [60, 64], 48), eot=False)]))
    write('midi/bad_rmid.rmi', b'RIFF' + struct.pack('<I', 20) + b'RMIX' + b'data' + good)
    # Track lengths longer and shorter than the track.
    t = track(notes(0, [60, 64], 48))
    write('midi/track_len_long.mid', midi(0, [])[:10] + struct.pack('>h', 1) + midi(0, [])[12:]
          + t[:4] + struct.pack('>I', len(t) - 8 + 50) + t[8:])
    write('midi/track_len_short.mid', midi(0, [])[:10] + struct.pack('>h', 1) + midi(0, [])[12:]
          + t[:4] + struct.pack('>I', 3) + t[8:])


def main():
    patches()
    configs()
    soundfont()
    midi_files()


if __name__ == '__main__':
    main()
