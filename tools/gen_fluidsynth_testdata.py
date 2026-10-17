#!/usr/bin/env python3
"""Generate sdl3-mixer-fluidsynth's test data (sdl3-mixer-fluidsynth/testdata/).

Everything here is synthetic, made by this script: a small SoundFont 2.04
(with 16- and 24-bit samples, stereo pairs, global and local zones, the
generators FluidSynth implements, and modulators, linked ones among them),
MIDI files exercising the synth's channel messages, SYSEX handling,
monophonic modes, NRPNs and the player, deliberately broken SoundFonts and
MIDI files, and `cases.txt`, the list of renderings the test compares (a
SoundFont, a MIDI file, a sample rate, a frame limit and settings). No
instrument data comes from anywhere else.

The output is deterministic (integer arithmetic and a fixed LCG), so
regenerating the files gives the same bytes. If they change, the
reference output (`testdata/reference.txt`), made by a C program built
from upstream SDL_mixer's FluidSynth and SDL3, must be regenerated too.

usage: tools/gen_fluidsynth_testdata.py [OUTDIR]
"""

import os
import struct
import sys

OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
    os.path.dirname(os.path.abspath(__file__)), '..', 'sdl3-mixer-fluidsynth', 'testdata')


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


# A sine-like table in integers (a parabola per half period), so nothing
# depends on the platform's math library.
SINE = [0] * 1024
for _i in range(1024):
    _x = _i % 512
    _y = (_x * (512 - _x) * 4 * 32767) // (512 * 512)
    SINE[_i] = _y if _i < 512 else -_y


