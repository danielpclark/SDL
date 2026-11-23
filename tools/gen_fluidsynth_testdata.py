#!/usr/bin/env python3
"""Generate sdl3-mixer-fluidsynth's test data (sdl3-mixer-fluidsynth/testdata/).

Everything here is synthetic, made by this script: a small SoundFont 2.04
(with 16- and 24-bit samples, stereo pairs, global and local zones, the
generators FluidSynth implements, and modulators, linked ones among them),
MIDI files exercising the synth's channel messages, SYSEX handling,
monophonic modes, NRPNs and the player, deliberately broken SoundFonts and
MIDI files, DLS files (written from the DLS specifications: instruments
with their regions, articulations and aliases, broken files, and files
without instruments), and `cases.txt`, the list of renderings the test
compares (a SoundFont or DLS file, a MIDI file, a sample rate, a frame
limit and settings). No instrument data comes from anywhere else.

The output is deterministic (integer arithmetic and a fixed LCG), so
regenerating the files gives the same bytes. If they change, the
reference outputs (`testdata/reference.txt`, and `testdata/reference_dls.txt`
for the DLS files), made by C programs built from upstream SDL_mixer's
FluidSynth and SDL3, must be regenerated too.

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


def mono_events(prog1=2):
    return [
        (0, prog(0, 0)), (0, prog(1, prog1)),
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


def many_notes(progs=(1, 0, 2), drums=()):
    ev = [(0, prog(c, p)) for c, p in enumerate(progs)] + list(drums)
    for i in range(40):
        ev.append((3, on(i % 3, 36 + (i * 7) % 60, 40 + (i * 13) % 80)))
    ev.append((24, cc(0, 123, 0)))
    for i in range(40):
        ev.append((0, off(1, 36 + (i * 7) % 60)))
    ev += [(0, on(9, 36)), (0, on(9, 42)), (0, on(9, 46))]
    return ev


def dls_basic_events():
    """test.dls's instruments (GS bank numbers): chords, bends and
    controllers, velocity layers, conditional regions, the Key Number
    generator, the drum kits' key groups, and a program that isn't there."""
    return [
        (0, tempo(500000)),
        (0, meta(0x03, b'dls basic')),
        (0, prog(0, 0)),                                                # Piano
        (0, cc(1, 0, 1)), (0, cc(1, 32, 2)), (0, prog(1, 48)),          # Strings (bank 1/2)
        (0, prog(2, 16)), (0, prog(3, 80)),                             # Organ, Square lead
        (0, prog(4, 40)), (0, prog(5, 41)),                             # Keynum two, Keynum one
        (0, cc(6, 0, 121)), (0, prog(6, 1)),                            # Bright GM2
        (0, cc(7, 0, 120)), (0, prog(7, 0)),                            # GM2 Rhythm
        (0, prog(8, 4)), (0, prog(9, 8)), (0, prog(10, 1)),             # XG voice, Kit, none
        (0, cc(0, 7, 100)), (0, cc(0, 10, 30)), (0, cc(1, 10, 100)), (0, cc(1, 11, 90)),
        (0, cc(0, 91, 80)), (0, cc(1, 93, 100)), (0, cc(2, 91, 40)),
    ] + rpn(0, 0, 0, 12) + rpn(1, 0, 0, 2) + [
        (12, on(0, 60)), (0, on(0, 64)), (0, on(0, 67)), (0, on(9, 36, 120)),
        (24, on(1, 57, 40)), (0, on(1, 64, 110)), (0, on(2, 52)), (0, on(2, 76, 90)),
        (12, bend(0, 0x2000 + 2000)), (0, bend(1, 0x2000 - 3000)), (6, bend(0, 0x2000)),
        (0, cc(1, 1, 90)), (0, bytes([0xD1, 70])), (0, bytes([0xA1, 64, 60])),
        (0, on(9, 42, 90)), (12, on(9, 46, 100)),
        (12, off(0, 60)), (0, off(0, 64)), (0, off(0, 67)), (0, off(9, 36)),
        (0, on(3, 72)), (0, on(4, 30)), (0, on(4, 60)), (0, on(5, 60)), (0, on(6, 60)), (0, on(7, 50)),
        (0, on(8, 64)), (0, on(10, 72)),
        (24, off(1, 57)), (0, off(1, 64)), (0, off(2, 52)), (0, off(2, 76)), (0, cc(1, 1, 0)),
        (0, off(3, 72)), (0, off(4, 30)), (0, off(4, 60)), (0, off(5, 60)), (0, off(6, 60)), (0, off(7, 50)),
        (0, off(8, 64)), (0, off(10, 72)),
        (0, prog(9, 25)), (0, on(9, 42)), (12, on(9, 44)), (12, on(9, 46)), (12, on(9, 38)),   # key group 1
        (0, cc(0, 64, 127)), (0, on(0, 72)), (24, off(0, 72)), (12, cc(0, 64, 0)),
        (0, cc(9, 0, 1)), (0, prog(9, 2)), (0, on(9, 40)), (0, on(9, 70)),
        (24, off(9, 40)), (0, off(9, 42)), (0, off(9, 44)), (0, off(9, 46)), (0, off(9, 38)), (0, off(9, 70)),
        (12, on(0, 62, 110)), (0, on(1, 50, 20)), (48, off(0, 62)), (0, off(1, 50)),
    ]


def dls_bank_events():
    """Bank selects (MSB, LSB) and programs on a melodic and the drum
    channel, which each bank select style reads its own way."""
    ev = [(0, tempo(300000))]
    for msb, lsb, p in [(0, 0, 0), (0, 3, 4), (1, 2, 48), (1, 0, 48), (0, 2, 48), (121, 0, 1), (120, 0, 0),
                        (0, 0, 1), (127, 0, 0), (0, 0, 8), (1, 1, 2), (2, 1, 0)]:
        ev += [(0, cc(0, 0, msb)), (0, cc(0, 32, lsb)), (0, prog(0, p)), (0, on(0, 60)), (12, off(0, 60))]
    for msb, lsb, p in [(0, 0, 8), (0, 0, 25), (1, 0, 2), (0, 0, 2), (120, 0, 0), (0, 1, 8)]:
        ev += [(0, cc(9, 0, msb)), (0, cc(9, 32, lsb)), (0, prog(9, p)), (0, on(9, 42)), (0, on(9, 46)),
               (12, off(9, 42)), (0, off(9, 46))]
    return ev


def dls_tuning_events():
    """A channel's MIDI tuning moves the key a region is chosen by: an
    octave tuning (+60 cents on D#), then single note tunings."""
    octave = sysex(bytes([0x7E, 0x7F, 0x08, 0x08, 0x03, 0x7F, 0x7F]) + bytes(
        [0x40 + 60 if i == 3 else 0x40 for i in range(12)]))
    notes = sysex(bytes([0x7F, 0x7F, 0x08, 0x02, 0x03, 0x02, 62, 66, 0x00, 0x00, 70, 57, 0x40, 0x00]))
    return [
        (0, prog(0, 0)), (0, prog(1, 40)), (0, prog(9, 8)),
        (0, octave),
        (0, on(0, 63)), (0, on(1, 63)), (0, on(9, 63)), (24, off(0, 63)), (0, off(1, 63)), (0, off(9, 63)),
        (0, notes),
    ] + rpn(0, 0, 4, 0) + rpn(0, 0, 3, 3) + rpn(1, 0, 4, 0) + rpn(1, 0, 3, 3) + [
        (0, on(0, 62)), (0, on(1, 62)), (24, off(0, 62)), (0, off(1, 62)),
        (0, on(0, 70)), (0, on(1, 70)), (24, off(0, 70)), (0, off(1, 70)),
    ]


def dls_drum_events():
    """dls_pgal.dls's drum note aliasing and instrument aliases."""
    ev = [(0, prog(9, 8)), (0, prog(0, 3)), (0, cc(1, 0, 1)), (0, cc(1, 32, 5)), (0, prog(1, 81))]
    for key in (35, 36, 40, 82, 100, 127):
        ev += [(0, on(9, key)), (12, off(9, key))]
    ev += [(0, on(0, 60)), (0, on(1, 72)), (24, off(0, 60)), (0, off(1, 72))]
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
    # for test.dls (and the other DLS files that load)
    write('midi/dls_basic.mid', midi(0, [track(dls_basic_events())]))
    write('midi/dls_banks.mid', midi(0, [track(dls_bank_events())]))
    write('midi/dls_tuning.mid', midi(0, [track(dls_tuning_events())]))
    write('midi/dls_drums.mid', midi(0, [track(dls_drum_events())]))
    # Strings: two velocity layers, a region articulation on one
    write('midi/dls_mono.mid', midi(0, [track([(0, cc(1, 0, 1)), (0, cc(1, 32, 2))] + mono_events(48))]))
    write('midi/dls_many.mid', midi(0, [track(many_notes((16, 0, 80), [(0, prog(9, 25))]))], division=48))


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


# --- DLS --------------------------------------------------------------------

# Written from the MMA's Downloadable Sounds specifications (DLS Level 1 and
# Level 2.2): a DLS file is a RIFF form of type 'DLS ' holding a collection
# header ('colh': the number of instruments), the instruments ('LIST'
# 'lins' of 'LIST' 'ins ', each with an instrument header 'insh' and its
# regions, 'LIST' 'lrgn' of 'LIST' 'rgn ' with a region header 'rgnh', an
# optional wave sample chunk 'wsmp' and a wave link 'wlnk'), the pool table
# ('ptbl': the offsets of the waves in the wave pool) and the wave pool
# ('LIST' 'wvpl' of 'LIST' 'wave', each a WAVE file's format 'fmt ', data
# and optional 'wsmp'), with names in 'LIST' 'INFO' 'INAM' chunks.
# Conditional chunks ('cdl ') hold expressions over the DLSIDs of
# dls2.h, which decide whether what holds them is used. A chunk's size
# doesn't count the pad byte that keeps the next chunk at an even offset.
#
# The first test files stop at the instruments: the broken files fail
# before them, and the files that read everything else have no 'lins'.
# test.dls and the files after it have instruments.

def dls_ck(cid, data, size=None):
    """A chunk with the given data and, if given, a size other than the
    data's (a corrupt one), padded to an even length."""
    n = len(data) if size is None else size
    return cid + struct.pack('<I', n) + data + (b'\0' if len(data) % 2 else b'')


def dls_list(ltype, chunks, size=None):
    return dls_ck(b'LIST', ltype + b''.join(chunks), size)


def dls_info(name, other=()):
    """An INFO list: other entries, then the name (INAM, NUL-terminated)."""
    return dls_list(b'INFO', list(other) + [dls_ck(b'INAM', name + b'\0')])


def dls_fmt(bits=16, rate=22050, channels=1, tag=1):
    """A WAVEFORMAT: PCM (tag 1) of the given sample size, rate and
    channels."""
    align = channels * bits // 8
    return dls_ck(b'fmt ', struct.pack('<HHIIHH', tag, channels, rate, rate * align, align, bits))


def dls_wsmp(unity=60, fine=0, gain=0, loop=None, cbsize=20, loop_cbsize=16, loops=None, size=None):
    """A wave sample chunk: the unity note, fine tune (cents), gain
    (1/65536 cB) and at most one loop (type, start, length); cbsize,
    loop_cbsize and loops override the structure's own (the corrupt
    ones)."""
    nloops = (1 if loop else 0) if loops is None else loops
    data = struct.pack('<IHhiII', cbsize, unity, fine, gain, 0, nloops)
    if loop:
        data += struct.pack('<IIII', loop_cbsize, *loop)
    return dls_ck(b'wsmp', data, size)


def dls_data(values, bits=16, size=None):
    """A data chunk of 16-bit samples, or 8-bit ones (unsigned, 128 being
    0), from 16-bit values."""
    if bits == 16:
        data = b''.join(struct.pack('<h', v) for v in values)
    else:
        data = bytes((v >> 8) + 128 for v in values)
    return dls_ck(b'data', data, size)


def dls_values(kind, n, period, amp, seed=1):
    """n 16-bit samples of a waveform (see wave())."""
    return [v >> 8 for v in wave(kind, n, period, amp, seed)]


def dls_ins(name, bank, prog, regions, drums=False):
    """An instrument: its bank (MSB << 8 | LSB), program, drum flag and
    regions (key range and wave index, and a wsmp chunk or None)."""
    rgns = []
    for (lo, hi), index, wsmp in regions:
        chunks = [dls_ck(b'rgnh', struct.pack('<HHHHHH', lo, hi, 0, 127, 0, 0))]
        if wsmp:
            chunks.append(wsmp)
        chunks.append(dls_ck(b'wlnk', struct.pack('<HHII', 0, 0, 1, index)))
        rgns.append(dls_list(b'rgn ', chunks))
    locale = struct.pack('<II', bank | (0x80000000 if drums else 0), prog)
    insh = dls_ck(b'insh', struct.pack('<I', len(regions)) + locale)
    return dls_list(b'ins ', [insh, dls_list(b'lrgn', rgns), dls_info(name)])


# DLSIDs (dls2.h: Data1, Data2, Data3, Data4)
def dlsid(d1, d2, d3, *d4):
    return struct.pack('<IHH8B', d1, d2, d3, *d4)


DLSID_GM = dlsid(0x178f2f24, 0xc364, 0x11d1, 0xa7, 0x60, 0x00, 0x00, 0xf8, 0x75, 0xac, 0x12)
DLSID_GS = dlsid(0x178f2f25, 0xc364, 0x11d1, 0xa7, 0x60, 0x00, 0x00, 0xf8, 0x75, 0xac, 0x12)
DLSID_XG = dlsid(0x178f2f26, 0xc364, 0x11d1, 0xa7, 0x60, 0x00, 0x00, 0xf8, 0x75, 0xac, 0x12)
DLSID_DLS1 = dlsid(0x178f2f27, 0xc364, 0x11d1, 0xa7, 0x60, 0x00, 0x00, 0xf8, 0x75, 0xac, 0x12)
DLSID_DLS2 = dlsid(0xf14599e5, 0x4689, 0x11d2, 0xaf, 0xa6, 0x00, 0xaa, 0x00, 0x24, 0xd8, 0xb6)
DLSID_MEMORY = dlsid(0x178f2f28, 0xc364, 0x11d1, 0xa7, 0x60, 0x00, 0x00, 0xf8, 0x75, 0xac, 0x12)
DLSID_MANUFACTURER = dlsid(0xb03e1181, 0x8095, 0x11d2, 0xa1, 0xef, 0x00, 0x60, 0x08, 0x33, 0xdb, 0xd8)
DLSID_PRODUCT = dlsid(0xb03e1182, 0x8095, 0x11d2, 0xa1, 0xef, 0x00, 0x60, 0x08, 0x33, 0xdb, 0xd8)
DLSID_RATE = dlsid(0x2a91f713, 0xa4bf, 0x11d2, 0xbb, 0xdf, 0x00, 0x60, 0x08, 0x33, 0xdb, 0xd8)
DLSID_UNKNOWN = dlsid(0x12345678, 0x9abc, 0xdef0, 1, 2, 3, 4, 5, 6, 7, 8)

# conditional chunk opcodes
CDL_AND, CDL_OR, CDL_XOR, CDL_ADD, CDL_SUBTRACT, CDL_MULTIPLY, CDL_DIVIDE = 1, 2, 3, 4, 5, 6, 7
CDL_LOGICAL_AND, CDL_LOGICAL_OR, CDL_LT, CDL_LE, CDL_GT, CDL_GE, CDL_EQ, CDL_NOT = 8, 9, 10, 11, 12, 13, 14, 15


def cdl_op(op):
    return struct.pack('<H', op)


def cdl_const(v):
    return struct.pack('<HI', 0x10, v & 0xFFFFFFFF)


def cdl_query(guid):
    return struct.pack('<H', 0x11) + guid


def cdl_supported(guid):
    return struct.pack('<H', 0x12) + guid


class Dls:
    """Builds a DLS file: its waves and instruments, with the top-level
    chunks tests corrupt given to build()."""

    def __init__(self):
        self.waves = []         # LIST wave chunks
        self.instruments = []   # LIST ins chunks

    def wave(self, chunks):
        self.waves.append(dls_list(b'wave', chunks))
        return len(self.waves) - 1

    def instrument(self, *args, **kwargs):
        self.instruments.append(dls_ins(*args, **kwargs))

    def cues(self):
        """The waves' offsets in the pool."""
        offsets, pos = [], 0
        for w in self.waves:
            offsets.append(pos)
            pos += len(w)
        return offsets

    def build(self, before=(), after=(), drop=(), replace=None, cues=None, ptbl_cbsize=8, form=b'DLS ',
              riff_size=None, tail=b''):
        """The file: before, the standard chunks (but those dropped or
        replaced), after, in a RIFF form whose size is riff_size if given,
        and tail after it."""
        cues = self.cues() if cues is None else cues
        chunks = {
            'vers': dls_ck(b'vers', struct.pack('<II', 0x00020002, 0)),
            'dlid': dls_ck(b'dlid', dlsid(0x0a9e7f3c, 0x5d21, 0x4b6e, 0x9f, 0x40, 0x12, 0x8c, 0x77, 0x1d, 0x05, 0xe3)),
            'colh': dls_ck(b'colh', struct.pack('<I', len(self.instruments))),
            'INFO': dls_info(b'Synthetic DLS', [dls_ck(b'ICMT', b'made by gen_fluidsynth_testdata.py')]),
            'lins': dls_list(b'lins', self.instruments),
            'ptbl': dls_ck(b'ptbl', struct.pack('<II', ptbl_cbsize, len(cues)) + b'\0' * (ptbl_cbsize - 8) +
                           b''.join(struct.pack('<I', c) for c in cues)),
            'wvpl': dls_list(b'wvpl', self.waves),
        }
        chunks.update(replace or {})
        body = list(before) + [c for k, c in chunks.items() if k not in drop] + list(after)
        data = form + b''.join(body)
        size = len(data) if riff_size is None else riff_size
        return b'RIFF' + struct.pack('<I', size) + data + tail


def make_dls():
    d = Dls()
    sine = d.wave([dls_fmt(16, 22050), dls_wsmp(60, 0, 0, (0, 8, 40)), dls_data(dls_values('sine', 48, 24, 70)),
                   dls_info(b'Sine 16')])
    square = d.wave([dls_info(b'Square 8 bit'), dls_ck(b'dlid', DLSID_UNKNOWN), dls_fmt(8, 11025),
                     dls_ck(b'guid', DLSID_UNKNOWN), dls_ck(b'fact', struct.pack('<I', 48)),
                     dls_ck(b'cue ', struct.pack('<I', 0)), dls_data(dls_values('square', 48, 16, 40), 8)])
    # (cbSize fields below the structures' sizes, as some files have them)
    saw = d.wave([dls_fmt(16, 32000), dls_wsmp(67, -10, -65536 * 6, (1, 4, 30), cbsize=12, loop_cbsize=8),
                  dls_data(dls_values('saw', 40, 20, 50)), dls_info(b'Saw', [dls_ck(b'ICMT', b'odd')])])
    noise = d.wave([dls_fmt(16, 22050), dls_ck(b'smpl', b'\0' * 36), dls_list(b'wavu', [dls_ck(b'abcd', b'')]),
                    dls_wsmp(60, 5, 0, (2, 0, 16)), dls_data(dls_values('noise', 32, 1, 60, seed=7))])
    d.instrument(b'Piano', 0, 0, [((0, 63), sine, None), ((64, 127), saw, None)])
    d.instrument(b'Square lead', 0, 80, [((0, 127), square, dls_wsmp(72))])
    d.instrument(b'Kit', 0, 8, [((35, 81), noise, None)], drums=True)
    return d


def dls_files():
    d = make_dls()
    good = d.build()
    nolins = dict(drop=('lins',))

    # the header
    write('dls_short.dls', good[:11])
    write('dls_notriff.dls', b'RIFX' + good[4:])
    write('dls_notdls.dls', d.build(form=b'sfbk'))
    write('dls_badform.dls', d.build(form=b'\xe9L\0S'))
    write('dls_toolong.dls', d.build(riff_size=len(good) - 8 + 100))
    write('dls_hint.dls', d.build(riff_size=len(good)))
    write('dls_extra.dls', d.build(tail=b'more', **nolins))

    # the top level: every chunk read, unknown ones, a ptbl with a larger
    # cbSize, then no instruments
    write('dls_nolins.dls', d.build(ptbl_cbsize=12, after=[
        dls_ck(b'zzzz', b'abc'), dls_list(b'junk', [dls_ck(b'abcd', b'1234')]), dls_ck(b'x\xe9\0y', b'')], **nolins))
    write('dls_nowvpl.dls', d.build(drop=('wvpl',)))
    write('dls_badlist.dls', d.build(before=[b'LIST' + struct.pack('<I', 2) + b'ab']))
    write('dls_truncchunk.dls', d.build(after=[b'abcd']))
    write('dls_colh.dls', d.build(replace={'colh': dls_ck(b'colh', struct.pack('<II', 3, 0))}))
    write('dls_info.dls', d.build(replace={'INFO': dls_list(b'INFO', [b'LIST' + struct.pack('<I', 2) + b'ab'])}))
    write('dls_ptbl_cbsize.dls', d.build(replace={'ptbl': dls_ck(b'ptbl', struct.pack('<II', 4, 0))}))
    write('dls_ptbl_cues.dls', d.build(replace={'ptbl': dls_ck(b'ptbl', struct.pack('<II', 8, 0x40000000))}))
    write('dls_ptbl_size.dls', d.build(replace={'ptbl': dls_ck(b'ptbl', struct.pack('<III', 8, 2, 0))}))
    write('dls_wvpl_chunk.dls', d.build(replace={'wvpl': dls_ck(b'wvpl', b''.join(d.waves))}, **nolins))
    write('dls_wvpl_size.dls', d.build(replace={'wvpl': dls_list(b'wvpl', d.waves, 4 + len(b''.join(d.waves)) + 64)}))

    # conditional chunks
    def cdl(*ops, **kwargs):
        return d.build(before=[dls_ck(b'cdl ', b''.join(ops))], **kwargs)

    def check(ops, expected):
        return list(ops) + [cdl_const(expected), cdl_op(CDL_EQ), cdl_op(CDL_LOGICAL_AND)]

    ops = [cdl_const(1)]
    ops += check([cdl_const(0xF0F0), cdl_const(0x0FF0), cdl_op(CDL_AND)], 0x00F0)
    ops += check([cdl_const(0xF000), cdl_const(0x000F), cdl_op(CDL_OR)], 0xF00F)
    ops += check([cdl_const(0xFF00), cdl_const(0x0FF0), cdl_op(CDL_XOR)], 0xF0F0)
    ops += check([cdl_const(0xFFFFFFFF), cdl_const(2), cdl_op(CDL_ADD)], 1)
    ops += check([cdl_const(7), cdl_const(5), cdl_op(CDL_SUBTRACT)], 5 - 7)
    ops += check([cdl_const(0x10000), cdl_const(0x10001), cdl_op(CDL_MULTIPLY)], 0x10000)
    ops += check([cdl_const(3), cdl_const(100), cdl_op(CDL_DIVIDE)], 33)
    ops += check([cdl_const(0), cdl_const(9), cdl_op(CDL_LOGICAL_AND)], 0)
    ops += check([cdl_const(0), cdl_const(9), cdl_op(CDL_LOGICAL_OR)], 1)
    for op, expected in ((CDL_LT, 1), (CDL_LE, 1), (CDL_GT, 0), (CDL_GE, 0)):
        ops += check([cdl_const(2), cdl_const(1), cdl_op(op)], expected)
    for op, expected in ((CDL_LT, 0), (CDL_LE, 1), (CDL_GT, 0), (CDL_GE, 1), (CDL_EQ, 1)):
        ops += check([cdl_const(4), cdl_const(4), cdl_op(op)], expected)
    ops += check([cdl_const(0), cdl_op(CDL_NOT)], 1)
    ops += check([cdl_const(5), cdl_op(CDL_NOT)], 0)
    for guid in (DLSID_GM, DLSID_GS, DLSID_XG, DLSID_DLS1, DLSID_DLS2, DLSID_MEMORY, DLSID_MANUFACTURER,
                 DLSID_PRODUCT, DLSID_RATE):
        ops += check([cdl_supported(guid)], 1)
    # (FluidSynth's QUERY pushes the "supported" flag over the value)
    ops += check([cdl_query(DLSID_GM), cdl_op(CDL_EQ)], 1)
    ops += check([cdl_query(DLSID_MANUFACTURER), cdl_op(CDL_SUBTRACT)], 1 - 0x4000)
    ops += check([cdl_query(DLSID_MEMORY), cdl_op(CDL_SUBTRACT)], 1 - 0xFFFFFFFF // 4 * 3)
    write('dls_cdl_ops.dls', cdl(*ops, **nolins))
    write('dls_cdl_false.dls', cdl(cdl_const(0)))
    # 44100 - the rate: false at 44.1 kHz (see dls_cdl_ops.dls for the QUERY)
    write('dls_cdl_rate.dls', cdl(cdl_query(DLSID_RATE), cdl_op(CDL_SUBTRACT), cdl_const(44099), cdl_op(CDL_ADD),
                                  **nolins))
    write('dls_cdl_overflow.dls', cdl(*[cdl_const(1)] * 65))
    write('dls_cdl_empty.dls', cdl())
    write('dls_cdl_div0.dls', cdl(cdl_const(0), cdl_const(5), cdl_op(CDL_DIVIDE)))
    write('dls_cdl_opcode.dls', cdl(cdl_const(1), cdl_op(0x13)))
    write('dls_cdl_query.dls', cdl(cdl_const(1), cdl_query(DLSID_UNKNOWN)))
    write('dls_cdl_supported.dls', cdl(cdl_supported(DLSID_UNKNOWN)))
    write('dls_cdl_end.dls', cdl(cdl_op(0x10) + b'\0\0'))

    # waves
    def broken(index, chunks):
        b = make_dls()
        b.waves[index] = dls_list(b'wave', chunks)
        return b.build()

    values = dls_values('sine', 16, 8, 50)
    write('dls_wave_format.dls', broken(1, [dls_fmt(16, 22050, 1, 2), dls_data(values)]))
    write('dls_wave_channels.dls', broken(1, [dls_fmt(16, 22050, 2), dls_data(values)]))
    write('dls_wave_bits.dls', broken(1, [dls_fmt(24, 22050), dls_data(values)]))
    write('dls_wave_order.dls', broken(1, [dls_data(values), dls_fmt(16, 22050)]))
    write('dls_wave_nodata.dls', broken(1, [dls_fmt(16, 22050), dls_info(b'No data')]))
    write('dls_wave_align.dls', broken(1, [dls_fmt(16, 22050), dls_ck(b'data', b'\1' * 33)]))
    write('dls_wave_short.dls', broken(3, [dls_fmt(16, 22050), dls_data(values, size=5000)]))
    write('dls_wave_loops.dls', broken(1, [dls_fmt(16, 22050), dls_wsmp(60, loop=(0, 0, 8), loops=2),
                                           dls_data(values)]))
    write('dls_wave_wsmp.dls', broken(1, [dls_fmt(16, 22050), dls_wsmp(60, loop=(0, 0, 8), size=40),
                                          dls_data(values)]))
    cues = d.cues()
    # (a cue at the first wave's wsmp chunk, after its fmt chunk; one at
    # the second wave's INFO list; one past the end)
    write('dls_wave_notlist.dls', d.build(cues=[cues[0], cues[0] + 12 + 24] + cues[2:]))
    write('dls_wave_list.dls', d.build(cues=[cues[0], cues[1] + 12] + cues[2:]))
    write('dls_wave_cue.dls', d.build(cues=[cues[0], 0x10000] + cues[2:]))


# Articulation: an instrument's or a region's 'LIST' 'lart' (DLS Level 1,
# of 'art1' chunks) or 'LIST' 'lar2' (Level 2, of 'art2' chunks) holds
# connection blocks, each a source, a control (a second source scaling the
# first), a destination, a transform and a 16.16 fixed-point scale (in the
# destination's unit: cents, time cents, centibels or tenths of a percent).
# Level 1's transform is one 4-bit curve for both sources; Level 2's has a
# curve, polarity and direction for each, and an output curve.

# connection block sources
SRC_NONE, SRC_LFO, SRC_VELOCITY, SRC_KEY, SRC_EG1, SRC_EG2, SRC_BEND = 0, 1, 2, 3, 4, 5, 6
SRC_POLYPRESSURE, SRC_CHANPRESSURE, SRC_VIBRATO = 7, 8, 9
SRC_CC1, SRC_CC7, SRC_CC10, SRC_CC11, SRC_CC91, SRC_CC93 = 0x81, 0x87, 0x8A, 0x8B, 0xDB, 0xDD
SRC_RPN0, SRC_RPN1, SRC_RPN2 = 0x100, 0x101, 0x102
# destinations
DST_NONE, DST_GAIN, DST_RESERVED, DST_PITCH, DST_PAN, DST_KEYNUMBER = 0, 1, 2, 3, 4, 5
DST_LEFT, DST_CHORUS, DST_REVERB = 0x10, 0x80, 0x81
DST_LFO_FREQ, DST_LFO_DELAY, DST_VIB_FREQ, DST_VIB_DELAY = 0x104, 0x105, 0x114, 0x115
DST_EG1_ATTACK, DST_EG1_DECAY, DST_EG1_RESERVED, DST_EG1_RELEASE = 0x206, 0x207, 0x208, 0x209
DST_EG1_SUSTAIN, DST_EG1_DELAY, DST_EG1_HOLD, DST_EG1_SHUTDOWN = 0x20A, 0x20B, 0x20C, 0x20D
DST_EG2_ATTACK, DST_EG2_DECAY, DST_EG2_RESERVED, DST_EG2_RELEASE = 0x30A, 0x30B, 0x30C, 0x30D
DST_EG2_SUSTAIN, DST_EG2_DELAY, DST_EG2_HOLD = 0x30E, 0x30F, 0x310
DST_FILTER_CUTOFF, DST_FILTER_Q = 0x500, 0x501
# curves
TRN_NONE, TRN_CONCAVE, TRN_CONVEX, TRN_SWITCH = 0, 1, 2, 3


def trn(src=0, ctl=0, src_bip=False, src_inv=False, ctl_bip=False, ctl_inv=False, out=0):
    """A Level 2 transform: each source's curve, polarity and direction,
    and the output's curve."""
    return (src_inv << 15) | (src_bip << 14) | (src << 10) | (ctl_inv << 9) | (ctl_bip << 8) | (ctl << 4) | out


def conn(source, control, dest, transform, scale):
    """A connection block; the scale is a whole number of units."""
    return struct.pack('<HHHHi', source, control, dest, transform, scale * 65536)


def dls_art(blocks, level1=False, cbsize=8, count=None, size=None, as_list=False):
    """An 'art2' (or Level 1 'art1') chunk; cbsize, count and size override
    the structure's own (the corrupt ones); as_list writes it as a 'LIST'
    (which isn't standard)."""
    n = len(blocks) if count is None else count
    data = struct.pack('<II', cbsize, n) + b'\0' * max(cbsize - 8, 0) + b''.join(blocks)
    cid = b'art1' if level1 else b'art2'
    return dls_list(cid, [data]) if as_list else dls_ck(cid, data, size)


def dls_lart(chunks, level1=False):
    return dls_list(b'lart' if level1 else b'lar2', chunks)


def dls_cdl(*ops):
    return dls_ck(b'cdl ', b''.join(ops))


def dls_rgn(keys, wave, vels=(0, 127), wsmp=None, group=0, options=1, rgn2=False, chunks=(), before=(),
            wlnk=True):
    """A region: the rgnh (key and velocity ranges, options, key group), a
    wsmp chunk if given, other chunks, then the wave link (unless not
    wlnk); before goes first."""
    body = list(before) + [dls_ck(b'rgnh', struct.pack('<HHHHHH', keys[0], keys[1], vels[0], vels[1], options,
                                                         group))]
    if wsmp:
        body.append(wsmp)
    body += list(chunks)
    if wlnk:
        body.append(dls_ck(b'wlnk', struct.pack('<HHII', 0, 0, 1, wave)))
    return dls_list(b'rgn2' if rgn2 else b'rgn ', body)


def dls_instrument(name, bank, prog, rgns, drums=False, larts=(), chunks=(), cregions=None, insh_size=None):
    """An instrument of the given regions (dls_rgn()), articulation lists
    and other chunks, its insh counting cregions regions if given."""
    n = len(rgns) if cregions is None else cregions
    insh = struct.pack('<III', n, bank | (0x80000000 if drums else 0), prog)
    if insh_size is not None:
        insh = insh[:insh_size]
    return dls_list(b'ins ', [dls_ck(b'insh', insh)] + list(chunks) + list(larts) +
                    [dls_list(b'lrgn', rgns), dls_info(name)])


def dls_pgal(table, aliases, version=1, count=None):
    """MobileBAE's 'pgal' chunk: a version (unless None), the drum note
    aliasing table (128 notes), then the instrument aliases, each (bank,
    program) of the alias and of the instrument, a bank being MSB << 7 |
    LSB."""
    data = (b'' if version is None else struct.pack('<I', version)) + bytes(table)
    data += struct.pack('<I', len(aliases) if count is None else count)
    for (bank2, pc2), (bank, pc) in aliases:
        data += struct.pack('<HBBHBB', bank2, pc2, 0, bank, pc, 0)
    return dls_ck(b'pgal', data)


def make_test_dls():
    """test.dls: make_dls()'s waves and instruments, and more (no two of
    them at one bank and program), in no order: an articulation at each
    level, both DLS levels, velocity layers, key groups, conditional
    regions, the Key Number generator, and the banks of each bank select
    style."""
    d = make_dls()
    sine, square, saw = 0, 1, 2
    long_sine = d.wave([dls_fmt(16, 32000), dls_wsmp(69, -7, -65536 * 15, (0, 32, 96)),
                        dls_data(dls_values('sine', 128, 32, 80)), dls_info(b'Long sine')])
    strings = dls_lart([dls_art([
        conn(SRC_NONE, SRC_NONE, DST_EG1_ATTACK, 0, -3986), conn(SRC_NONE, SRC_NONE, DST_EG1_DECAY, 0, -2000),
        conn(SRC_NONE, SRC_NONE, DST_EG1_SUSTAIN, 0, 700), conn(SRC_NONE, SRC_NONE, DST_EG1_RELEASE, 0, -3000),
        conn(SRC_NONE, SRC_NONE, DST_EG1_DELAY, 0, -12000), conn(SRC_NONE, SRC_NONE, DST_EG1_HOLD, 0, -10000),
        conn(SRC_NONE, SRC_NONE, DST_LFO_FREQ, 0, -500), conn(SRC_NONE, SRC_NONE, DST_LFO_DELAY, 0, -9000),
        conn(SRC_NONE, SRC_NONE, DST_VIB_FREQ, 0, 300), conn(SRC_NONE, SRC_NONE, DST_VIB_DELAY, 0, -8000),
        conn(SRC_LFO, SRC_NONE, DST_PITCH, trn(src_bip=True), 20),
        conn(SRC_VIBRATO, SRC_CC1, DST_PITCH, trn(src_bip=True), 50),
        conn(SRC_LFO, SRC_CHANPRESSURE, DST_PITCH, trn(src_bip=True), 30),
        conn(SRC_LFO, SRC_NONE, DST_FILTER_CUTOFF, trn(src_bip=True), 200),
        conn(SRC_LFO, SRC_NONE, DST_GAIN, trn(src_bip=True), 10),
        conn(SRC_NONE, SRC_NONE, DST_EG2_ATTACK, 0, -6000), conn(SRC_NONE, SRC_NONE, DST_EG2_DECAY, 0, -1000),
        conn(SRC_NONE, SRC_NONE, DST_EG2_SUSTAIN, 0, 800), conn(SRC_NONE, SRC_NONE, DST_EG2_RELEASE, 0, -2500),
        conn(SRC_NONE, SRC_NONE, DST_EG2_DELAY, 0, -11000), conn(SRC_NONE, SRC_NONE, DST_EG2_HOLD, 0, -9000),
        conn(SRC_EG2, SRC_NONE, DST_PITCH, 0, 100), conn(SRC_EG2, SRC_NONE, DST_FILTER_CUTOFF, 0, 1200),
        conn(SRC_NONE, SRC_NONE, DST_FILTER_CUTOFF, 0, 9000), conn(SRC_NONE, SRC_NONE, DST_FILTER_Q, 0, 30),
        conn(SRC_NONE, SRC_NONE, DST_PAN, 0, -200), conn(SRC_NONE, SRC_NONE, DST_CHORUS, 0, 300),
        conn(SRC_NONE, SRC_NONE, DST_REVERB, 0, 400),
        conn(SRC_VELOCITY, SRC_NONE, DST_GAIN, trn(TRN_CONCAVE, src_inv=True), -960),
        conn(SRC_KEY, SRC_NONE, DST_PITCH, 0, 10000),
        conn(SRC_CC7, SRC_NONE, DST_GAIN, trn(TRN_CONCAVE, src_inv=True), -960),
        conn(SRC_CC10, SRC_NONE, DST_PAN, trn(src_bip=True), 500),
        conn(SRC_CC11, SRC_NONE, DST_GAIN, trn(TRN_CONCAVE, src_inv=True), -960),
        conn(SRC_CC91, SRC_NONE, DST_REVERB, 0, 1000), conn(SRC_CC93, SRC_NONE, DST_CHORUS, 0, 500),
        conn(SRC_BEND, SRC_RPN0, DST_PITCH, trn(src_bip=True), 12800),
        conn(SRC_POLYPRESSURE, SRC_NONE, DST_GAIN, 0, -100),
        conn(SRC_KEY, SRC_NONE, DST_EG1_DECAY, 0, -100),
        conn(SRC_VELOCITY, SRC_NONE, DST_FILTER_CUTOFF, trn(TRN_CONVEX), 2400),
        conn(SRC_CC1, SRC_VELOCITY, DST_PAN, trn(TRN_SWITCH, TRN_CONCAVE, ctl_inv=True), 300),
    ])])
    # (a region articulation replaces the instrument's)
    release = dls_lart([dls_art([conn(SRC_NONE, SRC_NONE, DST_EG1_RELEASE, 0, -1000),
                                 conn(SRC_CC1, SRC_NONE, DST_PITCH, 0, 25)])])
    d.instruments.append(dls_instrument(b'Strings', 1 << 8 | 2, 48, [
        dls_rgn((0, 127), long_sine, (0, 63)),
        dls_rgn((0, 127), saw, (64, 127), dls_wsmp(64, 20, -65536 * 30, (0, 8, 24)), rgn2=True,
                chunks=[release]),
    ], larts=[strings]))
    hats = [dls_rgn((36, 36), square), dls_rgn((38, 38), square, group=0),
            dls_rgn((42, 42), sine, wsmp=dls_wsmp(42), group=1),
            dls_rgn((44, 44), sine, wsmp=dls_wsmp(44, 30), group=1, options=0),
            dls_rgn((46, 46), long_sine, wsmp=dls_wsmp(46, 0, 0, (1, 32, 96)), group=1)]
    d.instruments.append(dls_instrument(b'Kit 2', 0, 25, hats, drums=True))
    d.instruments.append(dls_instrument(b'Bright GM2', 121 << 8, 1, [dls_rgn((0, 127), sine)]))
    # DLS Level 1 (art1), whose transform the loader makes a Level 2 one
    organ = dls_lart([dls_art([
        conn(SRC_NONE, SRC_NONE, DST_EG1_ATTACK, 0, -6000), conn(SRC_NONE, SRC_NONE, DST_EG1_RELEASE, 0, -2000),
        conn(SRC_NONE, SRC_NONE, DST_EG1_SUSTAIN, 0, 1000),
        conn(SRC_VELOCITY, SRC_NONE, DST_GAIN, TRN_CONCAVE, -960),
        conn(SRC_CC7, SRC_NONE, DST_GAIN, TRN_CONCAVE, -960), conn(SRC_CC11, SRC_NONE, DST_GAIN, TRN_CONCAVE, -960),
        conn(SRC_LFO, SRC_NONE, DST_PITCH, 0, 15), conn(SRC_LFO, SRC_CC1, DST_PITCH, 0, 50),
        conn(SRC_BEND, SRC_RPN0, DST_PITCH, 0, 12800), conn(SRC_CC10, SRC_NONE, DST_PAN, 0, 500),
    ], level1=True)], level1=True)
    d.instruments.append(dls_instrument(b'Organ', 0, 16, [
        dls_rgn((0, 59), long_sine, chunks=[dls_cdl(cdl_supported(DLSID_GM))]),
        dls_rgn((60, 127), sine, wsmp=dls_wsmp(72, -12, 65536 * 20, (1, 8, 40)), chunks=[dls_cdl(cdl_const(1))]),
        dls_rgn((0, 127), saw, chunks=[dls_cdl(cdl_const(0))]),        # (its condition is false)
    ], larts=[organ]))
    d.instruments.append(dls_instrument(b'GM2 Rhythm', 120 << 8, 0, [dls_rgn((0, 127), square)]))
    d.instruments.append(dls_instrument(b'XG voice', 3, 4, [dls_rgn((0, 127), square, wsmp=dls_wsmp(60))]))
    # the Key Number generator (x 0.5): FluidSynth takes the instrument's
    # from a second articulation list only
    keynum = dls_lart([dls_art([conn(SRC_KEY, SRC_NONE, DST_KEYNUMBER, 0, 6400),
                                conn(SRC_NONE, SRC_NONE, DST_EG1_RELEASE, 0, -4000)])])
    split = [dls_rgn((0, 40), saw), dls_rgn((41, 127), long_sine)]
    d.instruments.append(dls_instrument(b'Keynum two', 0, 40, split, larts=[
        keynum, dls_lart([dls_art([conn(SRC_NONE, SRC_NONE, DST_PAN, 0, 300)])])]))
    d.instruments.append(dls_instrument(b'Keynum one', 0, 41, split, larts=[keynum]))
    d.instruments.append(dls_instrument(b'Kit 3', 1 << 8, 2, [dls_rgn((30, 60), sine, wsmp=dls_wsmp(48))],
                                        drums=True))
    # (the drum channel's program 0; LSB 1, as Piano is at bank 0 program 0)
    d.instruments.append(dls_instrument(b'Standard', 1, 0, [dls_rgn((35, 59), square), dls_rgn((60, 81), sine)],
                                        drums=True))
    return d


def dls_instrument_files():
    d = make_test_dls()
    write('test.dls', d.build())
    long_sine = len(d.waves) - 1

    # instruments that fail the file
    def one(*args, **kwargs):
        b = make_dls()
        b.instruments = [dls_instrument(*args, **kwargs)]
        return b

    rgn = [dls_rgn((0, 127), 0)]
    write('dls_insh.dls', one(b'Short insh', 0, 0, rgn, insh_size=8).build())
    write('dls_wlnk.dls', one(b'Bad link', 0, 0, [dls_rgn((0, 127), 9)]).build())
    # (no samples, and a region without a wave link)
    write('dls_at.dls', one(b'No link', 0, 0, [dls_rgn((0, 127), 0, wlnk=False)]).build(cues=[]))
    false = dls_cdl(cdl_const(0))
    art = dls_art([conn(SRC_NONE, SRC_NONE, DST_PAN, 0, 100)])
    # (FluidSynth fails the file where a false condition should bypass the articulation)
    write('dls_lart_cdl.dls', one(b'False lart', 0, 0, rgn, larts=[dls_lart([false, art])]).build())
    write('dls_rgn_lartcdl.dls', one(b'False region lart', 0, 0, [
        dls_rgn((0, 127), 0, chunks=[dls_lart([false, art])])]).build())
    write('dls_art_cbsize.dls', one(b'Art cbSize', 0, 0, rgn, larts=[dls_lart([dls_art([], cbsize=4)])]).build())
    write('dls_art_size.dls', one(b'Art size', 0, 0, rgn, larts=[
        dls_lart([dls_art([conn(SRC_NONE, SRC_NONE, DST_PAN, 0, 1)], size=12)])]).build())
    write('dls_art_count.dls', one(b'Art count', 0, 0, rgn, larts=[
        dls_lart([dls_art([], count=0x20000000)])]).build())
    write('dls_lart_notlist.dls', one(b'Plain lart', 0, 0, rgn, chunks=[dls_ck(b'lart', b'')]).build())

    # warnings: every connection block the loader ignores or changes, and
    # unknown and ignored chunks at each level
    b = make_dls()
    odd = dls_lart([dls_art([
        conn(SRC_RPN1, SRC_NONE, DST_PITCH, 0, 100), conn(SRC_RPN2, SRC_NONE, DST_PITCH, 0, 100),
        conn(SRC_EG1, SRC_NONE, DST_FILTER_CUTOFF, 0, 100),
        conn(SRC_LFO, SRC_NONE, DST_PITCH, trn(TRN_CONVEX, src_inv=True), 10),       # unipolar, transformed
        conn(SRC_LFO, SRC_NONE, DST_PAN, trn(src_bip=True), 10),                     # not a generator
        conn(SRC_VIBRATO, SRC_NONE, DST_PITCH, trn(TRN_CONCAVE), 10),
        conn(SRC_VIBRATO, SRC_NONE, DST_GAIN, trn(src_bip=True), 10),
        conn(SRC_EG2, SRC_NONE, DST_PITCH, trn(src_bip=True), 10), conn(SRC_EG2, SRC_NONE, DST_PAN, 0, 10),
        conn(SRC_CC7, SRC_NONE, DST_GAIN, trn(out=TRN_CONCAVE), -100),               # output transform moved
        conn(SRC_CC11, SRC_CC1, DST_GAIN, trn(out=TRN_CONVEX), -100),                # not moved
        conn(0x50, SRC_NONE, DST_PITCH, 0, 10), conn(SRC_CC1, 0x83, DST_PITCH, 0, 10),
        conn(SRC_CC1, SRC_NONE, DST_PITCH, trn(5), 10), conn(SRC_CC1, SRC_CC10, DST_PITCH, trn(ctl=6), 10),
        conn(SRC_NONE, SRC_CC1, DST_FILTER_Q, trn(ctl=TRN_CONVEX, ctl_bip=True), 20),   # control as source
        conn(SRC_CC1, SRC_NONE, DST_LEFT, 0, 10), conn(SRC_NONE, SRC_NONE, DST_EG1_SHUTDOWN, 0, 10),
        conn(SRC_NONE, SRC_NONE, DST_RESERVED, 0, 10), conn(SRC_NONE, SRC_NONE, DST_EG2_RESERVED, 0, 10),
        conn(SRC_NONE, SRC_NONE, 0x1234, 0, 10), conn(SRC_NONE, SRC_NONE, DST_NONE, 0, 10),
        conn(SRC_KEY, SRC_NONE, DST_KEYNUMBER, trn(TRN_CONCAVE), 6400),            # not linear: a modulator
        conn(SRC_CC1, SRC_NONE, DST_PITCH, 0, 10), conn(SRC_CC1, SRC_NONE, DST_PITCH, 0, 20),   # replaced
        conn(SRC_VELOCITY, SRC_NONE, DST_GAIN, trn(TRN_SWITCH), -100),             # made negative
        conn(SRC_NONE, SRC_NONE, DST_EG1_SUSTAIN, 0, 300), conn(SRC_NONE, SRC_NONE, DST_EG1_SUSTAIN, 0, 300),
    ]), dls_ck(b'abcd', b'xy'), dls_list(b'zzzz', [])])
    level1 = dls_lart([dls_art([conn(SRC_CC1, SRC_LFO, DST_PITCH, 0, 10),
                                conn(SRC_VIBRATO, SRC_NONE, DST_PITCH, 0, 10)], level1=True, as_list=True)],
                      level1=True)
    keynum = dls_lart([dls_art([conn(SRC_KEY, SRC_NONE, DST_KEYNUMBER, 0, 6400)])])
    crs1 = dls_ck(b'crs1', b'\0' * 28, size=0)                     # (its size is taken to be 28)
    b.instruments.append(dls_instrument(b'Odd', 0, 1, [
        dls_rgn((0, 59), 0, chunks=[keynum, dls_lart([dls_art([conn(SRC_NONE, SRC_NONE, DST_PAN, 0, 10)])]),
                                    dls_ck(b'abcd', b''), crs1, dls_ck(b'crs2', b''), dls_info(b'rgn info')],
                before=[dls_ck(b'wsmp', b'\0' * 20)]),
        dls_rgn((60, 127), 2, wsmp=dls_wsmp(60, 0, 0, (5, 4, 30))),     # unknown loop type
        dls_ck(b'abcd', b'1'), crs1, dls_ck(b'crs2', b''),
        dls_list(b'rgn ', [dls_ck(b'wlnk', struct.pack('<HHII', 0, 0, 1, 1))]),   # no rgnh
    ], larts=[odd, level1], chunks=[dls_ck(b'dlid', DLSID_UNKNOWN), crs1, dls_ck(b'crs2', b''),
                                    dls_ck(b'abcd', b'')]))
    b.instruments += [crs1, dls_ck(b'crs2', b''), dls_ck(b'zyxw', b'12')]
    write('dls_art.dls', b.build())

    # samples: one without data (which fails validation), one whose loop
    # is out of the data, one without a name, one with a long one
    b = Dls()
    b.wave([dls_fmt(16, 22050), dls_data([]), dls_info(b'Empty \xe9chantillon with a long name')])
    b.wave([dls_fmt(16, 22050), dls_wsmp(60, 0, 0, (0, 10, 100000)), dls_data(dls_values('saw', 32, 16, 50)),
            dls_info(b'Bad loop')])
    b.wave([dls_fmt(8, 22050), dls_wsmp(67, 0, 0, (1, 0, 0)), dls_data(dls_values('square', 32, 8, 50), 8)])
    b.wave([dls_fmt(16, 44100), dls_wsmp(60, -50, 0, (0, 30, 2)), dls_data(dls_values('sine', 40, 10, 60)),
            dls_info(b'A sample with a name of more than twenty characters')])
    b.instruments.append(dls_instrument(b'Samples', 0, 0, [
        dls_rgn((0, 61), 0), dls_rgn((62, 65), 1), dls_rgn((66, 69), 2), dls_rgn((70, 127), 3)]))
    write('dls_samples.dls', b.build())

    # more than 16 instruments (std::sort's insertion sort's limit), in
    # no order, no two at one bank and program
    b = make_dls()
    b.instruments = []
    for i in range(20):
        prog_ = (i * 7) % 20
        bank = ((i * 3) % 4) << 8 | (i % 3)
        b.instruments.append(dls_instrument(b'I%02d' % i, bank, prog_, [dls_rgn((0, 127), i % 3)],
                                            drums=i % 5 == 4))
    write('dls_sort.dls', b.build())

    # MobileBAE's instrument aliasing (pgal): the drum note table and
    # aliases of melodic instruments, two of them missing
    table = list(range(128))
    table[36], table[82], table[100] = 40, 50, 0
    aliases = [((0, 3), (0, 0)), ((1 << 7 | 5, 81), (0, 80)), ((0, 9), (0, 8)), ((0, 10), (2 << 7 | 1, 7))]
    write('dls_pgal.dls', make_dls().build(after=[dls_pgal(table, aliases)]))
    write('dls_pgal_nover.dls', make_dls().build(before=[dls_pgal(range(128), aliases[:1], None)]))
    write('dls_pgal_small.dls', make_dls().build(after=[dls_ck(b'pgal', b'\0' * 100)]))
    # (the aliases go on past the end of the file)
    write('dls_pgal_short.dls', make_dls().build(after=[dls_pgal(table, aliases[:1], count=3)]))
    return long_sine


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
    # DLS files, through the synth's DLS loader reading from memory
    ('dls_basic', 'dls:test.dls', 'dls_basic.mid', 44100, 400000, ''),
    ('dls_basic48k', 'dls:test.dls', 'dls_basic.mid', 48000, 400000, ''),
    ('dls_basic22k', 'dls:test.dls', 'dls_basic.mid', 22050, 200000, ''),
    ('dls_basic8k', 'dls:test.dls', 'dls_basic.mid', 8000, 100000, ''),
    ('dls_basic_dry', 'dls:test.dls', 'dls_basic.mid', 44100, 400000,
     'synth.reverb.active=0 synth.chorus.active=0'),
    ('dls_banks_gs', 'dls:test.dls', 'dls_banks.mid', 44100, 400000, 'synth.midi-bank-select=gs'),
    ('dls_banks_gm', 'dls:test.dls', 'dls_banks.mid', 44100, 400000, 'synth.midi-bank-select=gm'),
    ('dls_banks_xg', 'dls:test.dls', 'dls_banks.mid', 44100, 400000, 'synth.midi-bank-select=xg'),
    ('dls_banks_mma', 'dls:test.dls', 'dls_banks.mid', 44100, 400000, 'synth.midi-bank-select=mma'),
    ('dls_mono', 'dls:test.dls', 'dls_mono.mid', 44100, 400000, ''),
    ('dls_mono_xg', 'dls:test.dls', 'dls_mono.mid', 44100, 400000, 'synth.portamento-time=xg-gs'),
    ('dls_many', 'dls:test.dls', 'dls_many.mid', 44100, 400000, ''),
    ('dls_many_poly8', 'dls:test.dls', 'dls_many.mid', 44100, 400000, 'synth.polyphony=8'),
    ('dls_tuning', 'dls:test.dls', 'dls_tuning.mid', 44100, 400000, ''),
    ('dls_sysex', 'dls:test.dls', 'sysex.mid', 44100, 400000, ''),
    ('dls_nrpn', 'dls:test.dls', 'nrpn.mid', 44100, 400000, ''),
    ('dls_pgal', 'dls:dls_pgal.dls', 'dls_drums.mid', 44100, 400000, ''),
    ('dls_art', 'dls:dls_art.dls', 'dls_basic.mid', 44100, 400000, ''),
    ('dls_samples', 'dls:dls_samples.dls', 'dls_basic.mid', 44100, 400000, ''),
    ('dls_sort', 'dls:dls_sort.dls', 'dls_banks.mid', 44100, 400000, 'synth.midi-bank-select=mma'),
    ('dls_fail', 'dls:dls_lart_cdl.dls', 'dls_basic.mid', 44100, 300000, ''),
    # test.dls by its path, through the synth's own loaders (the SoundFont
    # loader's, then the DLS loader's)
    ('dls_path', 'path:test.dls', 'dls_basic.mid', 44100, 400000, ''),
]


def cases():
    lines = ['# name soundfont midi rate max-frames settings...\n']
    for name, sf, mid, rate, frames, settings in CASES:
        lines.append(' '.join(x for x in [name, sf, 'midi/' + mid, str(rate), str(frames), settings] if x) + '\n')
    write('cases.txt', ''.join(lines))


def main():
    soundfonts()
    dls_files()
    dls_instrument_files()
    midi_files()
    cases()


if __name__ == '__main__':
    main()