def wave(kind, n, period, amp, seed=1):
    """n samples (in 24-bit units, -2^23..2^23-1) of a waveform with the
    given period in samples and amplitude in percent."""
    rng = Lcg(seed)
    out = []
    for i in range(n):
        ph = (i * 1024 // period) % 1024
        if kind == 'sine':
            v = SINE[ph] * 256
        elif kind == 'square':
            v = 6000000 if ph < 512 else -6000000
        elif kind == 'saw':
            v = (ph - 512) * 15000
        elif kind == 'noise':
            v = (rng.next() >> 8) - (1 << 23)
        else:
            raise ValueError(kind)
        v = v * amp // 100
        out.append(max(-(1 << 23), min((1 << 23) - 1, v)))
    return out


def decaying(samples, half):
    """An integer decay with the given half life."""
    out = []
    for i, v in enumerate(samples):
        k = i // half
        f = (half - (i % half)) * 1000 // half
        scale = (1000 >> min(k, 15)) * (500 + f // 2) // 1000
        out.append(v * scale // 1000)
    return out


def riff_chunk(cid, data):
    if len(data) % 2:
        data += b'\0'
    return cid + struct.pack('<I', len(data)) + data


# --- SoundFont --------------------------------------------------------------

# generator numbers
START_OFS, END_OFS, LSTART_OFS, LEND_OFS, START_COARSE = 0, 1, 2, 3, 4
MODLFO_PITCH, VIBLFO_PITCH, MODENV_PITCH, FILTER_FC, FILTER_Q = 5, 6, 7, 8, 9
MODLFO_FC, MODENV_FC, END_COARSE, MODLFO_VOL = 10, 11, 12, 13
CHORUS, REVERB, PAN = 15, 16, 17
MODLFO_DELAY, MODLFO_FREQ, VIBLFO_DELAY, VIBLFO_FREQ = 21, 22, 23, 24
MODENV_DELAY, MODENV_ATTACK, MODENV_HOLD, MODENV_DECAY, MODENV_SUSTAIN, MODENV_RELEASE = 25, 26, 27, 28, 29, 30
KEY_MODENV_HOLD, KEY_MODENV_DECAY = 31, 32
VOLENV_DELAY, VOLENV_ATTACK, VOLENV_HOLD, VOLENV_DECAY, VOLENV_SUSTAIN, VOLENV_RELEASE = 33, 34, 35, 36, 37, 38
KEY_VOLENV_HOLD, KEY_VOLENV_DECAY = 39, 40
INSTRUMENT, KEYRANGE, VELRANGE, LSTART_COARSE, KEYNUM, VELOCITY, ATTENUATION = 41, 43, 44, 45, 46, 47, 48
LEND_COARSE, COARSE_TUNE, FINE_TUNE, SAMPLE_ID, SAMPLE_MODES = 50, 51, 52, 53, 54
SCALE_TUNING, EXCLUSIVE_CLASS, ROOT_KEY = 56, 57, 58


def gen(oper, amount):
    return struct.pack('<Hh', oper, amount)


def gen_range(oper, lo, hi):
    return struct.pack('<HBB', oper, lo, hi)


def src(index, cc=False, neg=False, bipolar=False, kind=0):
    """A modulator source: kind 0 linear, 1 concave, 2 convex, 3 switch."""
    return index | (0x80 if cc else 0) | (0x100 if neg else 0) | (0x200 if bipolar else 0) | (kind << 10)


def mod(s, dest, amount, amt_src=0, trans=0):
    return struct.pack('<HHhHH', s, dest, amount, amt_src, trans)


LINK = 127  # the "link" source (a modulator's output)


class SoundFont:
    """Builds the chunks of a SoundFont 2 (the parts tests corrupt are
    made through the keyword arguments of build())."""

    def __init__(self):
        self.smpl = []      # 24-bit sample values
        self.shdr = []
        self.insts = []     # (name, [(gens, mods)])
        self.presets = []   # (name, prog, bank, [(gens, mods)])

    def sample(self, name, data, loop, rate, pitch, corr, stype=1, link=0, pad=46):
        start = len(self.smpl)
        self.smpl += data + [0] * pad
        end = start + len(data)
        self.shdr.append([name, start, end, start + loop[0], start + loop[1], rate, pitch, corr, link, stype])
        return len(self.shdr) - 1

    def instrument(self, name, zones):
        self.insts.append((name, zones))
        return len(self.insts) - 1

    def preset(self, name, prog, bank, zones):
        self.presets.append((name, prog, bank, zones))

    def build(self, version=(2, 4), sm24=True, info_extra=b'', drop=(), pdta_extra=b'', shdr_extra=b'',
              sm24_len=None):
        smpl = b''.join(struct.pack('<h', v >> 8) for v in self.smpl)
        sm24data = bytes(v & 0xFF for v in self.smpl)
        if sm24_len is not None:
            sm24data = sm24data[:sm24_len]
        shdr = b''
        for name, start, end, ls, le, rate, pitch, corr, link, stype in self.shdr:
            shdr += name.ljust(20, b'\0') + struct.pack('<IIIIIBbHH', start, end, ls, le, rate, pitch, corr,
                                                         link, stype)
        shdr += shdr_extra
        shdr += b'EOS'.ljust(20, b'\0') + b'\0' * 26

        def zones_chunks(zone_lists, term_gen):
            bag = b''
            gens = b''
            mods = b''
            ng = nm = 0
            starts = []
            for zones in zone_lists:
                starts.append(len(bag) // 4)
                for zg, zm in zones:
                    bag += struct.pack('<HH', ng, nm)
                    for g in zg:
                        gens += g
                        ng += 1
                    for m in zm:
                        mods += m
                        nm += 1
            starts.append(len(bag) // 4)
            bag += struct.pack('<HH', ng, nm)
            gens += term_gen
            mods += b'\0' * 10
            return starts, bag, mods, gens

        istarts, ibag, imod, igen = zones_chunks([z for _, z in self.insts], gen(0, 0))
        inst = b''
        for (name, _), st in zip(self.insts, istarts):
            inst += name.ljust(20, b'\0') + struct.pack('<H', st)
        inst += b'EOI'.ljust(20, b'\0') + struct.pack('<H', istarts[-1])

        pstarts, pbag, pmod, pgen = zones_chunks([z for _, _, _, z in self.presets], gen(0, 0))
        phdr = b''
        for (name, prog, bank, _), st in zip(self.presets, pstarts):
            phdr += name.ljust(20, b'\0') + struct.pack('<HHH', prog, bank, st) + b'\0' * 12
        phdr += b'EOP'.ljust(20, b'\0') + struct.pack('<HHH', 0, 0, pstarts[-1]) + b'\0' * 12

        info = riff_chunk(b'ifil', struct.pack('<HH', *version)) + riff_chunk(b'isng', b'EMU8000\0')
        info += riff_chunk(b'INAM', b'fluidsynth test\0') + riff_chunk(b'ICMT', b'synthetic\0') + info_extra
        sdta = riff_chunk(b'smpl', smpl)
        if sm24:
            sdta += riff_chunk(b'sm24', sm24data)
        parts = [(b'phdr', phdr), (b'pbag', pbag), (b'pmod', pmod), (b'pgen', pgen), (b'inst', inst),
                 (b'ibag', ibag), (b'imod', imod), (b'igen', igen), (b'shdr', shdr)]
        pdta = b''.join(riff_chunk(c, d) for c, d in parts if c not in drop) + pdta_extra
        body = (b'sfbk' + riff_chunk(b'LIST', b'INFO' + info) + riff_chunk(b'LIST', b'sdta' + sdta)
                + riff_chunk(b'LIST', b'pdta' + pdta))
        return b'RIFF' + struct.pack('<I', len(body)) + body


def make_soundfont():
    sf = SoundFont()
    sine = sf.sample(b'sine', wave('sine', 2400, 80, 70), (400, 2400), 22050, 60, 0)
    saw = sf.sample(b'saw', wave('saw', 1600, 64, 50), (0, 1600), 32000, 67, -10)
    noise = sf.sample(b'noise', decaying(wave('noise', 3000, 1, 60, seed=7), 600), (100, 2900), 22050, 60, 0)
    # a stereo pair, linked to each other
    left = sf.sample(b'padL', wave('square', 2000, 100, 30), (200, 2000), 44100, 57, 5, stype=4, link=4)
    sf.sample(b'padR', wave('saw', 2000, 101, 30), (200, 2000), 44100, 57, 5, stype=2, link=3)
    rom = sf.sample(b'rom', wave('sine', 500, 50, 50), (0, 500), 22050, 60, 0, stype=0x8001)
    # a short sample with loop points that need sanitizing
    short = sf.sample(b'short', wave('sine', 40, 20, 90), (30, 90), 11025, 72, 0)

    lead = sf.instrument(b'Lead', [
        # global zone: envelope and filter defaults, a velocity-to-filter mod
        ([gen(VOLENV_ATTACK, -6000), gen(VOLENV_DECAY, -1000), gen(VOLENV_SUSTAIN, 120),
          gen(VOLENV_RELEASE, -2400), gen(FILTER_FC, 9000), gen(FILTER_Q, 60)],
         [mod(src(2, kind=1, neg=True), FILTER_FC, -2400),
          mod(src(1, cc=True), VIBLFO_PITCH, 50)]),
        ([gen_range(KEYRANGE, 0, 63), gen_range(VELRANGE, 0, 127), gen(VIBLFO_FREQ, -400),
          gen(VIBLFO_DELAY, -5000), gen(VIBLFO_PITCH, 15), gen(MODLFO_FREQ, 200), gen(MODLFO_VOL, 30),
          gen(MODLFO_FC, 600), gen(PAN, -250), gen(REVERB, 300), gen(CHORUS, 200), gen(SAMPLE_MODES, 1),
          gen(SAMPLE_ID, sine)], []),
        ([gen_range(KEYRANGE, 64, 127), gen(MODENV_ATTACK, -3000), gen(MODENV_DECAY, 1200),
          gen(MODENV_SUSTAIN, 400), gen(MODENV_RELEASE, -1200), gen(MODENV_PITCH, -300), gen(MODENV_FC, 2400),
          gen(KEY_MODENV_DECAY, 20), gen(KEY_VOLENV_HOLD, -30), gen(VOLENV_HOLD, -4000), gen(COARSE_TUNE, -12),
          gen(FINE_TUNE, 7), gen(SCALE_TUNING, 80), gen(SAMPLE_MODES, 3), gen(SAMPLE_ID, saw)],
         [mod(src(13), ATTENUATION, 200, src(1, cc=True, kind=2))]),
    ])
    drums = sf.instrument(b'Kit', [
        ([gen_range(KEYRANGE, 35, 36), gen(VOLENV_DECAY, -2000), gen(VOLENV_SUSTAIN, 1000), gen(ROOT_KEY, 48),
          gen(SAMPLE_ID, noise)], []),
        ([gen_range(KEYRANGE, 38, 38), gen(COARSE_TUNE, 5), gen(FILTER_FC, 7000), gen(START_OFS, 50),
          gen(SAMPLE_ID, noise)], []),
        # closed and open hi-hat share an exclusive class
        ([gen_range(KEYRANGE, 42, 42), gen(EXCLUSIVE_CLASS, 1), gen(VOLENV_DECAY, -3000), gen(VOLENV_SUSTAIN, 1440),
          gen(KEYNUM, 70), gen(SAMPLE_ID, noise)], []),
        ([gen_range(KEYRANGE, 46, 46), gen(EXCLUSIVE_CLASS, 1), gen(VELOCITY, 90), gen(SAMPLE_MODES, 1),
          gen(END_OFS, -100), gen(LSTART_OFS, 10), gen(LEND_OFS, -10), gen(SAMPLE_ID, noise)], []),
        ([gen_range(KEYRANGE, 49, 49), gen(SAMPLE_ID, rom)], []),   # a ROM sample: skipped
        ([gen_range(KEYRANGE, 50, 52), gen(SAMPLE_MODES, 1), gen(SAMPLE_ID, short)], []),
    ])
    pad = sf.instrument(b'Pad', [
        ([gen(VOLENV_ATTACK, -1200), gen(VOLENV_RELEASE, 0), gen(SAMPLE_MODES, 1)],
         [mod(src(1, cc=True, bipolar=True), 0x8001, 300),            # linked into the next one
          mod(src(LINK), FILTER_FC, 2000, src(2)),
          mod(src(10), ATTENUATION, 300, 0, 2)]),
        ([gen(PAN, -500), gen(SAMPLE_ID, left)], []),
        ([gen(PAN, 500), gen(SAMPLE_ID, left + 1)], []),
    ])
    layered = sf.instrument(b'Layer', [
        ([gen_range(VELRANGE, 0, 80), gen(ATTENUATION, 60), gen(SAMPLE_MODES, 1), gen(SAMPLE_ID, sine)],
         [mod(src(14, bipolar=True), FINE_TUNE, 30, src(16)), mod(src(3), PAN, 500)]),
        ([gen_range(VELRANGE, 60, 127), gen(START_COARSE, 0), gen(LSTART_COARSE, 0), gen(VOLENV_DECAY, 500),
          gen(VOLENV_SUSTAIN, 300), gen(SAMPLE_MODES, 1), gen(SAMPLE_ID, saw)],
         [mod(src(7, cc=True, kind=3), ATTENUATION, 480)]),
    ])

    sf.preset(b'Lead', 0, 0, [([gen(INSTRUMENT, lead)], [])])
    sf.preset(b'Pad', 1, 0, [
        ([gen(VOLENV_RELEASE, 600), gen(REVERB, 200)], [mod(src(74, cc=True), FILTER_FC, -1200)]),
        ([gen(FINE_TUNE, -15), gen(INSTRUMENT, pad)], []),
    ])
    sf.preset(b'Layer', 2, 0, [
        ([gen_range(KEYRANGE, 0, 100), gen(COARSE_TUNE, 12), gen(INSTRUMENT, layered)], []),
        ([gen_range(KEYRANGE, 48, 127), gen(ATTENUATION, 100), gen(INSTRUMENT, lead)], []),
    ])
    sf.preset(b'Lead Bank 1', 0, 1, [([gen(COARSE_TUNE, 7), gen(INSTRUMENT, lead)], [])])
    sf.preset(b'Standard', 0, 128, [([gen(INSTRUMENT, drums)], [])])
    sf.preset(b'Room', 8, 128, [([gen(FILTER_FC, -2000), gen(INSTRUMENT, drums)], [])])
    sf.preset(b'Bad zones', 5, 0, [
        ([gen(INSTRUMENT, lead), gen(PAN, 100)], []),               # a generator after the instrument
        ([gen_range(VELRANGE, 0, 50), gen_range(KEYRANGE, 0, 50), gen(INSTRUMENT, drums)], []),  # wrong order
        ([gen(PAN, 100)], [mod(src(2), 0x8000 | 70, 100)]),         # a second global zone: ignored
    ])
    return sf


# --- MIDI ------------------------------------------------------------------

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


def midi(fmt, tracks, division=96, ntracks=None):
    hdr = struct.pack('>hhh', fmt, len(tracks) if ntracks is None else ntracks, division)
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


def sysex(data):
    """A SYSEX event: the bytes after 0xF0, with the closing 0xF7."""
    return b'\xF0' + vlq(len(data) + 1) + data + b'\xF7'


def gs_dt1(addr, data):
    body = bytes([(addr >> 16) & 0x7F, (addr >> 8) & 0x7F, addr & 0x7F]) + data
    chk = (0x80 - (sum(body) & 0x7F)) & 0x7F
    return sysex(bytes([0x41, 0x10, 0x42, 0x12]) + body + bytes([chk]))


def rpn(ch, msb, lsb, value, value_lsb=None):
    ev = [(0, cc(ch, 101, msb)), (0, cc(ch, 100, lsb)), (0, cc(ch, 6, value))]
    if value_lsb is not None:
        ev.append((0, cc(ch, 38, value_lsb)))
    return ev


def nrpn(ch, msb, lsb, value, value_lsb=None):
    ev = [(0, cc(ch, 99, msb)), (0, cc(ch, 98, lsb)), (0, cc(ch, 6, value))]
    if value_lsb is not None:
        ev.append((0, cc(ch, 38, value_lsb)))
    return ev


def basic_events():
    return [
        (0, tempo(500000)),
        (0, meta(0x03, b'basic')),
        (0, prog(0, 0)), (0, prog(1, 1)), (0, prog(2, 2)),
        (0, cc(0, 7, 100)), (0, cc(0, 10, 30)), (0, cc(1, 10, 100)), (0, cc(2, 11, 90)),
        (0, cc(0, 91, 80)), (0, cc(1, 93, 100)),
    ] + rpn(0, 0, 0, 12) + [
        (12, on(0, 60)), (0, on(0, 64)), (0, on(0, 67)), (0, on(9, 36, 120)),
        (24, on(9, 42, 90)), (0, on(1, 57, 80)), (0, on(2, 52, 50)), (0, on(2, 76, 110)),
        (12, bend(0, 0x2000 + 2000)), (6, bend(0, 0x2000 - 3000)), (6, bend(0, 0x2000)),
        (0, cc(0, 1, 90)), (0, bytes([0xD0, 70])),                       # modulation, channel pressure
        (0, off(9, 36)), (0, on(9, 46, 100)), (12, on(9, 42, 100)),      # the open hi-hat is cut
        (12, off(0, 60)), (0, off(0, 64)), (0, off(0, 67)),
        (0, cc(0, 64, 127)), (0, on(0, 72)), (0, on(9, 38, 100)),       # sustained
        (24, off(0, 72)), (0, bytes([0xA1, 57, 60])),                   # key pressure
        (12, cc(0, 64, 0)), (0, off(9, 38)), (0, off(1, 57)),
        (0, on(1, 45, 0)),                                              # velocity 0: note off
        (12, cc(2, 7, 60)), (0, cc(2, 123, 0)),                         # all notes off
        (12, tempo(400000)), (0, cc(1, 120, 0)),                        # all sounds off
        (0, cc(0, 121, 0)),                                             # reset controllers
        (0, cc(1, 74, 20)), (0, on(1, 50, 100)), (24, off(1, 50)),
        (12, on(0, 62, 110)), (0, on(9, 35, 100)), (48, off(0, 62)), (0, off(9, 35)),
    ]


def format1():
    t0 = track([(0, meta(0x01, b'text')), (0, tempo(600000)), (0, meta(0x58, b'\x04\x02\x18\x08')),
                (0, meta(0x59, b'\x00\x00')), (0, meta(0x54, b'\x00\x00\x00\x00\x00')),
                (96, tempo(300000)), (0, meta(0x06, b'marker')), (0, meta(0x07, b'cue')),
                (96, tempo(450000)), (0, meta(0x7F, b'\x00\x01')), (0, meta(0x60, b'unknown')),
                (0, meta(0x02, b'(c) nobody')), (0, meta(0x04, b'instrument'))])
    # running status: one status byte for a run of notes
    run = bytes([0x90, 60, 90]) + b''.join(vlq(d) + bytes(e) for d, e in [
        (24, [60, 0]), (0, [64, 90]), (24, [64, 0]), (0, [67, 90]), (24, [67, 0]), (0, [72, 90]), (48, [72, 0])])
    t1 = b'MTrk' + struct.pack('>I', len(vlq(0) + bytes([0xC0, 1]) + vlq(0) + run + b'\x00\xFF\x2F\x00'))
    t1 += vlq(0) + bytes([0xC0, 1]) + vlq(0) + run + b'\x00\xFF\x2F\x00'
    t2 = track([(0, meta(0x05, b'la')), (0, on(9, 36)), (48, off(9, 36)), (0, meta(0x05, b'la la')),
                (0, on(9, 38)), (48, off(9, 38)), (48, on(9, 42)), (24, off(9, 42))])
    return midi(1, [t0, t1, t2])


def sysex_events():
    oct_1byte = sysex(bytes([0x7E, 0x7F, 0x08, 0x08, 0x03, 0x7F]) + bytes(
        [64 + (i * 7) % 30 - 15 for i in range(12)]))
    oct_2byte = sysex(bytes([0x7F, 0x7F, 0x08, 0x09, 0x00, 0x03, 0x7F]) + b''.join(
        bytes([0x40 + i, (i * 37) & 0x7F]) for i in range(12)))
    note_tune = sysex(bytes([0x7F, 0x7F, 0x08, 0x02, 0x00, 0x02, 60, 61, 0x10, 0x00, 64, 63, 0x7F, 0x7F]))
    note_tune_bank = sysex(bytes([0x7E, 0x7F, 0x08, 0x07, 0x01, 0x05, 0x01, 67, 66, 0x40, 0x00]))
    return [
        (0, sysex(bytes([0x7E, 0x7F, 0x09, 0x01]))),                    # GM on
        (0, prog(0, 0)), (0, prog(1, 2)),
        (12, gs_dt1(0x40007F, b'\x00')),                                 # GS reset
        (0, cc(1, 0, 1)), (0, prog(1, 0)),                               # bank 1 (GS style)
        (0, on(1, 55)), (24, off(1, 55)),
        (0, gs_dt1(0x401015, b'\x02')),                                  # part 1 (channel 0) is a drum part
        (0, on(0, 36)), (24, off(0, 36)),
        (0, gs_dt1(0x401015, b'\x00')),                                  # back to melodic
        (0, gs_dt1(0x401015, b'\x05')),                                  # invalid
        (0, sysex(bytes([0x41, 0x10, 0x42, 0x12, 0x40, 0x00, 0x7F, 0x00, 0x00]))),  # bad checksum
        (0, oct_1byte), (0, cc(0, 101, 0)), (0, cc(0, 100, 4)), (0, cc(0, 6, 0)),   # tuning bank 0
        (0, cc(0, 101, 0)), (0, cc(0, 100, 3)), (0, cc(0, 6, 3)),                   # tuning program 3
        (0, on(0, 60)), (0, on(0, 67)), (24, off(0, 60)), (0, off(0, 67)),
        (0, oct_2byte), (0, note_tune), (0, note_tune_bank),
        (0, prog(2, 0)), (0, on(2, 60)), (0, on(2, 61)), (24, off(2, 60)), (0, off(2, 61)),
        (0, sysex(bytes([0x7E, 0x10, 0x08, 0x00, 0x03]))),              # bulk dump request
        (0, sysex(bytes([0x43, 0x10, 0x4C, 0x00, 0x00, 0x7E, 0x00]))),  # XG reset
        (0, cc(3, 32, 1)), (0, prog(3, 0)), (0, on(3, 64)), (24, off(3, 64)),
        (0, sysex(bytes([0x7E, 0x22, 0x09, 0x01]))),                    # GM on, another device
        (0, sysex(bytes([0x7E, 0x7F, 0x09, 0x02]))),                    # GM off
        (0, sysex(bytes([0x7E, 0x7F, 0x09, 0x03]))),                    # GM2 on
        (0, sysex(b'')),                                                # empty
        (0, b'\xF0\x01\xF7'),                                           # only the closing byte
        (0, sysex(bytes([0x7D, 0x01, 0x02]))),                          # unknown
        (0, prog(4, 1)), (0, on(4, 60)), (48, off(4, 60)),
    ]


def mono_events():
    return [
        (0, prog(0, 0)), (0, prog(1, 2)),
        (0, cc(0, 126, 0)),                                             # mono on (poly off)
        (0, on(0, 60)), (24, on(0, 62)), (24, off(0, 60)), (12, on(0, 64)), (24, off(0, 62)),
        (0, off(0, 64)),
        (0, cc(0, 68, 127)),                                            # legato on
        (0, cc(0, 65, 127)), (0, cc(0, 5, 20)), (0, cc(0, 37, 64)),     # portamento on, time
        (0, on(0, 48)), (24, on(0, 55)), (24, off(0, 48)), (0, on(0, 60)), (24, off(0, 55)),
        (0, cc(0, 84, 72)), (0, on(0, 67)), (24, off(0, 60)), (24, off(0, 67)),
        (0, cc(0, 65, 0)), (0, cc(0, 68, 0)),
        (0, cc(0, 127, 0)),                                             # poly on
        (0, on(0, 60)), (0, on(0, 64)), (24, off(0, 60)), (0, off(0, 64)),
        (0, cc(1, 126, 0)), (0, cc(1, 2, 100)),                         # breath
        (0, on(1, 60)), (12, cc(1, 2, 30)), (12, on(1, 72)), (12, off(1, 72)), (12, off(1, 60)),
        (0, cc(0, 124, 0)), (0, cc(0, 125, 0)),                         # omni off, omni on
        (0, on(0, 57)), (48, off(0, 57)),
    ]


def nrpn_events():
    ev = [(0, prog(0, 0)), (0, prog(1, 1))]
    # SoundFont 2.01 NRPNs (MSB 120): generator offsets
    ev += nrpn(0, 120, 8, 0x40, 0x10)       # filter fc (select LSB 8 = generator 8)
    ev += nrpn(0, 120, 17, 0x50, 0x00)      # pan
    ev += [(0, cc(0, 96, 0)), (0, cc(0, 97, 0))]  # data increment, decrement
    ev += [(0, on(0, 60)), (24, off(0, 60))]
    # AWE32 NRPNs (MSB 127)
    ev += nrpn(1, 127, 16, 0x50, 0x20)
    ev += nrpn(1, 127, 21, 0x30)
    ev += [(0, on(1, 64)), (24, off(1, 64))]
    # RPNs: fine and coarse tune, modulation depth range, an unknown RPN
    ev += rpn(0, 0, 1, 0x50, 0x00) + rpn(0, 0, 2, 0x42) + rpn(0, 0, 5, 1, 0) + rpn(0, 0, 9, 1)
    ev += rpn(0, 0x7F, 0x7F, 0)
    ev += [(0, on(0, 67)), (0, cc(0, 1, 127)), (48, off(0, 67))]
    # bank select styles: MSB and LSB, then a drum program
    ev += [(0, cc(2, 0, 0)), (0, cc(2, 32, 1)), (0, prog(2, 0)), (0, on(2, 60)), (24, off(2, 60))]
    ev += [(0, prog(9, 8)), (0, on(9, 36)), (0, on(9, 38)), (24, off(9, 36)), (0, off(9, 38))]
    ev += [(0, prog(9, 99)), (0, on(9, 36)), (24, off(9, 36))]       # no such kit
    ev += [(0, prog(3, 5)), (0, on(3, 60)), (24, off(3, 60))]        # the preset with bad zones
    ev += [(0, prog(4, 77)), (0, on(4, 60)), (24, off(4, 60))]       # no such preset
    return ev


def many_notes():
    ev = [(0, prog(0, 1)), (0, prog(1, 0)), (0, prog(2, 2))]
    for i in range(40):
        ev.append((3, on(i % 3, 36 + (i * 7) % 60, 40 + (i * 13) % 80)))
    ev.append((24, cc(0, 123, 0)))
    for i in range(40):
        ev.append((0, off(1, 36 + (i * 7) % 60)))
    ev += [(0, on(9, 36)), (0, on(9, 42)), (0, on(9, 46))]
    return ev


def midi_files():
    write('midi/basic.mid', midi(0, [track(basic_events())]))
    write('midi/format1.mid', format1())
    write('midi/sysex.mid', midi(0, [track(sysex_events())]))
    write('midi/mono.mid', midi(0, [track(mono_events())]))
    write('midi/nrpn.mid', midi(0, [track(nrpn_events())]))
    write('midi/many.mid', midi(0, [track(many_notes())], division=48))
    # notes never released: the player turns them off at the end
    write('midi/hanging.mid', midi(0, [track([(0, prog(0, 1)), (0, cc(0, 64, 127)), (0, on(0, 60)),
                                              (0, on(0, 67)), (96, on(1, 64))])]))
    write('midi/empty.mid', midi(0, [track([])]))
    write('midi/notracks.mid', midi(0, []))
    write('midi/slow.mid', midi(0, [track([(0, tempo(16000000)), (0, on(0, 60)), (1, off(0, 60))])],
                                division=7))
    write('midi/fast.mid', midi(0, [track([(0, tempo(1)), (0, on(0, 60)), (500, off(0, 60))])],
                                division=0x7FFF))
    # a chunk that isn't MTrk before the track (the loop that skips it never reads another ID)
    good = track(basic_events())
    hdr1 = b'MThd' + struct.pack('>Ihhh', 6, 0, 1, 96)
    write('midi/junk_chunk.mid', hdr1 + b'JUNK' + struct.pack('>I', 4) + b'abcd' + good)
    write('midi/negative_chunk.mid', hdr1 + b'XXXX' + struct.pack('>i', -8) + good)
    write('midi/nonascii_chunk.mid', hdr1 + b'\xC0\xFF\x00\x01' + struct.pack('>I', 0) + good)
    # broken MIDI files
    basic = midi(0, [track(basic_events())])
    write('midi/truncated.mid', basic[:len(basic) * 2 // 3])
    write('midi/header_only.mid', basic[:14])
    write('midi/short_header.mid', basic[:10])
    write('midi/format2.mid', midi(2, [track(basic_events())]))
    write('midi/smpte.mid', midi(0, [track(basic_events())], division=(-(25 << 8)) | 40))
    write('midi/bad_mthd_len.mid', b'MThd' + struct.pack('>I', 7) + basic[8:])
    write('midi/more_tracks.mid', midi(1, [track(basic_events())], ntracks=3))
    write('midi/neg_tracks.mid', basic[:10] + b'\x80\x01' + basic[12:])
    write('midi/bad_varlen.mid', midi(0, [b'MTrk' + struct.pack('>I', 9) + b'\x81\x82\x83\x84\x00' +
                                          bytes([0x90, 60, 100])]))
    write('midi/no_status.mid', midi(0, [b'MTrk' + struct.pack('>I', 4) + b'\x00\x40\x40\x00']))
    write('midi/bad_eot.mid', midi(0, [track([(0, on(0, 60)), (96, off(0, 60)), (0, b'\xFF\x2F\x01\x00')],
                                             eot=False)]))
    write('midi/bad_tempo.mid', midi(0, [track([(0, b'\xFF\x51\x02\x07\xA1'), (0, on(0, 60))])]))
    write('midi/bad_timesig.mid', midi(0, [track([(0, b'\xFF\x58\x03\x04\x02\x18'), (0, on(0, 60))])]))
    write('midi/bad_keysig.mid', midi(0, [track([(0, b'\xFF\x59\x01\x00'), (0, on(0, 60))])]))
    write('midi/bad_smpte_ofs.mid', midi(0, [track([(0, b'\xFF\x54\x01\x00'), (0, on(0, 60))])]))
    write('midi/huge_meta.mid', midi(0, [track([(0, on(0, 60)), (0, b'\xFF\x01\xFF\xFF\xFF\x7F')],
                                               eot=False)]))
    write('midi/huge_sysex.mid', midi(0, [track([(0, on(0, 60)), (0, b'\xF0\xFF\xFF\xFF\x7F\x01')],
                                                eot=False)]))
    write('midi/sys_common.mid', midi(0, [track([(0, on(0, 60)), (24, b'\xF2\x01\x02'), (0, off(0, 60))])]))
    write('midi/no_eot.mid', midi(0, [track([(0, prog(0, 0)), (0, on(0, 60)), (48, off(0, 60))], eot=False)]))
    write('midi/short_track.mid', midi(0, [b'MTrk' + struct.pack('>I', 100) + b'\x00' + on(0, 60) +
                                           b'\x30' + off(0, 60)]))
    write('midi/long_track.mid', midi(0, [b'MTrk' + struct.pack('>I', 6) + b'\x00' + on(0, 60) +
                                          b'\x30' + off(0, 60) + b'\x00\xFF\x2F\x00']))
    write('midi/big_param.mid', midi(0, [track([(0, bytes([0x90, 0xC8, 0x64])), (0, bytes([0xB0, 0x07, 0xFF])),
                                                (0, bytes([0x90, 60, 0xF0])), (48, bytes([0x80, 60, 0]))])]))
    write('midi/not_midi.mid', b'RIFF\0\0\0\0RMIDdata')


def soundfonts():
    sf = make_soundfont()
    good = sf.build()
    write('test.sf2', good)
    write('test_v201.sf2', sf.build(version=(2, 1)))       # sm24 ignored
    write('sf2_truncated.sf2', good[:len(good) * 2 // 3])
    write('sf2_header.sf2', good[:12])
    write('sf2_badsize.sf2', good[:4] + struct.pack('<I', len(good)) + good[8:])
    write('sf2_version1.sf2', sf.build(version=(1, 0)))
    write('sf2_version3.sf2', sf.build(version=(3, 1)))
    write('sf2_version9.sf2', sf.build(version=(9, 0)))
    write('sf2_nopdta.sf2', sf.build(drop=(b'phdr', b'pbag', b'pmod', b'pgen', b'inst', b'ibag', b'imod',
                                           b'igen', b'shdr')))
    write('sf2_noshdr.sf2', sf.build(drop=(b'shdr',)))
    write('sf2_noimod.sf2', sf.build(drop=(b'imod',)))
    write('sf2_badinfo.sf2', sf.build(info_extra=riff_chunk(b'ICOP', b'x' * 300)))
    write('sf2_oddinfo.sf2', sf.build(info_extra=b'IENG\x05\0\0\0abcde'))
    write('sf2_extra_pdta.sf2', sf.build(pdta_extra=riff_chunk(b'zzzz', b'1234')))
    bad = make_soundfont()
    # a sample past the end of the data, a reversed loop, a bad link, a bad type
    bad.shdr.append([b'past', len(bad.smpl) - 10, len(bad.smpl) + 500, len(bad.smpl), len(bad.smpl) + 400, 22050,
                     60, 0, 0, 1])
    bad.shdr.append([b'reversed', 100, 900, 800, 200, 22050, 60, 0, 0, 1])
    bad.shdr.append([b'badlink', 0, 400, 10, 300, 22050, 60, 0, 99, 4])
    bad.shdr.append([b'badtype', 0, 400, 10, 300, 22050, 60, 0, 0, 0x40])
    bad.shdr.append([b'empty', 50, 50, 50, 50, 22050, 60, 0, 0, 1])
    bad.shdr.append([b'zerorate', 0, 400, 10, 300, 0, 255, 0, 0, 1])
    n = len(bad.shdr)
    bad.instrument(b'Broken', [
        ([gen_range(KEYRANGE, 0, 20), gen(SAMPLE_MODES, 1), gen(SAMPLE_ID, n - 6)], []),
        ([gen_range(KEYRANGE, 21, 40), gen(SAMPLE_MODES, 1), gen(SAMPLE_ID, n - 5)], []),
        ([gen_range(KEYRANGE, 41, 60), gen(SAMPLE_ID, n - 4)], []),
        ([gen_range(KEYRANGE, 61, 70), gen(SAMPLE_ID, n - 3)], []),
        ([gen_range(KEYRANGE, 71, 80), gen(SAMPLE_ID, n - 2)], []),
        ([gen_range(KEYRANGE, 81, 90), gen(SAMPLE_MODES, 1), gen(SAMPLE_ID, n - 1)], []),
        ([gen_range(KEYRANGE, 91, 100), gen(SAMPLE_ID, 200)], []),         # no such sample
        ([gen(PAN, 20), gen_range(KEYRANGE, 101, 127), gen(SAMPLE_ID, 0)], []),   # key range not first
        ([gen_range(KEYRANGE, 101, 127)], [mod(src(1, cc=True), 0x8000 | 50, 100),   # link to nowhere
                                          mod(src(0x99, cc=True), FILTER_FC, 100),   # invalid CC source
                                          mod(src(3, kind=7), PAN, 100)]),          # invalid curve
    ])
    bad.preset(b'Broken', 9, 0, [([gen(INSTRUMENT, len(bad.insts) - 1)], [])])
    write('sf2_badsamples.sf2', bad.build(sm24=False))
    bad.preset(b'Bad reference', 10, 0, [([gen(INSTRUMENT, 99)], [])])
    write('sf2_badinst.sf2', bad.build(sm24=False))
    write('sf2_badsm24.sf2', make_soundfont().build(sm24_len=10))


CASES = [
    # name, soundfont, MIDI file, rate, frame limit, settings
    ('basic', 'test.sf2', 'basic.mid', 44100, 400000, ''),
    ('basic48k', 'test.sf2', 'basic.mid', 48000, 400000, ''),
    ('basic22k', 'test.sf2', 'basic.mid', 22050, 200000, ''),
    ('basic8k', 'test.sf2', 'basic.mid', 8000, 100000, ''),
    ('basic96k', 'test.sf2', 'basic.mid', 96000, 400000, ''),
    ('basic_dry', 'test.sf2', 'basic.mid', 44100, 400000, 'synth.reverb.active=0 synth.chorus.active=0'),
    ('basic_fx', 'test.sf2', 'basic.mid', 44100, 400000,
     'synth.reverb.room-size=0.95 synth.reverb.damp=0.1 synth.reverb.width=100 synth.reverb.level=1.0 '
     'synth.chorus.nr=7 synth.chorus.speed=3.5 synth.chorus.depth=40 synth.chorus.level=2'),
    ('basic_tri', 'test.sf2', 'basic.mid', 44100, 400000, 'synth.chorus.nr=0'),
    ('basic_gain', 'test.sf2', 'basic.mid', 44100, 400000, 'synth.gain=7.5 synth.min-note-length=50'),
    ('basic_v201', 'test_v201.sf2', 'basic.mid', 44100, 400000, ''),
    ('basic_limit', 'test.sf2', 'basic.mid', 44100, 30000, ''),
    ('format1', 'test.sf2', 'format1.mid', 44100, 400000, ''),
    ('format1_noreset', 'test.sf2', 'format1.mid', 32000, 400000, 'player.reset-synth=0'),
    ('sysex', 'test.sf2', 'sysex.mid', 44100, 400000, ''),
    ('sysex_dev', 'test.sf2', 'sysex.mid', 44100, 400000, 'synth.device-id=34'),
    ('sysex_xg', 'test.sf2', 'sysex.mid', 44100, 400000, 'synth.midi-bank-select=xg'),
    ('mono', 'test.sf2', 'mono.mid', 44100, 400000, ''),
    ('mono_linear', 'test.sf2', 'mono.mid', 44100, 400000, 'synth.portamento-time=linear'),
    ('mono_xg', 'test.sf2', 'mono.mid', 44100, 400000, 'synth.portamento-time=xg-gs'),
    ('nrpn', 'test.sf2', 'nrpn.mid', 44100, 400000, ''),
    ('nrpn_mma', 'test.sf2', 'nrpn.mid', 44100, 400000, 'synth.midi-bank-select=mma'),
    ('nrpn_gm', 'test.sf2', 'nrpn.mid', 44100, 400000, 'synth.midi-bank-select=gm'),
    ('many', 'test.sf2', 'many.mid', 44100, 400000, ''),
    ('many_poly8', 'test.sf2', 'many.mid', 44100, 400000, 'synth.polyphony=8'),
    ('many_prio', 'test.sf2', 'many.mid', 44100, 400000,
     'synth.polyphony=12 synth.overflow.percussion=-4000 synth.overflow.released=0 synth.overflow.sustained=500 '
     'synth.overflow.volume=0 synth.overflow.age=2000 synth.overflow.important=100 '
     'synth.overflow.important-channels=1,3'),
    ('many_cut', 'test.sf2', 'many.mid', 44100, 400000, 'synth.note-cut=2 synth.midi-channels=32'),
    ('hanging', 'test.sf2', 'hanging.mid', 44100, 300000, ''),
    ('empty', 'test.sf2', 'empty.mid', 44100, 300000, ''),
    ('notracks', 'test.sf2', 'notracks.mid', 44100, 300000, ''),
    ('slow', 'test.sf2', 'slow.mid', 44100, 300000, ''),
    ('fast', 'test.sf2', 'fast.mid', 44100, 300000, ''),
    ('nosf', '-', 'basic.mid', 44100, 300000, ''),
    ('junk_chunk', 'test.sf2', 'junk_chunk.mid', 44100, 300000, ''),
    ('negative_chunk', 'test.sf2', 'negative_chunk.mid', 44100, 300000, ''),
    ('nonascii_chunk', 'test.sf2', 'nonascii_chunk.mid', 44100, 300000, ''),
    ('truncated', 'test.sf2', 'truncated.mid', 44100, 300000, ''),
    ('header_only', 'test.sf2', 'header_only.mid', 44100, 300000, ''),
    ('short_header', 'test.sf2', 'short_header.mid', 44100, 300000, ''),
    ('format2', 'test.sf2', 'format2.mid', 44100, 300000, ''),
    ('smpte', 'test.sf2', 'smpte.mid', 44100, 300000, ''),
    ('bad_mthd_len', 'test.sf2', 'bad_mthd_len.mid', 44100, 300000, ''),
    ('more_tracks', 'test.sf2', 'more_tracks.mid', 44100, 300000, ''),
    ('neg_tracks', 'test.sf2', 'neg_tracks.mid', 44100, 300000, ''),
    ('bad_varlen', 'test.sf2', 'bad_varlen.mid', 44100, 300000, ''),
    ('no_status', 'test.sf2', 'no_status.mid', 44100, 300000, ''),
    ('bad_eot', 'test.sf2', 'bad_eot.mid', 44100, 300000, ''),
    ('bad_tempo', 'test.sf2', 'bad_tempo.mid', 44100, 300000, ''),
    ('bad_timesig', 'test.sf2', 'bad_timesig.mid', 44100, 300000, ''),
    ('bad_keysig', 'test.sf2', 'bad_keysig.mid', 44100, 300000, ''),
    ('bad_smpte_ofs', 'test.sf2', 'bad_smpte_ofs.mid', 44100, 300000, ''),
    ('huge_meta', 'test.sf2', 'huge_meta.mid', 44100, 300000, ''),
    ('huge_sysex', 'test.sf2', 'huge_sysex.mid', 44100, 300000, ''),
    ('sys_common', 'test.sf2', 'sys_common.mid', 44100, 300000, ''),
    ('no_eot', 'test.sf2', 'no_eot.mid', 44100, 300000, ''),
    ('short_track', 'test.sf2', 'short_track.mid', 44100, 300000, ''),
    ('long_track', 'test.sf2', 'long_track.mid', 44100, 300000, ''),
    ('big_param', 'test.sf2', 'big_param.mid', 44100, 300000, ''),
    ('not_midi', 'test.sf2', 'not_midi.mid', 44100, 300000, ''),
    ('sf_truncated', 'sf2_truncated.sf2', 'basic.mid', 44100, 300000, ''),
    ('sf_header', 'sf2_header.sf2', 'basic.mid', 44100, 300000, ''),
    ('sf_badsize', 'sf2_badsize.sf2', 'basic.mid', 44100, 300000, ''),
    ('sf_version1', 'sf2_version1.sf2', 'basic.mid', 44100, 300000, ''),
    ('sf_version3', 'sf2_version3.sf2', 'basic.mid', 44100, 300000, ''),
    ('sf_version9', 'sf2_version9.sf2', 'basic.mid', 44100, 300000, ''),
    ('sf_nopdta', 'sf2_nopdta.sf2', 'basic.mid', 44100, 300000, ''),
    ('sf_noshdr', 'sf2_noshdr.sf2', 'basic.mid', 44100, 300000, ''),
    ('sf_noimod', 'sf2_noimod.sf2', 'basic.mid', 44100, 300000, ''),
    ('sf_badinfo', 'sf2_badinfo.sf2', 'basic.mid', 44100, 300000, ''),
    ('sf_oddinfo', 'sf2_oddinfo.sf2', 'basic.mid', 44100, 300000, ''),
    ('sf_extra_pdta', 'sf2_extra_pdta.sf2', 'basic.mid', 44100, 300000, ''),
    ('sf_badsamples', 'sf2_badsamples.sf2', 'many.mid', 44100, 300000, ''),
    ('sf_badsamples9', 'sf2_badsamples.sf2', 'nrpn.mid', 44100, 300000, ''),
    ('sf_badsm24', 'sf2_badsm24.sf2', 'basic.mid', 44100, 300000, ''),
    ('sf_badinst', 'sf2_badinst.sf2', 'basic.mid', 44100, 300000, ''),
]


def cases():
    lines = ['# name soundfont midi rate max-frames settings...\n']
    for name, sf, mid, rate, frames, settings in CASES:
        lines.append(' '.join(x for x in [name, sf, 'midi/' + mid, str(rate), str(frames), settings] if x) + '\n')
    write('cases.txt', ''.join(lines))


def main():
    soundfonts()
    midi_files()
    cases()


if __name__ == '__main__':
    main()
