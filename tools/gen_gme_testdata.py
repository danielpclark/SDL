#!/usr/bin/env python3
"""Generate sdl3-mixer-gme's test data (sdl3-mixer-gme/testdata/).

Every file here is synthetic, made by this script: tiny game music rips
whose sound programs are assembled here (for the 6502 of the NSF, NSFE
and SAP files, the Game Boy CPU of the GBS files, the Z80 of the AY and
KSS files, the HuC6280 of the HES files, the SPC-700 of the SPC files)
or written as command streams (the VGM files), driving the sound chips
the emulators emulate, with their metadata, track lists and timing.
Nothing comes from a real game. (An SPC file is always a 64K memory
image, so those two are not tiny.)

The output is deterministic, so regenerating the files gives the same
bytes. If they change, the reference output (`testdata/reference.txt`,
and sdl3-mixer's `src/testdata/reference_gme.txt`), made by a C++ program
built from the libgme that SDL_mixer vendors, must be regenerated too.

usage: tools/gen_gme_testdata.py [OUTDIR]
"""

import os
import struct
import sys

OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
    os.path.dirname(os.path.abspath(__file__)), '..', 'sdl3-mixer-gme', 'testdata')


def write(name, data):
    os.makedirs(OUT, exist_ok=True)
    with open(os.path.join(OUT, name), 'wb') as f:
        f.write(bytes(data))


# ---------------------------------------------------------------------------
# A small two-pass assembler for the 6502 (with the unofficial opcodes the
# NES CPU emulator handles specially).

OPS_6502 = {
    'ADC': {'imm': 0x69, 'zp': 0x65, 'zpx': 0x75, 'abs': 0x6D, 'absx': 0x7D, 'absy': 0x79, 'indx': 0x61, 'indy': 0x71},
    'AND': {'imm': 0x29, 'zp': 0x25, 'zpx': 0x35, 'abs': 0x2D, 'absx': 0x3D, 'absy': 0x39, 'indx': 0x21, 'indy': 0x31},
    'ASL': {'acc': 0x0A, 'zp': 0x06, 'zpx': 0x16, 'abs': 0x0E, 'absx': 0x1E},
    'BCC': {'rel': 0x90}, 'BCS': {'rel': 0xB0}, 'BEQ': {'rel': 0xF0}, 'BMI': {'rel': 0x30},
    'BNE': {'rel': 0xD0}, 'BPL': {'rel': 0x10}, 'BVC': {'rel': 0x50}, 'BVS': {'rel': 0x70},
    'BIT': {'zp': 0x24, 'abs': 0x2C},
    'BRK': {'imp': 0x00},
    'CLC': {'imp': 0x18}, 'CLD': {'imp': 0xD8}, 'CLI': {'imp': 0x58}, 'CLV': {'imp': 0xB8},
    'CMP': {'imm': 0xC9, 'zp': 0xC5, 'zpx': 0xD5, 'abs': 0xCD, 'absx': 0xDD, 'absy': 0xD9, 'indx': 0xC1, 'indy': 0xD1},
    'CPX': {'imm': 0xE0, 'zp': 0xE4, 'abs': 0xEC},
    'CPY': {'imm': 0xC0, 'zp': 0xC4, 'abs': 0xCC},
    'DEC': {'zp': 0xC6, 'zpx': 0xD6, 'abs': 0xCE, 'absx': 0xDE},
    'DEX': {'imp': 0xCA}, 'DEY': {'imp': 0x88},
    'EOR': {'imm': 0x49, 'zp': 0x45, 'zpx': 0x55, 'abs': 0x4D, 'absx': 0x5D, 'absy': 0x59, 'indx': 0x41, 'indy': 0x51},
    'INC': {'zp': 0xE6, 'zpx': 0xF6, 'abs': 0xEE, 'absx': 0xFE},
    'INX': {'imp': 0xE8}, 'INY': {'imp': 0xC8},
    'JMP': {'abs': 0x4C, 'ind': 0x6C},
    'JSR': {'abs': 0x20},
    'LDA': {'imm': 0xA9, 'zp': 0xA5, 'zpx': 0xB5, 'abs': 0xAD, 'absx': 0xBD, 'absy': 0xB9, 'indx': 0xA1, 'indy': 0xB1},
    'LDX': {'imm': 0xA2, 'zp': 0xA6, 'zpy': 0xB6, 'abs': 0xAE, 'absy': 0xBE},
    'LDY': {'imm': 0xA0, 'zp': 0xA4, 'zpx': 0xB4, 'abs': 0xAC, 'absx': 0xBC},
    'LSR': {'acc': 0x4A, 'zp': 0x46, 'zpx': 0x56, 'abs': 0x4E, 'absx': 0x5E},
    'NOP': {'imp': 0xEA},
    'ORA': {'imm': 0x09, 'zp': 0x05, 'zpx': 0x15, 'abs': 0x0D, 'absx': 0x1D, 'absy': 0x19, 'indx': 0x01, 'indy': 0x11},
    'PHA': {'imp': 0x48}, 'PHP': {'imp': 0x08}, 'PLA': {'imp': 0x68}, 'PLP': {'imp': 0x28},
    'ROL': {'acc': 0x2A, 'zp': 0x26, 'zpx': 0x36, 'abs': 0x2E, 'absx': 0x3E},
    'ROR': {'acc': 0x6A, 'zp': 0x66, 'zpx': 0x76, 'abs': 0x6E, 'absx': 0x7E},
    'RTI': {'imp': 0x40}, 'RTS': {'imp': 0x60},
    'SBC': {'imm': 0xE9, 'zp': 0xE5, 'zpx': 0xF5, 'abs': 0xED, 'absx': 0xFD, 'absy': 0xF9, 'indx': 0xE1, 'indy': 0xF1},
    'SEC': {'imp': 0x38}, 'SED': {'imp': 0xF8}, 'SEI': {'imp': 0x78},
    'STA': {'zp': 0x85, 'zpx': 0x95, 'abs': 0x8D, 'absx': 0x9D, 'absy': 0x99, 'indx': 0x81, 'indy': 0x91},
    'STX': {'zp': 0x86, 'zpy': 0x96, 'abs': 0x8E},
    'STY': {'zp': 0x84, 'zpx': 0x94, 'abs': 0x8C},
    'TAX': {'imp': 0xAA}, 'TAY': {'imp': 0xA8}, 'TSX': {'imp': 0xBA}, 'TXA': {'imp': 0x8A},
    'TXS': {'imp': 0x9A}, 'TYA': {'imp': 0x98},
    # unofficial
    'LAX': {'indy': 0xB3}, 'SAX': {'abs': 0x8F}, 'SBX': {'imm': 0xCB},
}

SIZES = {'imp': 1, 'acc': 1, 'imm': 2, 'zp': 2, 'zpx': 2, 'zpy': 2, 'abs': 3, 'absx': 3, 'absy': 3,
         'ind': 3, 'indx': 2, 'indy': 2, 'rel': 2}


class Asm:
    """Assembles 6502 source text: one instruction, label (`name:`) or
    directive (`.db`, `.dw`, `.org`) per line, `;` comments. Numbers are
    `$hex`, `%bin` or decimal; `<name` and `>name` are a label's low and
    high bytes."""

    def __init__(self, org, ops=OPS_6502):
        self.org = org
        self.ops = ops
        self.src = []

    def __call__(self, text):
        self.src.extend(text.split('\n'))
        return self

    def value(self, s, labels, final):
        s = s.strip()
        if s.startswith('<'):
            return self.value(s[1:], labels, final) & 0xFF
        if s.startswith('>'):
            return (self.value(s[1:], labels, final) >> 8) & 0xFF
        for op in ('+', '-'):
            # (left-to-right, no precedence)
            i = s.rfind(op)
            if i > 0:
                a = self.value(s[:i], labels, final)
                b = self.value(s[i + 1:], labels, final)
                return a + b if op == '+' else a - b
        if s.startswith('$'):
            return int(s[1:], 16)
        if s.startswith('%'):
            return int(s[1:], 2)
        if s[0].isdigit():
            return int(s)
        if s in labels:
            return labels[s]
        if final:
            raise ValueError('unknown label ' + s)
        return 0x1234

    def mode(self, mnem, arg, labels, final):
        modes = self.ops[mnem]
        if arg == '':
            return ('acc' if 'acc' in modes else 'imp'), None
        if arg == 'A' and 'acc' in modes:
            return 'acc', None
        if 'rel' in modes:
            return 'rel', self.value(arg, labels, final)
        if arg.startswith('#'):
            return 'imm', self.value(arg[1:], labels, final)
        if arg.startswith('(') and arg.upper().endswith(',X)'):
            return 'indx', self.value(arg[1:-3], labels, final)
        if arg.startswith('(') and arg.upper().endswith('),Y'):
            return 'indy', self.value(arg[1:-3], labels, final)
        if arg.startswith('('):
            return 'ind', self.value(arg[1:-1], labels, final)
        idx = ''
        if arg.upper().endswith(',X'):
            idx, arg = 'x', arg[:-2]
        elif arg.upper().endswith(',Y'):
            idx, arg = 'y', arg[:-2]
        v = self.value(arg, labels, final)
        is_zp = arg.startswith('$') and len(arg) <= 3 and ('zp' + idx if idx else 'zp') in modes
        return ('zp' if is_zp else 'abs') + idx, v

    def assemble(self):
        labels = {}
        for final in (False, True):
            out = bytearray()
            pc = self.org
            for line in self.src:
                line = line.split(';')[0].strip()
                if not line:
                    continue
                while ':' in line.split()[0] if line else False:
                    name, line = line.split(':', 1)
                    labels[name.strip()] = pc
                    line = line.strip()
                if not line:
                    continue
                parts = line.split(None, 1)
                mnem = parts[0].upper()
                arg = parts[1].strip() if len(parts) > 1 else ''
                if mnem == '.DB':
                    for v in arg.split(','):
                        out.append(self.value(v, labels, final) & 0xFF)
                    pc = self.org + len(out)
                    continue
                if mnem == '.DW':
                    for v in arg.split(','):
                        out += struct.pack('<H', self.value(v, labels, final) & 0xFFFF)
                    pc = self.org + len(out)
                    continue
                if mnem == '.ORG':
                    target = self.value(arg, labels, final)
                    assert target >= pc, (hex(target), hex(pc))
                    out += bytes(target - pc)
                    pc = target
                    continue
                out += self.encode(mnem, arg, labels, final, pc, line)
                pc = self.org + len(out)
        self.labels = labels
        return bytes(out)

    def encode(self, mnem, arg, labels, final, pc, line):
        """The bytes of one 6502 instruction."""
        out = bytearray()
        m, v = self.mode(mnem, arg, labels, final)
        opc = self.ops[mnem][m]
        out.append(opc)
        if m == 'rel':
            off = v - (pc + 2)
            if final:
                assert -128 <= off <= 127, (line, off)
            out.append(off & 0xFF)
        elif SIZES[m] == 2:
            out.append(v & 0xFF)
        elif SIZES[m] == 3:
            out += struct.pack('<H', v & 0xFFFF)
        return out


# ---------------------------------------------------------------------------
# NSF and NSFE


def nsf_header(tracks, first, load, init, play, game, author, copyright, ntsc_speed=0x411A,
               banks=(0,) * 8, pal_speed=0x4E20, speed_flags=0, chip_flags=0, version=1):
    def s32(s):
        b = s.encode('latin-1')[:32]
        return b + bytes(32 - len(b))
    h = b'NESM\x1A' + bytes([version, tracks, first]) + struct.pack('<HHH', load, init, play)
    h += s32(game) + s32(author) + s32(copyright)
    h += struct.pack('<H', ntsc_speed) + bytes(banks) + struct.pack('<H', pal_speed)
    h += bytes([speed_flags, chip_flags]) + bytes(4)
    assert len(h) == 0x80
    return h


# The CPU exercise: a routine using most instructions and addressing
# modes, which leaves a value in $10 (and the stack as it found it) that
# the songs use, so a mistake in any of them changes the sound.
CPU_EXERCISE = '''
cpu_test:
    LDA #$5A
    STA $20
    LDA #$00
    STA $21
    LDX #$07
ct_loop:
    LDA $20
    ASL A
    ROL $21
    BCC ct_nc
    EOR #$1D
ct_nc:
    STA $20
    CLC
    ADC $21
    STA $30,X
    SEC
    SBC #$37
    BVS ct_ov
    ORA #$01
ct_ov:
    DEX
    BPL ct_loop
    ; indirect addressing
    LDA #$30
    STA $40
    LDA #$00
    STA $41
    LDY #$03
    LDA ($40),Y
    LDX #$02
    EOR ($3E,X)
    STA $22
    LAX ($40),Y
    STX $23
    SAX $0624
    LDA #$F0
    LDX #$3C
    SBX #$11
    STX $25
    ; stack and flags
    PHP
    PHA
    LDA #$C3
    PHA
    PLA
    BIT $22
    BMI ct_neg
    LSR A
ct_neg:
    ROR A
    STA $26
    PLA
    PLP
    TSX
    TXA
    PHA
    LDX #$FF
    TXS
    TSX
    PLA
    TAX
    TXS
    ; compares
    LDA $22
    CMP $23
    BCS ct_ge
    INC $26
ct_ge:
    LDX $25
    CPX #$40
    BNE ct_ne
    DEC $26
ct_ne:
    LDY $26
    CPY $0624
    BEQ ct_eq
    INY
ct_eq:
    STY $27
    ; read-modify-write on absolute and indexed memory
    LDA #$81
    STA $0600
    STA $0605
    ASL $0600
    LSR $0605
    LDX #$05
    ROL $0600,X
    ROR $0600,X
    INC $0600,X
    DEC $0600
    LDA $0600
    CLC
    ADC $0605
    ADC $0604,X ; (crosses into $0609)
    STA $28
    ; JMP indirect, JSR/RTS
    LDA #<ct_jmp
    STA $0610
    LDA #>ct_jmp
    STA $0611
    JMP ($0610)
ct_back:
    LDA $20
    EOR $22
    EOR $25
    EOR $26
    EOR $27
    EOR $28
    STA $10
    RTS
ct_jmp:
    JSR ct_sub
    JMP ct_back
ct_sub:
    SED
    CLD
    CLV
    LDA #$7F
    ADC #$01
    BVC ct_sub_done
    INC $27
ct_sub_done:
    RTS
'''


def nsf_apu():
    a = Asm(0x8000)
    a('''
    JMP init
    JMP play
init:
    STA $00          ; track
    STX $01          ; PAL flag
    JSR cpu_test
    LDA #$0F
    STA $4015
    LDA #$BF
    STA $4000
    LDA #$08
    STA $4001
    LDA $10
    ORA #$40
    STA $4002
    LDA #$08
    STA $4003
    LDA #$7C
    STA $4004
    LDA #$A3
    STA $4005
    LDA #$80
    STA $4006
    LDA #$09
    STA $4007
    LDA #$FF
    STA $4008
    LDA #$90
    STA $400A
    LDA #$08
    STA $400B
    LDA #$3A
    STA $400C
    LDA $00
    BEQ init_n0
    LDA #$83
    JMP init_n1
init_n0:
    LDA #$05
init_n1:
    STA $400E
    LDA #$08
    STA $400F
    ; DMC
    LDA $00
    BEQ init_d0
    LDA #$4E         ; loop, rate 14
    JMP init_d1
init_d0:
    LDA #$8C         ; irq enabled, rate 12
init_d1:
    STA $4010
    LDA #$40
    STA $4011
    LDA #$30         ; $C000 + $30 * 64: bank 0 at $8C00, mirrored there
    STA $4012
    LDA #$02
    STA $4013
    LDA #$1F
    STA $4015
    ; code in RAM: copy a routine to $0700 and run it
    LDX #ram_code_end-ram_code-1
init_copy:
    LDA ram_code,X
    STA $0700,X
    DEX
    BPL init_copy
    JSR $0700
    LDA #$00
    STA $02
    RTS
ram_code:
    LDA #$11
    STA $0620
    RTS
ram_code_end:

play:
    INC $02
    LDA $02
    AND #$07
    TAX
    LDA notes,X
    CLC
    ADC $10
    STA $4002
    LDA $02
    LSR A
    LSR A
    AND #$07
    TAX
    LDA notes,X
    STA $400A
    LDA $4015        ; status (and the frame IRQ flag)
    STA $03
    AND #$10
    BNE play_dmc_on
    LDA $00
    BNE play_dmc_on
    LDA $02
    AND #$0F
    BNE play_dmc_on
    LDA #$1F         ; restart the DMC
    STA $4015
play_dmc_on:
    LDA $02
    AND #$1F
    BNE play_done
    LDA #$C0         ; frame counter mode 1
    STA $4017
    LDA $02
    ORA #$80
    STA $4001        ; sweep square 1
    LDA #$08
    STA $4003
    LDA $02
    AND #$3F
    STA $4011
play_done:
    RTS
notes:
    .db $FD, $E1, $C9, $BD, $A9, $96, $86, $7E
''')
    a(CPU_EXERCISE)
    a('''
    .org $8C00
dmc_sample:
    .db $55, $AA, $0F, $F0, $33, $CC, $FF, $00, $5A, $A5, $69, $96, $3C, $C3, $E1, $1E
    .db $55, $AA, $0F, $F0, $33, $CC, $FF, $00, $5A, $A5, $69, $96, $3C, $C3, $E1, $1E
    .db $00, $01, $02, $04, $08, $10, $20, $40, $80, $FF, $7F, $3F, $1F, $0F, $07, $03
''')
    code = a.assemble()
    h = nsf_header(2, 1, 0x8000, 0x8000, 0x8003, 'APU Test', 'gen_gme_testdata.py', '2026 Public Domain')
    write('nsf_apu.nsf', h + code)


def nsf_banks():
    # A bank-switched PAL rip loaded at $8080: three 4 KB banks (the first
    # $80 bytes of the first not in the file), the play routine switching
    # the one at $9000 every frame and playing the note it finds there.
    a = Asm(0x8080)
    a('''
init:
    STA $00
    LDA #$0F
    STA $4015
    LDA #$9F
    STA $4000
    LDA #$5F
    STA $4004
    LDA #$00
    STA $02
    LDA $00
    STA $5FF9        ; bank at $9000
    RTS
play:
    INC $02
    LDA $02
    AND #$01
    CLC
    ADC #$01
    STA $5FF9
    LDA $9000
    STA $4002
    LDA $9001
    STA $4003
    LDA $9002
    STA $4006
    LDA $9003
    STA $4007
    LDA $02
    CMP #$30
    BNE play_r
    LDA #$07         ; an invalid bank
    STA $5FFA
play_r:
    RTS
''')
    banks = [a.assemble().ljust(0x1000 - 0x80, b'\xEA')]
    banks.append(bytes([0x63, 0x09, 0x7F, 0x09]).ljust(0x1000, b'\x01'))
    banks.append(bytes([0x86, 0x08, 0x5E, 0x09]) + bytes(range(2, 120, 3)))
    data = b''.join(banks)
    h = nsf_header(3, 2, 0x8080, 0x8080, a.labels['play'], 'Banks', 'Nobody', '',
                   ntsc_speed=20000, banks=(0, 1, 2, 0, 0, 0, 0, 0), pal_speed=19997, speed_flags=1)
    write('nsf_banks.nsf', h + data)


def nsf_exp():
    # Every expansion sound chip at once (more voices than libgme's voice
    # type table has).
    a = Asm(0x8000)
    a('''
init:
    STA $00
    LDA #$0F
    STA $4015
    LDA #$BF
    STA $4000
    LDA #$C9
    STA $4002
    LDA #$08
    STA $4003
    ; VRC6: two squares and the saw
    LDA #$5F
    STA $9000
    LDA #$80
    STA $9001
    LDA #$81
    STA $9002
    LDA #$3A
    STA $A000
    LDA #$40
    STA $A001
    LDA #$82
    STA $A002
    LDA #$12
    STA $B000
    LDA #$A0
    STA $B001
    LDA #$80
    STA $B002
    ; FME-7: tones on A and B
    LDX #$00
init_fme:
    TXA
    STA $C000
    LDA fme_regs,X
    STA $E000
    INX
    CPX #$0E
    BNE init_fme
    ; Namco 106: a wave and two channels
    LDA #$80         ; auto-increment from 0
    STA $F800
    LDX #$00
init_wave:
    LDA n163_wave,X
    STA $4800
    INX
    CPX #$10
    BNE init_wave
    LDA #$F8         ; channel 7 regs at $78
    STA $F800
    LDX #$00
init_n7:
    LDA n163_ch,X
    STA $4800
    INX
    CPX #$08
    BNE init_n7
    LDA #$F0
    STA $F800
    LDX #$00
init_n6:
    LDA n163_ch2,X
    STA $4800
    INX
    CPX #$08
    BNE init_n6
    LDA #$7F
    STA $F800
    LDA $4800        ; read back (auto-increment off)
    STA $11
    ; FDS: wave, volume, frequency, modulation
    LDA #$80
    STA $4089        ; wave write enable
    LDX #$00
init_fdsw:
    TXA
    ASL A
    AND #$3F
    STA $4040,X
    INX
    CPX #$40
    BNE init_fdsw
    LDA #$00
    STA $4089        ; master volume 100%
    LDA #$A0         ; volume 32, envelope off
    STA $4080
    LDA #$50
    STA $4082
    LDA #$02
    STA $4083
    LDA #$80
    STA $4087        ; mod off to write the table
    LDX #$00
init_mod:
    LDA mod_table,X
    STA $4088
    INX
    CPX #$20
    BNE init_mod
    LDA #$90         ; sweep gain
    STA $4084
    LDA #$00
    STA $4085
    LDA #$20
    STA $4086
    LDA #$01
    STA $4087
    LDA #$E8
    STA $408A
    LDA $4090        ; envelope gain
    STA $12
    ; MMC5: squares, multiplier, ExRAM
    LDA #$03
    STA $5015
    LDA #$9F
    STA $5000
    LDA #$A9
    STA $5002
    LDA #$08
    STA $5003
    LDA #$5C
    STA $5004
    LDA #$53
    STA $5006
    LDA #$09
    STA $5007
    LDA #$0B
    STA $5205
    LDA #$0D
    STA $5206
    LDA $5205
    STA $5C10
    LDA $5C10
    STA $13
    ; VRC7: a custom instrument and two channels
    LDX #$00
init_vrc7i:
    STX $9010
    LDA vrc7_inst,X
    STA $9030
    INX
    CPX #$08
    BNE init_vrc7i
    LDA #$30
    STA $9010
    LDA #$00         ; custom instrument, full volume
    STA $9030
    LDA #$10
    STA $9010
    LDA #$AC
    STA $9030
    LDA #$20
    STA $9010
    LDA #$1C         ; key on, octave 6
    STA $9030
    LDA #$31
    STA $9010
    LDA #$32         ; instrument 3
    STA $9030
    LDA #$11
    STA $9010
    LDA #$58
    STA $9030
    LDA #$21
    STA $9010
    LDA #$3A         ; key on, sustain
    STA $9030
    LDA #$00
    STA $02
    RTS
play:
    INC $02
    LDA $02
    AND #$0F
    BNE play_r
    LDA $02
    LSR A
    LSR A
    ORA #$40
    STA $9001        ; VRC6 square 1 period
    STA $4082        ; FDS frequency
    LDA #$08
    STA $C000
    LDA $02
    LSR A
    LSR A
    LSR A
    LSR A
    AND #$0F
    STA $E000        ; FME-7 volume A
    LDA #$20
    STA $9010
    LDA $02
    AND #$10
    ORA #$0C
    STA $9030        ; VRC7 key on/off
    LDA $02
    STA $5002
    LDA #$C0         ; FDS envelope on
    STA $4080
play_r:
    RTS
fme_regs:
    .db $40, $01, $80, $00, $00, $02, $00, $3C, $0C, $0A, $10, $00, $00, $00
n163_wave:
    .db $10, $32, $54, $76, $98, $BA, $DC, $FE, $EF, $CD, $AB, $89, $67, $45, $23, $01
n163_ch:
    .db $00, $00, $40, $00, $E0, $00, $00, $7F   ; 2 channels active
n163_ch2:
    .db $80, $00, $20, $00, $E1, $00, $00, $0C
mod_table:
    .db 0, 1, 2, 3, 4, 5, 6, 7, 7, 6, 5, 4, 3, 2, 1, 0
    .db 1, 1, 7, 7, 2, 2, 6, 6, 3, 4, 3, 4, 0, 0, 1, 7
vrc7_inst:
    .db $21, $41, $1A, $07, $F0, $F2, $14, $24
''')
    code = a.assemble()
    h = nsf_header(1, 1, 0x8000, 0x8000, a.labels['play'], 'Expansion', 'All Chips', '<?>',
                   chip_flags=0x3F)
    write('nsf_exp.nsf', h + code)


def nsf_vrc7():
    # The VRC7 alone (all six channels on one buffer: emu2413's mono path),
    # every patch, and the rhythm register it ignores.
    a = Asm(0x8000)
    a('''
init:
    STA $00
    LDA #$0E
    STA $9010
    LDA #$3F         ; (rhythm: ignored on the VRC7)
    STA $9030
    LDX #$00
init_ch:
    TXA
    CLC
    ADC #$30
    STA $9010
    TXA
    ASL A
    ASL A
    ASL A
    ASL A
    ADC #$10
    ADC $00
    STA $9030        ; instrument, volume
    TXA
    CLC
    ADC #$10
    STA $9010
    LDA fnums,X
    STA $9030
    TXA
    CLC
    ADC #$20
    STA $9010
    TXA
    AND #$01
    ORA #$18
    STA $9030        ; key on
    INX
    CPX #$06
    BNE init_ch
    RTS
play:
    INC $02
    LDA $02
    AND #$1F
    BNE play_r
    LDA $02
    LSR A
    LSR A
    LSR A
    LSR A
    LSR A
    AND #$07
    TAX
    LDA #$20
    STA $9010
    TXA
    ASL A
    ORA #$12
    STA $9030
    LDA #$00
    STA $9010
    LDA $02
    STA $9030        ; custom instrument modulator
play_r:
    RTS
fnums:
    .db $AC, $C0, $D8, $E4, $FF, $90
''')
    code = a.assemble()
    h = nsf_header(2, 1, 0x8000, 0x8000, a.labels['play'], 'VRC7', '', '', chip_flags=0x02)
    write('nsf_vrc7.nsf', h + code)


def nsf_odd():
    # Odd programs: illegal opcodes (skipped, and the halting ones), a play
    # routine that never returns, BRK, an unsupported chip flag and an
    # unknown version.
    a = Asm(0x8000)
    a('''
init:
    STA $00
    LDA #$0F
    STA $4015
    LDA #$9F
    STA $4000
    LDA #$90
    STA $4002
    LDA #$08
    STA $4003
    LDA $00
    BEQ init_t0
    CMP #$01
    BEQ init_t1
    ; track 2: BRK into the vector at $FFFE
    BRK
    .db $00
    RTS
init_t0:
    .db $1A, $3A, $04, $10, $0C, $34, $12, $1C, $56, $34   ; NOPs, SKB, SKW
    .db $07, $10     ; SLO (unimplemented: skipped)
    .db $9C, $00, $06
    .db $B7, $10
    .db $FF, $00, $00
    LDA #$C0
    STA $4002
    RTS
init_t1:
    LDA #$A0
    STA $4002
    .db $02          ; halts the CPU
    RTS
play:
    INC $02
    LDA $02
    STA $4002
    LDA $00
    CMP #$01
    BNE play_r
play_forever:
    INC $4002
    JMP play_forever
play_r:
    RTS
brk_handler:
    LDA #$55
    STA $4002
    CLI
    SEI
    PLA
    PLA
    PLA
    RTS
    .org $8FFA       ; (bank 0 is mirrored at $F000)
    .dw brk_handler, brk_handler, brk_handler
''')
    code = a.assemble()
    h = nsf_header(3, 1, 0x8000, 0x8000, a.labels['play'], 'Odd Programs', '  padded  ', '?',
                   chip_flags=0x40, version=2)
    write('nsf_odd.nsf', h + code)

    # an invalid load address
    h = nsf_header(1, 1, 0x6000, 0x8000, 0x8000, 'Bad', '', '')
    write('nsf_badaddr.nsf', h + bytes([0x60]) * 64)


def nsfe_chunk(tag, data):
    return struct.pack('<I', len(data)) + tag + data


def nsfe_files():
    a = Asm(0x8000)
    a('''
init:
    STA $00
    LDA #$0F
    STA $4015
    LDA #$9F
    STA $4000
    LDA $00
    ASL A
    ASL A
    ASL A
    ASL A
    ORA #$80
    STA $4002
    LDA #$08
    STA $4003
    LDA #$7F
    STA $4008
    LDA #$C0
    STA $400A
    LDA #$08
    STA $400B
    RTS
play:
    INC $01
    LDA $01
    AND #$20
    BEQ play_r
    LDA $01
    STA $400A
play_r:
    RTS
''')
    code = a.assemble()
    play = a.labels['play']
    info = struct.pack('<HHH', 0x8000, 0x8000, play) + bytes([0, 0, 4, 0]) + bytes(6)
    auth = b'NSFE Game\0Some Author\0(C) 2026\0Ripper Name\0extra'
    tlbl = b'First\0  Second  \0\0Fourth'
    time = struct.pack('<iiii', 12000, -1, 0, 65000)
    plst = bytes([3, 0, 1])
    data = (b'NSFE' + nsfe_chunk(b'INFO', info) + nsfe_chunk(b'BANK', bytes(8))
            + nsfe_chunk(b'auth', auth) + nsfe_chunk(b'tlbl', tlbl) + nsfe_chunk(b'time', time)
            + nsfe_chunk(b'plst', plst) + nsfe_chunk(b'xtra', b'skipped')
            + nsfe_chunk(b'DATA', code) + nsfe_chunk(b'NEND', b''))
    write('nsfe_full.nsfe', data)

    # the smallest: INFO of 9 bytes (a track count), DATA, NEND
    info9 = struct.pack('<HHH', 0x8000, 0x8000, play) + bytes([0, 0, 2])
    data = b'NSFE' + nsfe_chunk(b'INFO', info9) + nsfe_chunk(b'DATA', code) + nsfe_chunk(b'NEND', b'')
    write('nsfe_min.nsfe', data)

    # DATA before INFO (no track count yet), a big INFO, an empty plst
    info20 = info + bytes(4)
    data = (b'NSFE' + nsfe_chunk(b'DATA', code) + nsfe_chunk(b'INFO', info20)
            + nsfe_chunk(b'auth', b'Only Game') + nsfe_chunk(b'NEND', b''))
    write('nsfe_order.nsfe', data)


# ---------------------------------------------------------------------------
# A small assembler for the Game Boy's CPU (the LR35902), on the same
# two-pass assembler: the operands are written as in the usual syntax
# (`LD A,(HL+)`, `JR NZ,label`, `LDH ($80),A`, `BIT 7,(HL)`...).

GB_R8 = ['B', 'C', 'D', 'E', 'H', 'L', '(HL)', 'A']
GB_R16 = ['BC', 'DE', 'HL', 'SP']
GB_R16_STACK = ['BC', 'DE', 'HL', 'AF']
GB_CC = ['NZ', 'Z', 'NC', 'C']
GB_ALU = ['ADD', 'ADC', 'SUB', 'SBC', 'AND', 'XOR', 'OR', 'CP']
GB_CB_SHIFTS = ['RLC', 'RRC', 'RL', 'RR', 'SLA', 'SRA', 'SWAP', 'SRL']
GB_IMPLIED = {
    'NOP': 0x00, 'RLCA': 0x07, 'RRCA': 0x0F, 'RLA': 0x17, 'RRA': 0x1F, 'DAA': 0x27,
    'CPL': 0x2F, 'SCF': 0x37, 'CCF': 0x3F, 'HALT': 0x76, 'RETI': 0xD9, 'DI': 0xF3,
    'EI': 0xFB,
}


class GbAsm(Asm):
    """Assembles Game Boy CPU source text (see Asm for the directives and
    numbers)."""

    def encode(self, mnem, arg, labels, final, pc, line):
        ops = [o.strip() for o in arg.split(',')] if arg else []
        ops_u = [o.upper() for o in ops]
        val = lambda s: self.value(s, labels, final)

        def imm8(s):
            return bytes([val(s) & 0xFF])

        def imm16(s):
            return struct.pack('<H', val(s) & 0xFFFF)

        def rel(s, size):
            off = val(s) - (pc + size)
            if final:
                assert -128 <= off <= 127, (line, off)
            return bytes([off & 0xFF])

        def is_ind(s):
            return s.startswith('(') and s.endswith(')')

        if mnem in GB_IMPLIED and not ops:
            return bytes([GB_IMPLIED[mnem]])
        if mnem == 'STOP':
            return bytes([0x10, 0x00])
        if mnem == 'LD':
            d, s = ops_u
            if d in GB_R8 and s in GB_R8:
                return bytes([0x40 + GB_R8.index(d) * 8 + GB_R8.index(s)])
            table = {
                ('(BC)', 'A'): 0x02, ('(DE)', 'A'): 0x12, ('(HL+)', 'A'): 0x22, ('(HL-)', 'A'): 0x32,
                ('A', '(BC)'): 0x0A, ('A', '(DE)'): 0x1A, ('A', '(HL+)'): 0x2A, ('A', '(HL-)'): 0x3A,
                ('(C)', 'A'): 0xE2, ('A', '(C)'): 0xF2, ('SP', 'HL'): 0xF9,
            }
            if (d, s) in table:
                return bytes([table[(d, s)]])
            if d in GB_R8 and not is_ind(s):
                return bytes([0x06 + GB_R8.index(d) * 8]) + imm8(ops[1])
            if d == 'HL' and (s.startswith('SP+') or s.startswith('SP-')):
                return bytes([0xF8]) + imm8(('' if s[2] == '+' else '0-') + ops[1][3:])
            if d in GB_R16 and not is_ind(s):
                return bytes([0x01 + GB_R16.index(d) * 16]) + imm16(ops[1])
            if is_ind(d) and s == 'A':
                return bytes([0xEA]) + imm16(ops[0][1:-1])
            if is_ind(d) and s == 'SP':
                return bytes([0x08]) + imm16(ops[0][1:-1])
            if d == 'A' and is_ind(s):
                return bytes([0xFA]) + imm16(ops[1][1:-1])
            raise ValueError(line)
        if mnem == 'LDH':
            d, s = ops_u
            if s == 'A':
                return bytes([0xE0]) + imm8(ops[0][1:-1])
            return bytes([0xF0]) + imm8(ops[1][1:-1])
        if mnem in GB_ALU:
            if len(ops) == 2 and ops_u[0] == 'HL':
                return bytes([0x09 + GB_R16.index(ops_u[1]) * 16])
            if len(ops) == 2 and ops_u[0] == 'SP':
                return bytes([0xE8]) + imm8(ops[1])
            if len(ops) == 2:
                assert ops_u[0] == 'A', line
                ops, ops_u = ops[1:], ops_u[1:]
            i = GB_ALU.index(mnem)
            if ops_u[0] in GB_R8:
                return bytes([0x80 + i * 8 + GB_R8.index(ops_u[0])])
            return bytes([0xC6 + i * 8]) + imm8(ops[0])
        if mnem in ('INC', 'DEC'):
            o = ops_u[0]
            if o in GB_R8:
                return bytes([(0x04 if mnem == 'INC' else 0x05) + GB_R8.index(o) * 8])
            return bytes([(0x03 if mnem == 'INC' else 0x0B) + GB_R16.index(o) * 16])
        if mnem in ('PUSH', 'POP'):
            return bytes([(0xC5 if mnem == 'PUSH' else 0xC1) + GB_R16_STACK.index(ops_u[0]) * 16])
        if mnem == 'JP':
            if ops_u == ['(HL)'] or ops_u == ['HL']:
                return bytes([0xE9])
            if len(ops) == 2:
                return bytes([0xC2 + GB_CC.index(ops_u[0]) * 8]) + imm16(ops[1])
            return bytes([0xC3]) + imm16(ops[0])
        if mnem == 'JR':
            if len(ops) == 2:
                return bytes([0x20 + GB_CC.index(ops_u[0]) * 8]) + rel(ops[1], 2)
            return bytes([0x18]) + rel(ops[0], 2)
        if mnem == 'CALL':
            if len(ops) == 2:
                return bytes([0xC4 + GB_CC.index(ops_u[0]) * 8]) + imm16(ops[1])
            return bytes([0xCD]) + imm16(ops[0])
        if mnem == 'RET':
            if ops:
                return bytes([0xC0 + GB_CC.index(ops_u[0]) * 8])
            return bytes([0xC9])
        if mnem == 'RST':
            return bytes([0xC7 + (val(ops[0]) & 0x38)])
        if mnem in GB_CB_SHIFTS:
            return bytes([0xCB, GB_CB_SHIFTS.index(mnem) * 8 + GB_R8.index(ops_u[0])])
        if mnem in ('BIT', 'RES', 'SET'):
            base = {'BIT': 0x40, 'RES': 0x80, 'SET': 0xC0}[mnem]
            return bytes([0xCB, base + (val(ops[0]) & 7) * 8 + GB_R8.index(ops_u[1])])
        if mnem == '.OP':
            # a raw opcode (the ones the CPU emulator doesn't run)
            return bytes([val(ops[0]) & 0xFF])
        raise ValueError('unknown instruction: ' + line)


def gbs_header(tracks, first, load, init, play, sp, game, author, copyright, timer_modulo=0,
               timer_mode=0, version=1):
    def s32(s):
        b = s.encode('latin-1')[:32]
        return b + bytes(32 - len(b))
    h = b'GBS' + bytes([version, tracks, first]) + struct.pack('<HHHH', load, init, play, sp)
    h += bytes([timer_modulo, timer_mode]) + s32(game) + s32(author) + s32(copyright)
    assert len(h) == 112
    return h


# The Game Boy CPU exercise: most instructions, on every register and
# (HL), leaving a value in $C010 (and the stack as it found it) that the
# songs use, so a mistake in any of them changes the sound.
GB_CPU_EXERCISE = '''
cpu_test:
    PUSH AF
    PUSH BC
    PUSH DE
    PUSH HL
    LD A,$5A
    LD B,$13
    LD C,$C4
    LD D,$7F
    LD E,$80
    LD HL,$C100
    ; 8-bit loads and arithmetic on every register
    LD (HL),$29
    ADD A,B
    ADC A,C
    SUB D
    SBC A,E
    AND $F7
    XOR H
    OR L
    CP (HL)
    ADD A,(HL)
    ADC A,$33
    SUB $11
    SBC A,(HL)
    XOR (HL)
    OR (HL)
    AND (HL)
    LD (HL+),A
    LD (HL-),A
    LD A,(HL+)
    LD A,(HL-)
    CP $40
    JR C,ct_lt
    CPL
ct_lt:
    LD B,A
    LD C,B
    LD D,C
    LD E,D
    LD H,$C1
    LD L,$02
    LD (HL),E
    INC (HL)
    INC (HL)
    DEC (HL)
    LD A,(HL)
    INC A
    DEC B
    INC C
    DEC D
    INC E
    ADD A,A
    ADC A,B
    SBC A,C
    SUB E
    ADD A,D
    ADD A,H
    ADD A,L
    LD (HL),A
    ; 16-bit
    LD BC,$1234
    LD DE,$FEDC
    LD HL,$0F0F
    ADD HL,BC
    ADD HL,DE
    ADD HL,HL
    INC BC
    DEC DE
    INC HL
    ADD HL,SP
    LD A,H
    XOR L
    LD ($C103),A
    LD HL,SP+$05
    LD A,L
    LD ($C104),A
    LD HL,SP-$03
    LD A,L
    LD ($C105),A
    ADD SP,$08
    ADD SP,$F8
    LD ($C106),SP
    ; the CB instructions
    LD A,$96
    LD B,$69
    RLC A
    RRC B
    RL A
    RR B
    SLA A
    SRA B
    SWAP A
    SRL B
    LD C,A
    LD D,B
    RLCA
    RRCA
    RLA
    RRA
    LD HL,$C107
    LD (HL),$C3
    RLC (HL)
    RR (HL)
    SLA (HL)
    SRA (HL)
    SRL (HL)
    SWAP (HL)
    RL (HL)
    RRC (HL)
    BIT 7,(HL)
    JR Z,ct_b0
    SET 0,(HL)
ct_b0:
    BIT 0,C
    JR NZ,ct_b1
    RES 7,(HL)
ct_b1:
    SET 3,D
    RES 1,C
    BIT 6,D
    LD A,(HL)
    XOR C
    XOR D
    LD E,A
    ; flags
    SCF
    CCF
    JR NC,ct_f0
    INC E
ct_f0:
    SCF
    LD A,E
    ADC A,$00
    LD E,A
    AND A
    OR A
    XOR A
    JR NZ,ct_f1
    INC E
ct_f1:
    ; the stack and AF
    LD BC,$12F0
    PUSH BC
    POP AF
    PUSH AF
    POP BC
    LD A,C
    XOR E
    LD E,A
    ; memory through BC and DE, the high page
    LD BC,$C108
    LD DE,$C109
    LD A,E
    LD (BC),A
    LD (DE),A
    LD A,(BC)
    LD H,A
    LD A,(DE)
    ADD A,H
    LDH ($80),A
    LD C,$81
    LD (C),A
    LDH A,($80)
    LD B,A
    LD A,(C)
    ADD A,B
    XOR E
    LD E,A
    ; calls, returns and jumps
    CALL ct_sub
    CALL Z,ct_sub
    CALL NZ,ct_sub
    CALL C,ct_sub
    CALL NC,ct_sub
    LD HL,ct_jp
    JP HL
ct_jp_back:
    XOR A
    JP Z,ct_jz
    INC E
ct_jz:
    JP NZ,ct_jnz
    SCF
    JP C,ct_jc
    INC E
ct_jc:
    JP NC,ct_jnz
    INC E
    INC E
ct_jnz:
    RST $08
    LD A,E
    LD ($C010),A
    POP HL
    POP DE
    POP BC
    POP AF
    RET
ct_jp:
    INC E
    JP ct_jp_back
ct_sub:
    INC E
    LD A,E
    AND $03
    RET Z
    CP $02
    RET C
    SCF
    RET NC
    RET NZ
    RET
'''


def gbs_apu():
    a = GbAsm(0x400)
    a('''
    .org $0408
rst_08:
    INC E
    RET
    .org $0410
init:
    LD ($C000),A     ; track
    CALL cpu_test
    LD A,$80
    LDH ($26),A      ; power on
    LD A,$77
    LDH ($24),A
    LD A,$FF
    LDH ($25),A
    ; square 1: sweep and envelope
    LD A,$35
    LDH ($10),A
    LD A,$80
    LDH ($11),A
    LD A,$F3
    LDH ($12),A
    LD A,($C010)
    LDH ($13),A
    LD A,$86
    LDH ($14),A
    ; square 2: duty 3, rising envelope, a length
    LD A,$D0
    LDH ($16),A
    LD A,$2F
    LDH ($17),A
    LD A,$40
    LDH ($18),A
    LD A,$C7
    LDH ($19),A
    ; wave: its RAM, then on
    LD A,$00
    LDH ($1A),A
    LD HL,$FF30
    LD B,$10
    LD A,($C000)
    LD C,A
init_wave:
    LD A,B
    SWAP A
    XOR C
    ADD A,B
    LD (HL+),A
    DEC B
    JR NZ,init_wave
    LD A,$80
    LDH ($1A),A
    LD A,$20
    LDH ($1C),A
    LD A,$30
    LDH ($1D),A
    LD A,$87
    LDH ($1E),A
    ; noise: 7-bit for track 1
    LD A,$A1
    LDH ($21),A
    LD A,($C000)
    AND A
    LD A,$35
    JR Z,init_n7
    LD A,$3D
init_n7:
    LDH ($22),A
    LD A,$80
    LDH ($23),A
    XOR A
    LD ($C001),A
    RET

play:
    LD HL,$C001
    INC (HL)
    LD A,(HL)
    LD B,A
    AND $07
    LD E,A
    LD D,$00
    LD HL,notes
    ADD HL,DE
    LD A,(HL)
    LDH ($18),A
    LD A,$87
    LDH ($19),A
    ; the status, read back into the wave's volume
    LDH A,($26)
    LD ($C002),A
    AND $04
    RLCA
    RLCA
    RLCA
    OR $20
    LDH ($1C),A
    ; panning every 8 frames, master volume every 16
    LD A,B
    AND $07
    JR NZ,play_p
    LD A,B
    RRCA
    RRCA
    RRCA
    XOR $5A
    LDH ($25),A
play_p:
    LD A,B
    AND $0F
    JR NZ,play_v
    LD A,B
    SWAP A
    AND $77
    OR $11
    LDH ($24),A
play_v:
    ; retrigger square 1 and noise every 32
    LD A,B
    AND $1F
    JR NZ,play_t
    LD A,$86
    LDH ($14),A
    LD A,$80
    LDH ($23),A
play_t:
    ; power off and on at frame 100 of track 2
    LD A,($C000)
    CP $02
    RET NZ
    LD A,B
    CP 100
    RET NZ
    XOR A
    LDH ($26),A
    LD A,$80
    LDH ($26),A
    LD A,$F0
    LDH ($12),A
    LD A,$87
    LDH ($14),A
    RET
notes:
    .db $10, $40, $70, $90, $A8, $C0, $D0, $E0
''')
    a(GB_CPU_EXERCISE)
    code = a.assemble()
    h = gbs_header(3, 1, 0x400, 0x410, a.labels['play'], 0xFFFE, 'GBS Test',
                   'gen_gme_testdata.py', '2026 Public Domain')
    write('gbs_apu.gbs', h + code)


def gbs_timer():
    # the timer drives play; the ROM crosses into a second bank (loaded
    # just below it), switched by play (banks 1, 2, which wraps to 0, and
    # 3, past the ROM); the timer rate changed while playing
    a = GbAsm(0x3F00)
    a('''
init:
    LD ($C000),A
    LD A,$80
    LDH ($26),A
    LD A,$77
    LDH ($24),A
    LD A,$FF
    LDH ($25),A
    LD A,$C2
    LDH ($11),A
    LD A,$F0
    LDH ($12),A
    LD A,$87
    LDH ($14),A
    LD A,$00
    LD ($C001),A
    RET
play:
    LD HL,$C001
    INC (HL)
    LD A,(HL)
    AND $03
    LD ($2000),A     ; bank 0 (as 1), 1, 2 or 3 at $4000
    LD A,($4000)
    ADD A,$40
    LDH ($13),A
    LD A,$86
    LDH ($14),A
    LD A,(HL)
    CP $40
    RET NZ
    LD A,$06         ; faster timer
    LDH ($07),A
    LD A,$E0
    LDH ($06),A
    RET
    .org $4000
    .db $20, $30, $40
''')
    code = a.assemble()
    h = gbs_header(2, 1, 0x3F00, 0x3F00, a.labels['play'], 0xDFFF, 'Timer Banks', 'Nobody', '',
                   timer_modulo=0xC0, timer_mode=0x04)
    write('gbs_timer.gbs', h + code)

    # the same, with the timer at its other rates and the double-speed bit
    h = gbs_header(1, 1, 0x3F00, 0x3F00, a.labels['play'], 0xDFFF, 'Timer Fast', '', '',
                   timer_modulo=0x80, timer_mode=0x87)
    write('gbs_timer2.gbs', h + code)


def gbs_odd():
    # an invalid timer mode and a load address below
    # $400 (warnings), the instructions the CPU emulator stops at, code
    # that runs off the end of memory (track 1), the bank register past
    # the ROM, writes to the unmapped and I/O pages, a stack over the APU
    a = GbAsm(0x100)
    a('''
init:
    LD ($C000),A
    AND A
    JR Z,init0
    DEC A
    JR Z,init1
    ; track 2: the stack over the APU's registers (triggering the wave)
    LD A,$80
    LDH ($26),A
    LDH ($1A),A
    LD A,$20
    LDH ($1C),A
    LD SP,$FF20
    LD BC,$F387
    PUSH BC
    LD SP,$FFFC
    RET
init1:
    LD A,$80
    LDH ($26),A
    LD A,$F0
    LDH ($17),A
    LD A,$87
    LDH ($19),A
    LD HL,$FFF0
    JP HL            ; runs off the end of memory, through NOPs
init0:
    LD A,$80
    LDH ($26),A
    LD A,$1F
    LD ($2000),A     ; a bank past the ROM
    LD A,$55
    LD ($8000),A     ; unmapped
    LD ($E000),A     ; echo RAM, reads back as $FF
    LD A,($E000)
    LDH ($13),A
    LDH A,($00)      ; joypad
    LDH ($11),A
    LD A,$F1
    LDH ($12),A
    LD A,$87
    LDH ($14),A
    .op $76          ; HALT, which stops the CPU
    .op $D3          ; and an unknown opcode
    .op $10          ; STOP
    RET
play:
    LD HL,$C001
    INC (HL)
    LD A,(HL)
    LDH ($13),A
    .op $DB
    RET
''')
    code = a.assemble()
    h = gbs_header(3, 2, 0x100, 0x100, a.labels['play'], 0xFFFE, 'Odd GBS', '', '(C)',
                   timer_mode=0x7B)
    write('gbs_odd.gbs', h + code)


# ---------------------------------------------------------------------------
# A small assembler for the Z80 (with the undocumented IXH/IXL/IYH/IYL
# and SLL forms the emulators run), on the same two-pass assembler: the
# usual syntax (`LD A,(IX+5)`, `JR NZ,label`, `OUT (C),A`, `EX AF,AF'`...).

Z80_R8 = ['B', 'C', 'D', 'E', 'H', 'L', '(HL)', 'A']
Z80_R16 = ['BC', 'DE', 'HL', 'SP']
Z80_R16_STACK = ['BC', 'DE', 'HL', 'AF']
Z80_CC = ['NZ', 'Z', 'NC', 'C', 'PO', 'PE', 'P', 'M']
Z80_ALU = ['ADD', 'ADC', 'SUB', 'SBC', 'AND', 'XOR', 'OR', 'CP']
Z80_CB_SHIFTS = ['RLC', 'RRC', 'RL', 'RR', 'SLA', 'SRA', 'SLL', 'SRL']
Z80_IMPLIED = {
    'NOP': [0x00], 'RLCA': [0x07], 'RRCA': [0x0F], 'RLA': [0x17], 'RRA': [0x1F],
    'DAA': [0x27], 'CPL': [0x2F], 'SCF': [0x37], 'CCF': [0x3F], 'HALT': [0x76],
    'EXX': [0xD9], 'DI': [0xF3], 'EI': [0xFB],
    'NEG': [0xED, 0x44], 'RETN': [0xED, 0x45], 'RETI': [0xED, 0x4D],
    'RRD': [0xED, 0x67], 'RLD': [0xED, 0x6F],
    'LDI': [0xED, 0xA0], 'CPI': [0xED, 0xA1], 'INI': [0xED, 0xA2], 'OUTI': [0xED, 0xA3],
    'LDD': [0xED, 0xA8], 'CPD': [0xED, 0xA9], 'IND': [0xED, 0xAA], 'OUTD': [0xED, 0xAB],
    'LDIR': [0xED, 0xB0], 'CPIR': [0xED, 0xB1], 'INIR': [0xED, 0xB2], 'OTIR': [0xED, 0xB3],
    'LDDR': [0xED, 0xB8], 'CPDR': [0xED, 0xB9], 'INDR': [0xED, 0xBA], 'OTDR': [0xED, 0xBB],
}


class Z80Asm(Asm):
    """Assembles Z80 source text (see Asm for the directives and numbers)."""

    def encode(self, mnem, arg, labels, final, pc, line):
        ops = [o.strip() for o in arg.split(',')] if arg else []
        ops_u = [o.upper() for o in ops]
        val = lambda s: self.value(s, labels, final)

        def b8(s):
            return bytes([val(s) & 0xFF])

        def w16(s):
            return struct.pack('<H', val(s) & 0xFFFF)

        def rel(s, size):
            off = val(s) - (pc + size)
            if final:
                assert -128 <= off <= 127, (line, off)
            return bytes([off & 0xFF])

        def is_ind(s):
            return s.startswith('(') and s.endswith(')')

        def ixy(s):
            """(prefix, displacement text) of `(IX+d)`/`(IY-d)`, or None."""
            u = s.upper()
            for reg, p in (('IX', 0xDD), ('IY', 0xFD)):
                if u.startswith('(' + reg) and u.endswith(')'):
                    inner = s[3:-1].strip()
                    if inner == '':
                        return p, '0'
                    return p, ('0' + inner if inner[0] in '+-' else inner)
            return None

        def half(s):
            """(prefix, register index) of IXH/IXL/IYH/IYL, or None."""
            m = {'IXH': (0xDD, 4), 'IXL': (0xDD, 5), 'IYH': (0xFD, 4), 'IYL': (0xFD, 5)}
            return m.get(s.upper())

        if mnem in Z80_IMPLIED and not ops:
            return bytes(Z80_IMPLIED[mnem])
        if mnem == '.OP':
            return bytes(val(o) & 0xFF for o in ops)
        if mnem == 'EX':
            if ops_u == ["AF", "AF'"]:
                return bytes([0x08])
            if ops_u == ['DE', 'HL']:
                return bytes([0xEB])
            if ops_u == ['(SP)', 'HL']:
                return bytes([0xE3])
            if ops_u == ['(SP)', 'IX']:
                return bytes([0xDD, 0xE3])
            if ops_u == ['(SP)', 'IY']:
                return bytes([0xFD, 0xE3])
        if mnem == 'IM':
            return bytes([0xED, {0: 0x46, 1: 0x56, 2: 0x5E}[val(ops[0])]])
        if mnem == 'OUT':
            if ops_u[0] == '(C)':
                if ops[1] == '0':
                    return bytes([0xED, 0x71])
                return bytes([0xED, 0x41 + Z80_R8.index(ops_u[1]) * 8])
            return bytes([0xD3]) + b8(ops[0][1:-1])
        if mnem == 'IN':
            if ops_u[1] == '(C)':
                return bytes([0xED, 0x40 + Z80_R8.index(ops_u[0]) * 8])
            return bytes([0xDB]) + b8(ops[1][1:-1])
        if mnem == 'LD':
            d, s = ops_u
            dx, sx = ixy(ops[0]), ixy(ops[1])
            dh, sh = half(d), half(s)
            if d in Z80_R8 and s in Z80_R8:
                return bytes([0x40 + Z80_R8.index(d) * 8 + Z80_R8.index(s)])
            if dx and s in Z80_R8:
                return bytes([dx[0], 0x70 + Z80_R8.index(s)]) + b8(dx[1])
            if dx:
                return bytes([dx[0], 0x36]) + b8(dx[1]) + b8(ops[1])
            if sx and d in Z80_R8:
                return bytes([sx[0], 0x46 + Z80_R8.index(d) * 8]) + b8(sx[1])
            if dh and (s in Z80_R8 or half(s)):
                si = half(s)[1] if half(s) else Z80_R8.index(s)
                return bytes([dh[0], 0x40 + dh[1] * 8 + si])
            if sh and d in Z80_R8:
                return bytes([sh[0], 0x40 + Z80_R8.index(d) * 8 + sh[1]])
            if dh:
                return bytes([dh[0], 0x06 + dh[1] * 8]) + b8(ops[1])
            table = {
                ('(BC)', 'A'): [0x02], ('(DE)', 'A'): [0x12], ('A', '(BC)'): [0x0A],
                ('A', '(DE)'): [0x1A], ('SP', 'HL'): [0xF9], ('SP', 'IX'): [0xDD, 0xF9],
                ('SP', 'IY'): [0xFD, 0xF9], ('I', 'A'): [0xED, 0x47], ('R', 'A'): [0xED, 0x4F],
                ('A', 'I'): [0xED, 0x57], ('A', 'R'): [0xED, 0x5F],
            }
            if (d, s) in table:
                return bytes(table[(d, s)])
            if d in Z80_R8 and not is_ind(s):
                return bytes([0x06 + Z80_R8.index(d) * 8]) + b8(ops[1])
            if d in ('IX', 'IY'):
                p = 0xDD if d == 'IX' else 0xFD
                if is_ind(s):
                    return bytes([p, 0x2A]) + w16(ops[1][1:-1])
                return bytes([p, 0x21]) + w16(ops[1])
            if s in ('IX', 'IY') and is_ind(d):
                return bytes([0xDD if s == 'IX' else 0xFD, 0x22]) + w16(ops[0][1:-1])
            if d in Z80_R16 and is_ind(s):
                if d == 'HL':
                    return bytes([0x2A]) + w16(ops[1][1:-1])
                return bytes([0xED, 0x4B + Z80_R16.index(d) * 16]) + w16(ops[1][1:-1])
            if s in Z80_R16 and is_ind(d):
                if s == 'HL':
                    return bytes([0x22]) + w16(ops[0][1:-1])
                return bytes([0xED, 0x43 + Z80_R16.index(s) * 16]) + w16(ops[0][1:-1])
            if d in Z80_R16:
                return bytes([0x01 + Z80_R16.index(d) * 16]) + w16(ops[1])
            if is_ind(d) and s == 'A':
                return bytes([0x32]) + w16(ops[0][1:-1])
            if d == 'A' and is_ind(s):
                return bytes([0x3A]) + w16(ops[1][1:-1])
            raise ValueError(line)
        if mnem in Z80_ALU:
            i = Z80_ALU.index(mnem)
            if len(ops) == 2 and ops_u[0] in ('HL', 'IX', 'IY'):
                if ops_u[0] == 'HL':
                    r = Z80_R16.index(ops_u[1])
                    if mnem == 'ADD':
                        return bytes([0x09 + r * 16])
                    return bytes([0xED, (0x4A if mnem == 'ADC' else 0x42) + r * 16])
                p = 0xDD if ops_u[0] == 'IX' else 0xFD
                src = ops_u[1]
                r = 2 if src in ('IX', 'IY') else Z80_R16.index(src)
                return bytes([p, 0x09 + r * 16])
            if len(ops) == 2:
                assert ops_u[0] == 'A', line
                ops, ops_u = ops[1:], ops_u[1:]
            o = ops[0]
            x, hx = ixy(o), half(o)
            if x:
                return bytes([x[0], 0x86 + i * 8]) + b8(x[1])
            if hx:
                return bytes([hx[0], 0x80 + i * 8 + hx[1]])
            if ops_u[0] in Z80_R8:
                return bytes([0x80 + i * 8 + Z80_R8.index(ops_u[0])])
            return bytes([0xC6 + i * 8]) + b8(o)
        if mnem in ('INC', 'DEC'):
            o = ops[0]
            u = ops_u[0]
            x, hx = ixy(o), half(o)
            if x:
                return bytes([x[0], 0x34 if mnem == 'INC' else 0x35]) + b8(x[1])
            if hx:
                return bytes([hx[0], (0x04 if mnem == 'INC' else 0x05) + hx[1] * 8])
            if u in ('IX', 'IY'):
                return bytes([0xDD if u == 'IX' else 0xFD, 0x23 if mnem == 'INC' else 0x2B])
            if u in Z80_R8:
                return bytes([(0x04 if mnem == 'INC' else 0x05) + Z80_R8.index(u) * 8])
            return bytes([(0x03 if mnem == 'INC' else 0x0B) + Z80_R16.index(u) * 16])
        if mnem in ('PUSH', 'POP'):
            base = 0xC5 if mnem == 'PUSH' else 0xC1
            if ops_u[0] in ('IX', 'IY'):
                return bytes([0xDD if ops_u[0] == 'IX' else 0xFD, base + 0x20])
            return bytes([base + Z80_R16_STACK.index(ops_u[0]) * 16])
        if mnem == 'JP':
            if ops_u[0] in ('(HL)', 'HL'):
                return bytes([0xE9])
            if ops_u[0] in ('(IX)', 'IX'):
                return bytes([0xDD, 0xE9])
            if ops_u[0] in ('(IY)', 'IY'):
                return bytes([0xFD, 0xE9])
            if len(ops) == 2:
                return bytes([0xC2 + Z80_CC.index(ops_u[0]) * 8]) + w16(ops[1])
            return bytes([0xC3]) + w16(ops[0])
        if mnem == 'JR':
            if len(ops) == 2:
                return bytes([0x20 + Z80_CC.index(ops_u[0]) * 8]) + rel(ops[1], 2)
            return bytes([0x18]) + rel(ops[0], 2)
        if mnem == 'DJNZ':
            return bytes([0x10]) + rel(ops[0], 2)
        if mnem == 'CALL':
            if len(ops) == 2:
                return bytes([0xC4 + Z80_CC.index(ops_u[0]) * 8]) + w16(ops[1])
            return bytes([0xCD]) + w16(ops[0])
        if mnem == 'RET':
            if ops:
                return bytes([0xC0 + Z80_CC.index(ops_u[0]) * 8])
            return bytes([0xC9])
        if mnem == 'RST':
            return bytes([0xC7 + (val(ops[0]) & 0x38)])
        if mnem in Z80_CB_SHIFTS or mnem in ('BIT', 'RES', 'SET'):
            if mnem in Z80_CB_SHIFTS:
                op = Z80_CB_SHIFTS.index(mnem) * 8
                o = ops[0]
            else:
                op = {'BIT': 0x40, 'RES': 0x80, 'SET': 0xC0}[mnem] + (val(ops[0]) & 7) * 8
                o = ops[1]
            x = ixy(o)
            if x:
                return bytes([x[0], 0xCB]) + b8(x[1]) + bytes([op + 6])
            return bytes([0xCB, op + Z80_R8.index(o.upper())])
        raise ValueError('unknown instruction: ' + line)


def ay_rel(table, at, target):
    """Store at `at` in `table` the AY format's pointer to `target`: a
    big-endian 16-bit offset from the pointer itself."""
    struct.pack_into('>h', table, at, target - at)


def ay_file(tracks, author, comment, version=0, first=0, extra=None):
    """An AY file. tracks: a list of dicts with name, sp, init, play,
    length (frames), hi, lo (the register values) and blocks
    [(addr, bytes)], or the raw bytes of a track's data (for broken ones).
    extra(out, layout) can patch the result."""
    out = bytearray(b'ZXAYEMUL' + bytes([version, 0, 0, 0]) + bytes(4)
                    + bytes([len(tracks) - 1, first]) + bytes(2))
    strings = {}

    def add_string(s):
        if s not in strings:
            strings[s] = len(out)
            out.extend(s.encode('latin-1') + b'\0')
        return strings[s]

    list_at = len(out)
    out.extend(bytes(4 * len(tracks)))
    ay_rel(out, 18, list_at)
    ay_rel(out, 12, add_string(author))
    ay_rel(out, 14, add_string(comment))
    layout = {}
    for i, t in enumerate(tracks):
        ay_rel(out, list_at + i * 4, add_string(t['name']))
        data_at = len(out)
        out.extend(bytes(14))
        ay_rel(out, list_at + i * 4 + 2, data_at)
        struct.pack_into('>HH', out, data_at + 4, t['length'], 0)
        out[data_at + 8] = t['hi']
        out[data_at + 9] = t['lo']
        points_at = len(out)
        out.extend(struct.pack('>HHH', t['sp'], t['init'], t['play']))
        ay_rel(out, data_at + 10, points_at)
        blocks_at = len(out)
        out.extend(bytes(6 * len(t['blocks']) + 2))
        ay_rel(out, data_at + 12, blocks_at)
        for j, (addr, code) in enumerate(t['blocks']):
            code_at = len(out)
            out.extend(code)
            struct.pack_into('>HH', out, blocks_at + j * 6, addr, len(code))
            ay_rel(out, blocks_at + j * 6 + 4, code_at)
        layout[i] = (data_at, points_at, blocks_at)
    if extra:
        extra(out, layout)
    return bytes(out)


# The Z80 exercise: most instructions, on every register, (HL) and
# (IX+d)/(IY+d), leaving a value at $C010 (and the stack as it found it)
# that the songs use, so a mistake in any of them changes the sound.
Z80_CPU_EXERCISE = '''
cpu_test:
    PUSH AF
    PUSH BC
    PUSH DE
    PUSH HL
    PUSH IX
    PUSH IY
    LD A,$5A
    LD B,$13
    LD C,$C4
    LD D,$7F
    LD E,$80
    LD HL,$C100
    LD (HL),$29
    ADD A,B
    ADC A,C
    SUB D
    SBC A,E
    AND $F7
    XOR H
    OR L
    CP (HL)
    ADD A,(HL)
    ADC A,$33
    SUB $11
    SBC A,(HL)
    XOR (HL)
    OR (HL)
    AND (HL)
    DAA
    LD (HL),A
    INC (HL)
    DEC (HL)
    INC A
    DEC B
    INC C
    DEC D
    INC E
    INC H
    DEC L
    LD B,A
    LD C,B
    LD D,C
    LD E,D
    LD H,$C1
    LD L,$02
    LD (HL),E
    LD A,(HL)
    NEG
    DAA
    CPL
    SCF
    CCF
    RLA
    RRA
    RLCA
    RRCA
    LD ($C101),A
    ; 16-bit
    LD BC,$1234
    LD DE,$FEDC
    LD HL,$0F0F
    ADD HL,BC
    ADC HL,DE
    SBC HL,BC
    ADD HL,HL
    ADD HL,SP
    INC BC
    DEC DE
    INC HL
    INC SP
    DEC SP
    LD ($C102),HL
    LD ($C104),BC
    LD ($C106),DE
    LD ($C108),SP
    LD HL,($C104)
    LD BC,($C106)
    LD DE,($C102)
    EX DE,HL
    EXX
    LD BC,$0101
    LD DE,$0202
    LD HL,$0303
    EXX
    EX AF,AF'
    LD A,$77
    EX AF,AF'
    ; IX and IY
    LD IX,$C120
    LD IY,$C140
    LD (IX+0),$11
    LD (IX+5),A
    LD (IY-3),$7E
    LD B,(IX+5)
    ADD A,(IX+0)
    ADC A,(IY-3)
    SUB (IX+5)
    SBC A,(IY-3)
    AND (IX+0)
    XOR (IY-3)
    OR (IX+5)
    CP (IY-3)
    INC (IX+0)
    DEC (IY-3)
    LD IXH,$12
    LD IXL,B
    LD IYL,IYH
    ADD A,IXH
    ADC A,IXL
    SUB IYH
    AND IYL
    XOR IXL
    OR IXH
    CP IYL
    INC IXH
    DEC IYL
    LD C,IXL
    ADD IX,BC
    ADD IY,DE
    ADD IX,IX
    ADD IY,SP
    INC IX
    DEC IY
    LD ($C110),IX
    LD IY,($C110)
    PUSH IX
    POP IY
    EX (SP),IX
    EX (SP),IY
    RLC (IX+0)
    RR (IY-3)
    SLA (IX+5)
    SRL (IY-3)
    SET 3,(IX+0)
    RES 0,(IY-3)
    BIT 7,(IX+5)
    LD A,(IX+0)
    XOR (IY-3)
    XOR (IX+5)
    LD ($C111),A
    ; the CB instructions
    LD A,$96
    LD B,$69
    RLC A
    RRC B
    RL A
    RR B
    SLA A
    SRA B
    SLL A
    SRL B
    LD HL,$C107
    LD (HL),$C3
    RLC (HL)
    RR (HL)
    SLA (HL)
    SRA (HL)
    SRL (HL)
    SLL (HL)
    RL (HL)
    RRC (HL)
    BIT 7,(HL)
    JR Z,ct_b0
    SET 0,(HL)
ct_b0:
    BIT 0,C
    JR NZ,ct_b1
    RES 7,(HL)
ct_b1:
    SET 3,D
    RES 1,C
    BIT 6,D
    RRD
    RLD
    LD A,(HL)
    XOR C
    XOR D
    LD E,A
    ; block instructions
    LD HL,$C100
    LD DE,$C180
    LD BC,$0008
    LDIR
    LD HL,$C187
    LD DE,$C197
    LD BC,$0004
    LDDR
    LD HL,$C180
    LD BC,$0010
    LD A,$29
    CPIR
    LD A,C
    XOR E
    LD E,A
    LD HL,$C18F
    LD BC,$0003
    LD A,$00
    CPDR
    LDI
    LDD
    CPI
    CPD
    LD A,C
    ADD A,E
    LD E,A
    ; flags into the result
    PUSH AF
    POP BC
    LD A,C
    XOR E
    LD E,A
    LD A,I
    LD A,$3C
    LD I,A
    LD A,I
    ADD A,E
    LD E,A
    ; jumps, calls and returns, with every condition
    CALL ct_sub
    CALL Z,ct_sub
    CALL NZ,ct_sub
    CALL C,ct_sub
    CALL NC,ct_sub
    CALL PE,ct_sub
    CALL PO,ct_sub
    CALL M,ct_sub
    CALL P,ct_sub
    LD HL,ct_jp
    JP (HL)
ct_jp_back:
    LD IX,ct_jp2
    JP (IX)
ct_jp2_back:
    XOR A
    JP Z,ct_jz
    INC E
ct_jz:
    JP PE,ct_jpe
    INC E
ct_jpe:
    JP M,ct_jm
    INC E
ct_jm:
    LD B,5
ct_djnz:
    INC E
    DJNZ ct_djnz
    RST $28
    LD A,E
    LD ($C010),A
    POP IY
    POP IX
    POP HL
    POP DE
    POP BC
    POP AF
    RET
ct_jp:
    INC E
    JP ct_jp_back
ct_jp2:
    INC E
    JP ct_jp2_back
ct_sub:
    INC E
    LD A,E
    AND $03
    RET Z
    CP $02
    RET C
    RET PO
    RET PE
'''

AY_WRITE = '''
ay_write:            ; register A <- E, through the Spectrum's ports
    LD BC,$FFFD
    OUT (C),A
    LD B,$BF
    OUT (C),E
    RET
'''


def ay_spectrum():
    a = Z80Asm(0x8000)
    a('''
init:
    LD ($C000),A     ; (the track's A: the hi register value)
    CALL cpu_test
    LD HL,regs
    XOR A
init_regs:
    LD E,(HL)
    PUSH AF
    CALL ay_write
    POP AF
    INC HL
    INC A
    CP 14
    JR NZ,init_regs
    XOR A
    LD ($C001),A
    RET

play:
    LD HL,$C001
    INC (HL)
    LD A,(HL)
    LD D,A
    ; tone A from the exercise's value and the frame
    LD A,($C010)
    ADD A,D
    LD E,A
    XOR A
    CALL ay_write
    ; tone B, a slide
    LD A,D
    RLCA
    RLCA
    LD E,A
    LD A,2
    CALL ay_write
    ; the envelope shape, every 8 frames (all 16 of them)
    LD A,D
    AND $07
    JR NZ,play_e
    LD A,D
    RRCA
    RRCA
    RRCA
    AND $0F
    LD E,A
    LD A,13
    CALL ay_write
play_e:
    ; the noise period and the mixer, every 16 frames
    LD A,D
    AND $0F
    JR NZ,play_m
    LD A,D
    RRCA
    AND $1F
    LD E,A
    LD A,6
    CALL ay_write
    LD A,D
    RRCA
    RRCA
    RRCA
    RRCA
    AND $3F
    LD E,A
    LD A,7
    CALL ay_write
play_m:
    RET
regs:
    .db $40, $01, $80, $02, $20, $00, $07, $38, $0F, $10, $0C, $40, $00, $0E
''')
    a(AY_WRITE)
    a('''
beeper_init:
    XOR A
    LD ($C001),A
    RET
beeper_play:
    LD HL,$C001
    INC (HL)
    LD A,(HL)
    AND $07
    ADD A,4
    LD D,A           ; half periods per frame
    LD A,(HL)
    AND $3F
    ADD A,$20
    LD C,A           ; loop count of each half period
beeper_loop:
    LD A,($C002)
    XOR $10
    LD ($C002),A
    OUT ($FE),A
    LD B,C
beeper_wait:
    DJNZ beeper_wait
    DEC D
    JR NZ,beeper_loop
    RET

passive_init:
    ; IM 2 with the vector table's entry at $C2FF
    LD A,$C2
    LD I,A
    LD HL,isr
    LD ($C2FF),HL
    LD A,7
    LD E,$3E
    CALL ay_write
    LD A,8
    LD E,$0F
    CALL ay_write
    XOR A
    LD ($C001),A
    RET
isr:
    PUSH AF
    PUSH BC
    PUSH DE
    PUSH HL
    LD HL,$C001
    INC (HL)
    LD A,(HL)
    ADD A,A
    ADD A,A
    LD E,A
    XOR A
    CALL ay_write
    LD A,(HL)
    AND $0F
    LD E,A
    LD A,8
    CALL ay_write
    POP HL
    POP DE
    POP BC
    POP AF
    RET
''')
    a(Z80_CPU_EXERCISE)
    code = a.assemble()
    L = a.labels
    rst = Z80Asm(0x0028)
    rst('''
    INC E
    RET
''')
    tracks = [
        dict(name='Tones', sp=0xF000, init=L['init'], play=L['play'], length=500, hi=0x00,
             lo=0x55, blocks=[(0x8000, code), (0x0028, rst.assemble())]),
        dict(name='Beeper', sp=0xF000, init=L['beeper_init'], play=L['beeper_play'], length=0,
             hi=0x01, lo=0x00, blocks=[(0x8000, code)]),
        dict(name='Passive', sp=0xF000, init=L['passive_init'], play=0, length=300, hi=0x02,
             lo=0xAA, blocks=[(0x8000, code)]),
    ]
    write('ay_spectrum.ay', ay_file(tracks, 'gen_gme_testdata.py', 'AY Test'))


def ay_cpc():
    # the Amstrad CPC's ports (which switch the emulator to the CPC's clock)
    a = Z80Asm(0x4000)
    a('''
init:
    LD HL,regs
    LD D,0
init_regs:
    LD A,(HL)
    CALL cpc_write
    INC HL
    INC D
    LD A,D
    CP 14
    JR NZ,init_regs
    RET
play:
    LD HL,$C001
    INC (HL)
    LD A,(HL)
    LD D,0
    CALL cpc_write
    LD A,(HL)
    AND $0F
    LD D,9
    CALL cpc_write
    RET
cpc_write:           ; register D <- A
    PUSH AF
    LD C,0
    LD B,$F4
    OUT (C),D
    LD B,$F6
    LD A,$C0
    OUT (C),A
    LD B,$F4
    POP AF
    OUT (C),A
    LD B,$F6
    LD A,$80
    OUT (C),A
    RET
regs:
    .db $80, $00, $40, $01, $00, $00, $00, $3C, $0C, $0A, $00, $00, $00, $00
''')
    code = a.assemble()
    L = a.labels
    tracks = [dict(name='CPC', sp=0xBFF0, init=L['init'], play=L['play'], length=100, hi=0,
                   lo=0, blocks=[(0x4000, code)])]
    write('ay_cpc.ay', ay_file(tracks, 'Someone', '', version=2))


def ay_odd():
    # an unknown version; blocks too long for memory and for the file, a
    # block with a null offset; a track without data blocks; strings that
    # run to the end of the file; code with the instructions the CPU
    # emulator warns about and that wraps around memory
    a = Z80Asm(0xFFE8)
    a('''
    LD A,8
    LD BC,$FFFD
    OUT (C),A
    LD A,$0F
    LD B,$BF
    OUT (C),A
    LD A,R
    .op $ED, $00     ; an ED opcode the emulator doesn't support
    .op $DD, $00     ; a DD prefix on an instruction without IX
    .op $DD, $CB, $00, $00
    NOP
''')
    code = a.assemble()
    tracks = [
        dict(name='Wrap', sp=0xF000, init=0xFFE8, play=0, length=0, hi=0, lo=0,
             blocks=[(0xFFE8, code)]),
        dict(name='Long', sp=0xF000, init=0, play=0x8000, length=10, hi=0, lo=0,
             blocks=[(0x8000, bytes([0xC9]) * 4), (0xFFF8, bytes([0xC9]) * 32),
                     (0x9000, bytes([0x00]) * 8)]),
        dict(name='Null', sp=0xF000, init=0x8000, play=0, length=10, hi=0, lo=0,
             blocks=[(0x8000, bytes([0x3E, 0x07, 0xC9]))]),
    ]

    def extra(out, layout):
        # track 2's block: a null offset
        data_at, points_at, blocks_at = layout[2]
        struct.pack_into('>h', out, blocks_at + 4, 0)
        # track 1's last block runs past the end of the file
        data_at, points_at, blocks_at = layout[1]
        struct.pack_into('>H', out, blocks_at + 12 + 2, 0x7000)
        # the comment runs on to the end of the file
        n = len(out)
        out.extend(b'no terminator')
        ay_rel(out, 14, n)

    write('ay_odd.ay', ay_file(tracks, '  Odd  ', '', version=3, extra=extra))

    # more tracks than the file has room for in its track list
    data = bytearray(ay_file(tracks[2:], 'x', 'y'))
    data[16] = 200
    write('ay_badtracks.ay', bytes(data))


def kss_header(tag, load_addr, load_size, init, play, first_bank=0, bank_mode=0, extra=0,
               device=0):
    return tag + struct.pack('<HHHHBBBB', load_addr, load_size, init, play, first_bank,
                             bank_mode, extra, device)


def kss_scc():
    # the MSX's PSG (through its ports and the BIOS) and Konami's SCC, two
    # 8K banks switched by play
    a = Z80Asm(0x4000)
    a('''
    JP init
    JP play
init:
    LD ($C000),A
    CALL cpu_test
    ; the PSG, through the BIOS's WRTPSG
    LD HL,psg
    LD D,0
init_psg:
    LD A,D
    LD E,(HL)
    CALL $0093
    INC HL
    INC D
    LD A,D
    CP 14
    JR NZ,init_psg
    ; the SCC's waves (the fifth shares the fourth's)
    LD HL,$9800
    LD B,$80
    LD C,0
init_wave:
    LD A,C
    ADD A,A
    ADD A,A
    ADD A,A
    XOR B
    LD (HL),A
    INC HL
    INC C
    DJNZ init_wave
    LD HL,scc
    LD DE,$9880
    LD BC,16
    LDIR
    XOR A
    LD ($C001),A
    RET
play:
    LD HL,$C001
    INC (HL)
    LD A,(HL)
    LD B,A
    ; bank 2 or 3 at $A000, then a byte of it as SCC frequency 1
    AND 1
    ADD A,2
    LD ($B000),A
    LD A,($A000)
    ADD A,B
    LD ($9880),A
    ; and bank 2 at $8000 every 16 frames (OUT $FE), 3 (LD ($9000)) otherwise
    LD A,B
    AND $0F
    JR NZ,play_b
    LD A,2
    OUT ($FE),A
    JR play_c
play_b:
    LD A,3
    LD ($9000),A
play_c:
    LD A,($8000)
    LD ($9882),A
    ; the PSG's tone A through its ports, and its envelope shape
    LD A,0
    OUT ($A0),A
    LD A,B
    ADD A,A
    LD HL,$C000
    ADD A,(HL)       ; (the track)
    OUT ($A1),A
    LD A,B
    AND $07
    RET NZ
    LD A,13
    OUT ($A0),A
    LD A,B
    RRCA
    RRCA
    RRCA
    OUT ($A1),A
    ; the SCC's enables
    LD A,B
    RRCA
    RRCA
    RRCA
    RRCA
    OR $11
    LD ($988F),A
    RET
psg:
    .db $80, $01, $00, $02, $40, $00, $1F, $30, $0D, $10, $00, $00, $08, $0C
scc:
    .dw $0123, $0234, $0155, $0080, $0300
    .db $0C, $0A, $0F, $08, $0E, $1F
''')
    a(Z80_CPU_EXERCISE)
    code = a.assemble()
    # (bank 2 is short, read past its end as zeros; bank 3 isn't there:
    # it maps RAM)
    bank2 = bytes([0x40]) + bytes(range(1, 64))
    data = code + bank2
    h = kss_header(b'KSCC', 0x4000, len(code), 0x4000, 0x4003, first_bank=2, bank_mode=0x80 | 2)
    write('kss_scc.kss', h + data)


def kss_sms():
    # the Sega Master System's SN76489 (and the Game Gear's stereo), at the
    # PAL rate; a KSSX header with its extra header
    a = Z80Asm(0x0100)
    a('''
init:
    LD ($C000),A
    LD HL,sn
    LD B,sn_end-sn
init_sn:
    LD A,(HL)
    OUT ($7F),A
    INC HL
    DJNZ init_sn
    LD A,$F3
    OUT ($06),A
    RET
play:
    LD HL,$C001
    INC (HL)
    LD A,(HL)
    AND $0F
    OR $80
    OUT ($7F),A      ; tone 1, low bits
    LD A,(HL)
    RRCA
    RRCA
    RRCA
    AND $3F
    OUT ($7F),A      ; tone 1, high bits
    LD A,(HL)
    AND $07
    OR $E0
    OUT ($7E),A      ; noise mode
    LD A,(HL)
    RRCA
    AND $0F
    OR $F0
    OUT ($7F),A      ; noise volume
    LD A,(HL)
    AND $1F
    RET NZ
    LD A,(HL)
    OUT ($06),A      ; Game Gear stereo
    RET
sn:
    .db $8E, $05, $90, $A0, $0A, $B4, $C4, $12, $D8, $E4, $F6
sn_end:
''')
    code = a.assemble()
    ext = struct.pack('<IIHHBBBB', len(code), 0, 0, 3, 0, 0, 0, 0)
    for device, name in ((0x42, 'kss_sms.kss'), (0x46, 'kss_gg.kss')):
        h = kss_header(b'KSSX', 0x0100, len(code), 0x0100, a.labels['play'], extra=0x10,
                       device=device)
        write(name, h + ext + code)


def kss_odd():
    # header warnings (extra header bytes, unknown and FM device flags in a
    # KSCC header, a KSSX extra header longer than 16 bytes), a load size
    # past the file, banks missing, code that runs to the end of memory
    # and stops at the idle address, port reads
    a = Z80Asm(0xFFE0)
    a('''
init:
    IN A,($A2)
    LD B,A
    LD A,8
    LD E,$0F
    CALL $0093
    CALL $0096       ; RDPSG
    LD A,$55
    LD ($9000),A     ; a bank that isn't there: RAM
    LD A,($8000)
    LD ($9800),A
    LD A,$1F
    LD ($988F),A
    NOP
    NOP
''')
    code = a.assemble()
    h = kss_header(b'KSCC', 0xFFE0, 0x400, 0xFFE0, 0xFFE0, bank_mode=4, extra=3, device=0xF9)
    write('kss_odd.kss', h + code)
    ext = bytes(0x20)
    h = kss_header(b'KSSX', 0xFFE0, len(code), 0xFFE0, 0, extra=0x20, device=0x01)
    write('kss_ext.kss', h + ext + code)


# ---------------------------------------------------------------------------
# A HuC6280 (the PC Engine's 65C02 with its own instructions) on the 6502
# assembler: the 65C02's (zp) mode and instructions, and the HuC6280's
# swaps, clears, VDP stores, mapping registers, TST, RMB/SMB, BBR/BBS and
# block transfers (`TII src,dst,len`).

HUC_OPS = {k: dict(v) for k, v in OPS_6502.items() if k not in ('LAX', 'SAX', 'SBX')}
for _m, _op in (('ORA', 0x12), ('AND', 0x32), ('EOR', 0x52), ('ADC', 0x72), ('STA', 0x92),
                ('LDA', 0xB2), ('CMP', 0xD2), ('SBC', 0xF2)):
    HUC_OPS[_m]['zpind'] = _op
HUC_OPS['BIT'].update({'imm': 0x89, 'zpx': 0x34, 'absx': 0x3C})
HUC_OPS['INC']['acc'] = 0x1A
HUC_OPS['DEC']['acc'] = 0x3A
HUC_OPS['JMP']['indabsx'] = 0x7C
HUC_OPS.update({
    'BRA': {'rel': 0x80}, 'BSR': {'rel': 0x44},
    'PHX': {'imp': 0xDA}, 'PHY': {'imp': 0x5A}, 'PLX': {'imp': 0xFA}, 'PLY': {'imp': 0x7A},
    'STZ': {'zp': 0x64, 'zpx': 0x74, 'abs': 0x9C, 'absx': 0x9E},
    'TSB': {'zp': 0x04, 'abs': 0x0C}, 'TRB': {'zp': 0x14, 'abs': 0x1C},
    'SXY': {'imp': 0x02}, 'SAX': {'imp': 0x22}, 'SAY': {'imp': 0x42},
    'CLA': {'imp': 0x62}, 'CLX': {'imp': 0x82}, 'CLY': {'imp': 0xC2},
    'CSL': {'imp': 0x54}, 'CSH': {'imp': 0xD4}, 'SET': {'imp': 0xF4},
    'ST0': {'imm': 0x03}, 'ST1': {'imm': 0x13}, 'ST2': {'imm': 0x23},
    'TAM': {'imm': 0x53}, 'TMA': {'imm': 0x43},
    'TST': {'zp': 0x83, 'abs': 0x93, 'zpx': 0xA3, 'absx': 0xB3},
})
for _n in range(8):
    HUC_OPS['RMB%d' % _n] = {'zp': 0x07 + _n * 0x10}
    HUC_OPS['SMB%d' % _n] = {'zp': 0x87 + _n * 0x10}
HUC_BLOCK = {'TII': 0x73, 'TDD': 0xC3, 'TIN': 0xD3, 'TIA': 0xE3, 'TAI': 0xF3}
SIZES['zpind'] = 2
SIZES['indabsx'] = 3


class HucAsm(Asm):
    """Assembles HuC6280 source text (see Asm)."""

    def __init__(self, org):
        super().__init__(org, HUC_OPS)

    def mode(self, mnem, arg, labels, final):
        modes = self.ops[mnem]
        u = arg.upper()
        if mnem == 'JMP' and arg.startswith('(') and u.endswith(',X)'):
            return 'indabsx', self.value(arg[1:-3], labels, final)
        if 'zpind' in modes and arg.startswith('(') and not u.endswith(',X)') \
                and not u.endswith('),Y'):
            return 'zpind', self.value(arg[1:-1], labels, final)
        return super().mode(mnem, arg, labels, final)

    def encode(self, mnem, arg, labels, final, pc, line):
        val = lambda s: self.value(s, labels, final)
        if mnem in HUC_BLOCK:
            src, dst, n = [x.strip() for x in arg.split(',')]
            return bytes([HUC_BLOCK[mnem]]) + struct.pack('<HHH', val(src) & 0xFFFF,
                                                          val(dst) & 0xFFFF, val(n) & 0xFFFF)
        if len(mnem) == 4 and mnem[:3] in ('BBR', 'BBS'):
            zp, target = [x.strip() for x in arg.split(',')]
            op = (0x0F if mnem[2] == 'R' else 0x8F) + int(mnem[3]) * 0x10
            off = val(target) - (pc + 3)
            if final:
                assert -128 <= off <= 127, (line, off)
            return bytes([op, val(zp) & 0xFF, off & 0xFF])
        if mnem == 'TST':
            imm, rest = [x.strip() for x in arg.split(',', 1)]
            assert imm.startswith('#'), line
            m, v = self.mode(mnem, rest, labels, final)
            out = bytes([self.ops[mnem][m], val(imm[1:]) & 0xFF])
            return out + (bytes([v & 0xFF]) if m in ('zp', 'zpx') else struct.pack('<H', v & 0xFFFF))
        return super().encode(mnem, arg, labels, final, pc, line)


# ---------------------------------------------------------------------------
# HES

def hes_header(init, banks, size, addr, version=0, data_tag=b'DATA', unused=bytes(4), first=0):
    h = b'HESM' + bytes([version, first]) + struct.pack('<H', init) + bytes(banks)
    h += data_tag + struct.pack('<II', size, addr) + unused
    assert len(h) == 0x20
    return h


def hes_fields(*fields, size=0x20):
    """The text fields (each `size` bytes, zero-padded)."""
    out = b''
    for f in fields:
        b = f.encode('latin-1') if isinstance(f, str) else f
        out += b + bytes(size - len(b))
    return out


# The HuC6280 exercise: most instructions and addressing modes, the
# HuC6280's own among them, leaving a value in $10 (zero page, in RAM at
# $2000) and the stack as it found it, which the songs use, so a mistake in
# any of them changes the sound. It uses BRK (see brk_irq).
HUC_CPU_EXERCISE = '''
cpu_test:
    LDA #$5A
    STA $20
    STZ $21
    LDX #$07
ct_loop:
    LDA $20
    ASL A
    ROL $21
    BCC ct_nc
    EOR #$1D
ct_nc:
    STA $20
    CLC
    ADC $21
    STA $30,X
    SEC
    SBC #$37
    BVS ct_ov
    ORA #$01
ct_ov:
    DEX
    BPL ct_loop
    ; indirect modes, (zp) among them
    LDA #$30
    STA $40
    LDA #$20         ; $2030: zero page $30, through page 1
    STA $41
    LDY #$03
    LDA ($40),Y
    LDX #$02
    EOR ($3E,X)
    ADC ($40)
    STA $22
    ORA ($40)
    AND ($40)
    SEC
    SBC ($40)
    CMP ($40)
    STA ($40)
    EOR ($40)
    STA $23
    LDA ($40)
    STA ($40),Y
    STA ($3E,X)
    ; swaps and clears
    LDX #$11
    LDY #$22
    LDA #$33
    SXY
    SAX
    SAY
    STX $24
    STY $25
    CLA
    CLX
    CLY
    INC A
    INC A
    DEC A
    CLC
    ADC $24
    ADC $25
    STA $26
    ; the stack
    LDX #$C3
    LDY #$3C
    PHX
    PHY
    PLX
    PLY
    STX $27
    TYA
    EOR $27
    STA $27
    PHP
    SED
    CLC
    LDA #$19
    ADC #$28         ; (in binary: decimal mode isn't emulated)
    CLD
    PLP
    STA $28
    ; BIT, TST, TSB and TRB
    LDA #$C0
    STA $2222
    BIT #$40
    BIT $20
    LDX #$02
    BIT $20,X
    BIT $2220,X
    PHP
    PLA
    STA $29
    TST #$81,$20
    TST #$81,$20,X
    TST #$C3,$2222
    TST #$C3,$2220,X
    PHP
    PLA
    EOR $29
    STA $29
    LDA #$0F
    TSB $28
    TRB $27
    LDA #$3C
    TSB $2222
    TRB $2222
    ; RMB, SMB, BBR and BBS
    LDA #$A5
    STA $2A
    RMB0 $2A
    SMB1 $2A
    RMB2 $2A
    SMB3 $2A
    RMB4 $2A
    SMB5 $2A
    RMB6 $2A
    SMB7 $2A
    BBR0 $2A,ct_b0
    INC $2A
ct_b0:
    BBS1 $2A,ct_b1
    INC $2A
ct_b1:
    BBS0 $2A,ct_b2
    INC $2A
ct_b2:
    BBR7 $2A,ct_b3
    INC $2A
ct_b3:
    ; STZ
    LDA #$77
    STA $2B
    STA $2C
    STA $2224
    STA $2225
    STZ $2B
    LDX #$01
    STZ $2B,X
    STZ $2224
    STZ $2224,X
    LDA $2224
    ORA $2C
    ORA #$80
    STA $2B
    ; read-modify-write
    LDA #$81
    STA $2200
    STA $2205
    ASL $2200
    LSR $2205
    LDX #$05
    ROL $2200,X
    ROR $2200,X
    INC $2200,X
    DEC $2200
    LSR $2200,X
    ASL $2200,X
    DEC $2200,X
    ASL $30
    ROL $31
    LSR $32,X
    ROR $33
    INC $34,X
    DEC $35
    ASL $30,X
    ROL $31,X
    ROR $32,X
    DEC $33,X
    LSR $34
    INC $35
    LDA $2200
    CLC
    ADC $2205
    ADC $2204,X
    ADC $30
    ADC $37
    LDY #$02
    ADC $2201,Y
    EOR $2203,X
    AND $2203,Y
    ORA $36
    SBC $2205
    CMP $2205,X
    CMP $2203,Y
    STA $2D
    ; compares
    LDA $22
    CMP $23
    BCS ct_ge
    INC $2D
ct_ge:
    LDX $25
    CPX #$40
    BNE ct_ne
    DEC $2D
ct_ne:
    LDY $26
    CPY $2205
    BEQ ct_eq
    INY
ct_eq:
    CPX $24
    BCC ct_lt
    INY
ct_lt:
    CPY $2200
    ROL A
    CPX $2200
    ROR A
    CPY $28
    ROL A
    STA $2E
    STY $2F
    ; loads of every kind
    LDX $2B
    LDY $2C
    LDA $2200,Y
    LDX $2202,Y
    LDY $2203,X
    LDX $24,Y
    LDY $25,X
    LDA $24,X
    STX $2226
    STY $2227
    LDY #$04
    STX $26,Y
    STY $27,X
    STA $2228,Y
    EOR $2226
    EOR $2227
    EOR $2E
    STA $2E
    ; block transfers
    TII ct_table,$2240,8
    TDD ct_table+7,$224F,8
    TIN ct_table,$2250,4
    TIA ct_table,$2258,8
    TAI ct_table,$2260,8
    LDA $2240
    EOR $224B
    EOR $2250
    EOR $2259
    EOR $2263
    EOR $2266
    STA $2C
    ; the mapping registers: SGX RAM into page 2, and back
    TMA #$04
    PHA
    LDA #$F9
    TAM #$04
    LDA #$5C
    STA $4000
    LDA #$FA
    TAM #$04
    LDA #$A3
    STA $4000
    LDA #$F9
    TAM #$04
    LDA $4000
    EOR $2C
    STA $2C
    PLA
    TAM #$04
    TMA #$06
    EOR $2C
    STA $2C
    ; JMP (abs,X), JMP (abs), BSR, BRA, BRK
    LDX #$02
    JMP (ct_vec,X)
ct_back:
    BSR ct_sub
    BRA ct_fin
ct_sub:
    LDA #$7F
    CLC
    ADC #$01
    BVC ct_sub_done
    INC $27
ct_sub_done:
    CLV
    RTS
ct_fin:
    BRK
    .db $00
    LDA $20
    EOR $22
    EOR $23
    EOR $26
    EOR $27
    EOR $28
    EOR $29
    EOR $2A
    EOR $2B
    EOR $2C
    EOR $2D
    EOR $2E
    EOR $2F
    EOR $0F
    STA $10
    RTS
ct_jmp:
    LDA #<ct_back
    STA $2230
    LDA #>ct_back
    STA $2231
    JMP ($2230)
ct_vec:
    .dw ct_back, ct_jmp
ct_table:
    .db $01, $12, $23, $34, $45, $56, $67, $78
brk_irq:
    INC $0F
    RTI
'''


def hes_psg():
    # The PSG: four wave channels and two that can play noise, through
    # their waveform writes and DDA, the balances, the timer and VDP
    # interrupts, the ADPCM's registers. Track 0 runs on the timer, track 1
    # also on the VDP's vertical blank, the others (track 2 and up) change
    # the balances and the waves' periods as they go.
    a = HucAsm(0xF800)
    a('''
    .org $F880
init:
    SEI
    CSH
    STA $00          ; the track
    STZ $01
    STZ $02
    STZ $0F
    JSR cpu_test
    LDA #$FF
    STA $0801        ; global balance
    CLX
init_ch:
    STX $0800
    LDA #$40
    STA $0804        ; DDA on (write mode reset)
    STZ $0804
    LDA periods_lo,X
    EOR $10
    STA $0802
    LDA periods_hi,X
    STA $0803
    LDY #$00
init_wave:
    TYA
    ASL A
    STA $03
    TXA
    ASL A
    ASL A
    EOR $03
    AND #$1F
    STA $0806
    INY
    CPY #32
    BNE init_wave
    LDA balances,X
    STA $0805
    LDA controls,X
    STA $0804
    INX
    CPX #6
    BNE init_ch
    ; channel 0's wave again, by block transfer
    STZ $0800
    STZ $0804
    TIN wave,$0806,32
    LDA #$9F
    STA $0804
    ; channel 4 noise, channel 5 DDA
    LDA #$04
    STA $0800
    LDA #$9A
    STA $0807
    LDA #$05
    STA $0800
    LDA #$DC
    STA $0804
    LDA #$10
    STA $0806
    LDA #$80
    STA $0809        ; the LFO (not emulated)
    ; the ADPCM: 16 bytes at 0, played at 16 kHz with a fade
    STZ $1808
    STZ $1809
    LDA #$03
    STA $180D
    LDY #$10
init_adpcm:
    TYA
    EOR #$9C
    STA $180A
    DEY
    BNE init_adpcm
    LDA #$10
    STA $1808
    LDA #$18
    STA $180D
    LDA #$0E
    STA $180E
    LDA #$0A
    STA $180F
    LDA #$58
    STA $180D
    ; the timer, and on track 1 the VDP
    LDA #$1F
    STA $0C00
    LDA #$01
    STA $0C01
    LDA $00
    CMP #$01
    BNE init_novdp
    ST0 #$05
    ST1 #$08
    ST2 #$00
    LDA #$01
    BRA init_irq
init_novdp:
    LDA #$03
init_irq:
    STA $1402
    CLI
    RTS

timer_irq:
    PHA
    PHX
    PHY
    STA $1403        ; acknowledge
    INC $01
    LDA $01
    AND #$07
    TAX
    STZ $0800
    LDA notes,X
    STA $0802
    LDA #$05
    STA $0800
    LDA $01
    AND #$1F
    STA $0806        ; DDA sample
    LDA $0C00        ; the timer's count
    STA $04
    LDA $180C        ; ADPCM status
    EOR $04
    STA $05
    LDA $180A        ; ADPCM data
    EOR $05
    AND #$1F
    ORA #$80
    LDY #$04
    STY $0800
    STA $0807        ; the noise's frequency
    LDA $00
    CMP #$02
    BCC timer_done
    ; tracks 2 and up: balances and periods
    LDA $01
    AND #$0F
    BNE timer_done
    LDA $01
    STA $0801        ; global balance
    LDA #$02
    STA $0800
    LDA $01
    LSR A
    LSR A
    LSR A
    LSR A
    AND #$07
    STA $0802        ; a period too short to sound, or none
    STZ $0803
    LDA $01
    AND #$30
    BNE timer_bal
    LDA #$1F
    STA $0804        ; DDA off: the wave plays from its start again
timer_bal:
    LDA $01
    STA $0805
    LDA #$03
    STA $0800
    LDA $01
    AND #$40
    ORA #$9C
    STA $0804        ; DDA on and off
timer_done:
    PLY
    PLX
    PLA
    RTI

vdp_irq:
    PHA
    LDA $0000        ; acknowledge (the status)
    STA $06
    INC $02
    LDA #$01
    STA $0800
    LDA $02
    AND #$1F
    ORA #$80
    STA $0804
    LDA $1403        ; the interrupt status
    STA $07
    PLA
    RTI

periods_lo:
    .db $FE, $7F, $54, $3F, $2A, $11
periods_hi:
    .db $01, $01, $01, $01, $00, $00
balances:
    .db $FF, $F8, $8F, $CC, $FF, $AA
controls:
    .db $9F, $98, $9C, $93, $9F, $DC
notes:
    .db $FE, $E2, $C9, $BE, $A9, $97, $86, $7F
wave:
    .db $00, $04, $08, $0C, $10, $14, $18, $1C, $1F, $1F, $1F, $1F, $1F, $1F, $1F, $1F
    .db $1F, $1B, $17, $13, $0F, $0B, $07, $03, $00, $00, $00, $00, $00, $00, $00, $00
''')
    a(HUC_CPU_EXERCISE)
    a('''
    .org $FFF6
    .dw brk_irq, vdp_irq, timer_irq, init, init
''')
    code = bytearray(a.assemble())
    code[0x20:0x80] = hes_fields('PSG Test', 'gen_gme_testdata.py', '2026 Public Domain')
    banks = [0xFF, 0xF8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
    h = hes_header(a.labels['init'], banks, len(code), 0x1800)
    write('hes_psg.hes', h + code)


def hes_odd():
    # header warnings (an unknown version, the DATA tag missing, unknown
    # header data, an address past the 1 MB ROM, a size past the file),
    # fields of 0x30 bytes, the instructions that are illegal or not
    # emulated, writes to ROM pages (which go to the I/O page), the I/O
    # page mapped elsewhere, code at the end of RAM reading past it, code
    # running past the end of memory (track 1), interrupts enabled and
    # disabled by PLP and RTI, a scanline interrupt (a warning), a block
    # transfer to the PSG, banks with nothing in them
    a = HucAsm(0x7C00)
    a('''
    .org $7CC0
init:
    STA $00
    STZ $FF          ; (BBS7 on the unmapped page's $FF bytes doesn't branch)
    STZ $0804
    TIN odd_wave,$0806,32
    LDA #$01
    STA $0800
    STZ $0804
    TIN odd_wave+3,$0806,32
    STZ $0800
    LDA #$9F
    STA $0804
    LDA #$A0
    STA $0802
    LDA #$FF
    STA $0805
    LDA $00
    BEQ odd0
    CMP #$01
    BNE odd2
    ; track 1: off the end of memory, through the unmapped page (in
    ; page 7 too)
    LDA #$80
    TAM #$80
    JMP $FFF0
odd2:
    ; tracks 2 and up: a scanline interrupt, the timer read when it is
    ; disabled, the PSG by block transfer, illegal instructions
    ST0 #$05
    ST1 #$0C
    LDA $0C00
    STA $0802
    TIA odd_wave,$0806,64
    CSL
    SET
    .db $0B, $1B, $33
    LDA #$01
    STA $0C01
    LDA #$00
    STA $0C00
    STZ $1402
    LDA #$00
    PHA
    PLP              ; I clear: interrupts on
    LDA #$04
    PHA
    PLP              ; I set: off
    LDA #>odd2_ret
    PHA
    LDA #<odd2_ret
    PHA
    LDA #$00
    PHA
    RTI              ; I clear again
odd2_ret:
    SEI
    LDA $0C00
    STA $0802
    LDA #$01
    STA $0803
    CLI
    RTS
odd0:
    ; a write to a ROM page: the I/O page's PSG
    LDA #$01
    STA $6800
    LDA #$9C
    STA $6804
    LDA #$C0
    STA $6802
    ; the I/O page also in page 4, and RAM in page 0: a write to page 4
    ; is dropped
    LDA #$FF
    TAM #$10
    LDA #$F8
    TAM #$01
    LDA #$55
    STA $8800
    LDA #$FF
    TAM #$01
    ; code at the end of RAM: LDA abs with its operand past it
    LDA #$AD
    STA $3FFF
    LDA #$F9
    TAM #$04         ; SGX RAM after it
    LDA #$60         ; RTS
    STA $4002
    JSR $3FFF
    STA $0802
    ; banks that aren't there
    LDA #$80
    TAM #$10
    LDA $8000
    STA $0803
    LDA #$7F
    TAM #$10
    LDA $8000
    ORA #$01
    STA $0803
    LDA #$F8
    TAM #$10
    CLI
    RTS
timer_irq:
vdp_irq:
brk_irq:
    STA $1403
    LDA $0000
    INC $01
    LDA $01
    STA $0806
    RTI
odd_wave:
    .db $00, $1F, $02, $1D, $04, $1B, $06, $19, $08, $17, $0A, $15, $0C, $13, $0E, $11
    .org $7FF6
    .dw brk_irq, vdp_irq, timer_irq, init, init
''')
    code = bytearray(a.assemble())
    long_field = b'Odd HES, its name in 0x30 bytes' + b'!' * 16
    code[0x20:0xB0] = hes_fields(long_field, 'gen_gme_testdata.py', '(C) 2026', size=0x30)
    banks = [0xFF, 0xF8, 0x10, 0x11, 0x12, 0x13, 0x00, 0x11]
    h = hes_header(a.labels['init'], banks, 0x1000, 0x123C00, version=1, data_tag=b'DATB',
                   unused=b'\x00\x01\x00\x00')
    write('hes_odd.hes', h + code)


def hes_multi():
    # a second DATA block (a warning), a field that isn't text (and the
    # ones after it left out), the RAM's initial zeros
    a = HucAsm(0xE000)
    a('''
    .org $E0C0
init:
    STZ $0804
    TIN init,$0806,32
    LDA #$9F
    STA $0804
    LDA $2200
    ORA #$40
    STA $0802
    LDA #$FF
    STA $0805
    RTS
irq:
    RTI
    .org $E1F6
    .dw irq, irq, irq, init, init
''')
    code = bytearray(a.assemble())
    code[0x20:0x80] = hes_fields(b'Multi\x01', 'not shown', 'not shown')
    second = b'DATA' + struct.pack('<II', 4, 0) + b'\xEA\xEA\xEA\xEA'
    banks = [0xFF, 0xF8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
    h = hes_header(a.labels['init'], banks, len(code), 0x0000)
    write('hes_multi.hes', h + code + second)
    # data after the size (a warning), no fields at all
    code[0x20] = 0x1F
    h = hes_header(a.labels['init'], banks, len(code) - 0x10, 0x0000)
    write('hes_extra.hes', h + code)


# ---------------------------------------------------------------------------
# ---------------------------------------------------------------------------
# SAP

def sap_file(tags, blocks, separators=True):
    """A SAP file: its text header (`tags`, lines without their CR LF),
    then the binary blocks (start address, data), each after an $FFFF."""
    out = b'SAP\r\n'
    for t in tags:
        out += (t.encode('latin-1') if isinstance(t, str) else t) + b'\r\n'
    for i, (start, data) in enumerate(blocks):
        if i == 0 or separators:
            out += b'\xFF\xFF'
        out += struct.pack('<HH', start, start + len(data) - 1) + data
    return out


# The CPU exercise for the SAP files: the NSF's, without the unofficial
# instructions the Atari's CPU emulator doesn't run (see SAP_UNOFFICIAL),
# and keeping the stack where the player's return address is.
SAP_CPU_EXERCISE = CPU_EXERCISE.replace('''    LAX ($40),Y
    STX $23
    SAX $0624
    LDA #$F0
    LDX #$3C
    SBX #$11
    STX $25''', '''    LDA ($40),Y
    STA $23
    STA $0624
    LDA #$F0
    LDX #$3C
    STX $25''').replace('''    PHA
    LDX #$FF
    TXS
    TSX
    PLA''', '''    PHA
    TSX
    PLA''')
assert 'LAX' not in SAP_CPU_EXERCISE and 'SBX' not in SAP_CPU_EXERCISE

# The unofficial instructions the Atari's CPU emulator runs: the skips
# (of a byte and of a word), the NOPs and SBC #imm's copy.
SAP_UNOFFICIAL = '''
unofficial:
    .db $80, $12     ; SKB
    .db $04, $12, $14, $12, $34, $12, $44, $12, $54, $12, $64, $12, $74, $12
    .db $82, $12, $89, $12, $C2, $12, $D4, $12, $E2, $12, $F4, $12
    .db $0C, $34, $12 ; SKW
    LDX #$F0
    .db $1C, $34, $12, $3C, $34, $12, $5C, $34, $12, $7C, $34, $12, $DC, $34, $12, $FC, $34, $12
    .db $1A, $3A, $5A, $7A, $DA, $FA ; NOPs
    SEC
    LDA $10
    .db $EB, $21     ; SBC #$21
    STA $10
    RTS
'''


def sap_b():
    # A type B player: three tracks, the POKEY's four channels with their
    # distortions (pure tones, the 4-, 5-, 9- and 17-bit polynomials, the
    # volume-only mode), and AUDCTL's clocks, joined channels and high-pass
    # filters, track 2 changing them as it plays
    a = Asm(0x2000)
    a('''
init:
    STA $80          ; the track
    LDA #$00
    STA $81
    JSR cpu_test
    JSR unofficial
    LDX $80
    LDA audctl,X
    STA $D208
    LDA #$03
    STA $D20F        ; SKCTL (unmapped)
    LDA #$A8
    STA $D201
    LDA $10
    ORA #$40
    STA $D200
    LDA #$28
    STA $D203
    LDA #$79
    STA $D202
    LDA #$C6
    STA $D205
    LDA #$20
    STA $D204
    LDA #$06
    STA $D207
    LDA #$10
    STA $D206
    STA $D209        ; STIMER
    RTS

play:
    INC $81
    LDA $81
    AND #$07
    TAX
    LDA notes,X
    CLC
    ADC $10
    STA $D200
    LDA $81
    LSR A
    LSR A
    AND #$07
    TAX
    LDA notes,X
    STA $D202
    LDA $81
    AND #$10
    BEQ play_poly
    LDA $81
    AND #$0F
    ORA #$10         ; volume only
    STA $D207
    BNE play_ctl
play_poly:
    LDA $81
    AND #$E0
    ORA #$06
    STA $D207
play_ctl:
    LDA $80
    CMP #$02
    BNE play_done
    LDA $81
    AND #$1F
    BNE play_done
    LDA $81
    LSR A
    LSR A
    LSR A
    LSR A
    LSR A
    AND #$07
    TAX
    LDA audctl_seq,X
    STA $D208
play_done:
    RTS

audctl:
    .db $00, $81, $00
audctl_seq:
    .db $10, $18, $06, $60, $78, $FF, $04, $01
notes:
    .db $F3, $D9, $C1, $B6, $A2, $90, $80, $79
''')
    a(SAP_CPU_EXERCISE)
    a(SAP_UNOFFICIAL)
    code = a.assemble()
    tags = ['AUTHOR "gen_gme_testdata.py"', 'NAME "POKEY Test"', 'DATE "2026"', 'SONGS 3',
            'TYPE B', 'INIT %04X' % a.labels['init'], 'PLAYER %04X' % a.labels['play']]
    write('sap_b.sap', sap_file(tags, [(0x2000, code)]))


def sap_c():
    # A type C player (initialized through its address + 3 with the music's
    # address, then the track; played through its address + 6), at twice
    # the rate, its music in a second block; and a stereo rip, its second
    # POKEY at $D210
    a = Asm(0x3000)
    a('''
player:
    RTS
    NOP
    NOP
    JMP c_init
    JMP c_play
c_init:
    CMP #$70
    BNE c_song
    STX $90          ; the music's address
    STY $91
    RTS
c_song:
    STX $92          ; the track
    LDA #$00
    STA $93
    STA $D208
    LDA #$AA
    STA $D201
    LDA #$A6
    STA $D203
    STA $D211
    LDA #$AC
    STA $D213
    LDA #$00
    STA $D218
    RTS
c_play:
    INC $93
    LDA $93
    AND #$0F
    TAY
    LDA ($90),Y
    CLC
    ADC $92
    STA $D200
    STA $D212
    LSR A
    STA $D202
    STA $D210
    LDA $93
    AND #$1F
    BNE c_done
    LDA $93
    STA $D218        ; the second POKEY's AUDCTL
c_done:
    RTS
''')
    code = a.assemble()
    music = bytes([0x51, 0x48, 0x40, 0x3C, 0x35, 0x2F, 0x2A, 0x28,
                   0x51, 0x5B, 0x66, 0x6C, 0x79, 0x88, 0x99, 0xA2])
    tags = ['AUTHOR "gen_gme_testdata.py"', 'NAME "Type C"', 'SONGS 2', 'TYPE C',
            'PLAYER 3000', 'MUSIC 4000', 'FASTPLAY 156']
    write('sap_c.sap', sap_file(tags, [(0x3000, code), (0x4000, music)], separators=False))
    write('sap_stereo.sap', sap_file(tags[:1] + ['NAME "Stereo"', 'STEREO'] + tags[2:],
                                     [(0x3000, code), (0x4000, music)]))


def sap_odd():
    # tags that are prefixes ("IN" is INIT) or unknown, strings without
    # quotes, an empty line; code returning by RTI (through the NOP at
    # $FEFE), an illegal instruction (track 1), a BRK past the idle address
    # (track 2), an init that never returns (track 3), a block ending
    # before it starts (a warning); and a Digimusic rip (an error)
    a = Asm(0x2000)
    a('''
init:
    STA $80
    TAX
    BEQ init_done
    DEX
    BEQ init_done
    DEX
    BEQ init_brk
init_loop:
    JMP init_loop
init_brk:
    JMP $FF00
init_done:
    LDA #$A6
    STA $D201
    LDA #$40
    STA $D200
    RTS
play:
    INC $81
    LDA $81
    STA $D200
    LDA $80
    BEQ play_rti
    .db $02          ; KIL: illegal
play_rti:
    RTI
''')
    code = a.assemble()
    tags = ['AUTHOR gen_gme_testdata.py', 'NAME "Odd SAP', 'DATE "2026"', 'SONGS 4',
            'IN 2000', 'PLAYER %04X' % a.labels['play'], 'FASTPLAY 100', 'TIME 01:00', '']
    blocks = [(0x2000, code), (0xFEFE, b'\xEA')]
    data = sap_file(tags, blocks)
    data += b'\xFF\xFF' + struct.pack('<HH', 0x0600, 0x05FF) + b'\x00\x00'
    write('sap_odd.sap', data)
    write('sap_digi.sap', sap_file(['TYPE D', 'INIT 2000'], [(0x2000, b'\x60' * 8)]))


# ---------------------------------------------------------------------------
# A small assembler for the SPC-700, on the same two-pass assembler, in
# the usual syntax with explicit operand kinds: `$12` a direct page
# address, `!label` or `!$1234` an absolute one, `#$12` an immediate,
# `$12.3` a direct page bit, `!label.3` an absolute bit (MOV1 and the
# other bit instructions), branch targets as labels.

def _spc_ops():
    t = {}

    def op(code, text, order=None):
        assert code not in t.values(), hex(code)
        t[text] = (code, order)

    alu = [('OR', 0x00), ('AND', 0x20), ('EOR', 0x40), ('CMP', 0x60), ('ADC', 0x80), ('SBC', 0xA0)]
    for name, b in alu:
        op(b + 0x04, name + ' A,D')
        op(b + 0x05, name + ' A,!W')
        op(b + 0x06, name + ' A,(X)')
        op(b + 0x07, name + ' A,[D+X]')
        op(b + 0x08, name + ' A,#I')
        op(b + 0x09, name + ' D,S', 'SD')
        op(b + 0x14, name + ' A,D+X')
        op(b + 0x15, name + ' A,!W+X')
        op(b + 0x16, name + ' A,!W+Y')
        op(b + 0x17, name + ' A,[D]+Y')
        op(b + 0x18, name + ' D,#I', 'ID')
        op(b + 0x19, name + ' (X),(Y)')
    for n in range(16):
        op(n * 0x10 + 0x01, 'TCALL %d' % n)
    for bit in range(8):
        op(0x02 + bit * 0x20, 'SET1 D.%d' % bit)
        op(0x12 + bit * 0x20, 'CLR1 D.%d' % bit)
        op(0x03 + bit * 0x20, 'BBS D.%d,R' % bit)
        op(0x13 + bit * 0x20, 'BBC D.%d,R' % bit)
    for code, text in [
            (0x00, 'NOP'), (0x0A, 'OR1 C,M'), (0x2A, 'OR1 C,/M'), (0x4A, 'AND1 C,M'),
            (0x6A, 'AND1 C,/M'), (0x8A, 'EOR1 C,M'), (0xAA, 'MOV1 C,M'), (0xCA, 'MOV1 M,C'),
            (0xEA, 'NOT1 M'),
            (0x0B, 'ASL D'), (0x0C, 'ASL !W'), (0x1B, 'ASL D+X'), (0x1C, 'ASL A'),
            (0x2B, 'ROL D'), (0x2C, 'ROL !W'), (0x3B, 'ROL D+X'), (0x3C, 'ROL A'),
            (0x4B, 'LSR D'), (0x4C, 'LSR !W'), (0x5B, 'LSR D+X'), (0x5C, 'LSR A'),
            (0x6B, 'ROR D'), (0x6C, 'ROR !W'), (0x7B, 'ROR D+X'), (0x7C, 'ROR A'),
            (0x0D, 'PUSH PSW'), (0x2D, 'PUSH A'), (0x4D, 'PUSH X'), (0x6D, 'PUSH Y'),
            (0x8E, 'POP PSW'), (0xAE, 'POP A'), (0xCE, 'POP X'), (0xEE, 'POP Y'),
            (0x0E, 'TSET1 !W'), (0x4E, 'TCLR1 !W'),
            (0x0F, 'BRK'), (0x1F, 'JMP [!W+X]'), (0x5F, 'JMP !W'), (0x3F, 'CALL !W'),
            (0x4F, 'PCALL U'), (0x6F, 'RET'), (0x7F, 'RETI'),
            (0x10, 'BPL R'), (0x30, 'BMI R'), (0x50, 'BVC R'), (0x70, 'BVS R'),
            (0x90, 'BCC R'), (0xB0, 'BCS R'), (0xD0, 'BNE R'), (0xF0, 'BEQ R'), (0x2F, 'BRA R'),
            (0x1A, 'DECW D'), (0x3A, 'INCW D'), (0x5A, 'CMPW YA,D'), (0x7A, 'ADDW YA,D'),
            (0x9A, 'SUBW YA,D'), (0xBA, 'MOVW YA,D'), (0xDA, 'MOVW D,YA'),
            (0x1D, 'DEC X'), (0x3D, 'INC X'), (0x5D, 'MOV X,A'), (0x7D, 'MOV A,X'),
            (0x9D, 'MOV X,SP'), (0xBD, 'MOV SP,X'), (0xDD, 'MOV A,Y'), (0xFD, 'MOV Y,A'),
            (0x1E, 'CMP X,!W'), (0x3E, 'CMP X,D'), (0x5E, 'CMP Y,!W'), (0x7E, 'CMP Y,D'),
            (0xC8, 'CMP X,#I'), (0xAD, 'CMP Y,#I'),
            (0x20, 'CLRP'), (0x40, 'SETP'), (0x60, 'CLRC'), (0x80, 'SETC'), (0xA0, 'EI'),
            (0xC0, 'DI'), (0xE0, 'CLRV'), (0xED, 'NOTC'),
            (0x2E, 'CBNE D,R'), (0xDE, 'CBNE D+X,R'), (0x6E, 'DBNZ D,R'), (0xFE, 'DBNZ Y,R'),
            (0x8B, 'DEC D'), (0x8C, 'DEC !W'), (0x9B, 'DEC D+X'), (0x9C, 'DEC A'),
            (0xAB, 'INC D'), (0xAC, 'INC !W'), (0xBB, 'INC D+X'), (0xBC, 'INC A'),
            (0xDC, 'DEC Y'), (0xFC, 'INC Y'),
            (0x8D, 'MOV Y,#I'), (0x9E, 'DIV YA,X'), (0x9F, 'XCN A'),
            (0xBE, 'DAS A'), (0xDF, 'DAA A'), (0xCF, 'MUL YA'),
            (0xAF, 'MOV (X)+,A'), (0xBF, 'MOV A,(X)+'),
            (0xC4, 'MOV D,A'), (0xC5, 'MOV !W,A'), (0xC6, 'MOV (X),A'), (0xC7, 'MOV [D+X],A'),
            (0xC9, 'MOV !W,X'), (0xCB, 'MOV D,Y'), (0xCC, 'MOV !W,Y'), (0xCD, 'MOV X,#I'),
            (0xD4, 'MOV D+X,A'), (0xD5, 'MOV !W+X,A'), (0xD6, 'MOV !W+Y,A'), (0xD7, 'MOV [D]+Y,A'),
            (0xD8, 'MOV D,X'), (0xD9, 'MOV D+Y,X'), (0xDB, 'MOV D+X,Y'),
            (0xE4, 'MOV A,D'), (0xE5, 'MOV A,!W'), (0xE6, 'MOV A,(X)'), (0xE7, 'MOV A,[D+X]'),
            (0xE8, 'MOV A,#I'), (0xE9, 'MOV X,!W'), (0xEB, 'MOV Y,D'), (0xEC, 'MOV Y,!W'),
            (0xF4, 'MOV A,D+X'), (0xF5, 'MOV A,!W+X'), (0xF6, 'MOV A,!W+Y'), (0xF7, 'MOV A,[D]+Y'),
            (0xF8, 'MOV X,D'), (0xF9, 'MOV X,D+Y'), (0xFB, 'MOV Y,D+X'),
            (0xEF, 'SLEEP'), (0xFF, 'STOP')]:
        op(code, text)
    op(0x8F, 'MOV D,#I', 'ID')
    op(0xFA, 'MOV D,S', 'SD')
    assert sorted(c for c, _ in t.values()) == list(range(256))
    return t


SPC_OPS = _spc_ops()


def _spc_patterns():
    import re
    regex = {
        'D': r'(\$[0-9A-Fa-f]{1,2})',
        'S': r'(\$[0-9A-Fa-f]{1,2})',
        'W': r'([\w$<>]+)',
        'I': r'([^,\s]+)',
        'R': r'([\w$]+)',
        'M': r'!([\w$<>]+)\.([0-7])',
        'U': r'(\$[0-9A-Fa-f]{1,2})',
    }
    pats = []
    for text, (code, order) in SPC_OPS.items():
        rx = ''
        kinds = []
        # (the operand placeholders are D, S, W, I, R, M and U standing
        # alone, not in a name like PSW or SP)
        for i, ch in enumerate(text):
            alone = ch in regex and not (i > 0 and text[i - 1].isalpha()) and \
                not (i + 1 < len(text) and text[i + 1].isalpha())
            if alone and ' ' in text[:i]:
                rx += regex[ch]
                kinds.append(ch)
            elif ch == ' ':
                rx += r'\s+'
            else:
                rx += re.escape(ch)
        pats.append((re.compile('^' + rx + '$', re.I), code, kinds, order))
    return pats


SPC_PATTERNS = _spc_patterns()


class SpcAsm(Asm):
    """Assembles SPC-700 source text (see Asm and the operand kinds above)."""

    def __init__(self, org):
        super().__init__(org, {})

    def encode(self, mnem, arg, labels, final, pc, line):
        text = (mnem + ' ' + arg).strip() if arg else mnem
        text = ' '.join(text.split(None, 1)) if ' ' in text else text
        text = text.replace(', ', ',')
        for rx, code, kinds, order in SPC_PATTERNS:
            m = rx.match(text)
            if not m:
                continue
            vals = {}
            gi = 1
            for k in kinds:
                if k == 'M':
                    vals['M'] = (self.value(m.group(gi), labels, final) & 0x1FFF) | \
                        int(m.group(gi + 1)) << 13
                    gi += 2
                else:
                    vals[k] = self.value(m.group(gi), labels, final)
                    gi += 1
            out = bytearray([code])
            size = 1 + sum(2 if k in 'WM' else 1 for k in kinds)
            for k in (order if order else kinds):
                v = vals[k]
                if k in 'WM':
                    out += struct.pack('<H', v & 0xFFFF)
                elif k == 'R':
                    off = v - (pc + size)
                    if final:
                        assert -128 <= off <= 127, (line, off)
                    out.append(off & 0xFF)
                else:
                    out.append(v & 0xFF)
            return bytes(out)
        raise ValueError('bad SPC-700 instruction: ' + line)


# ---------------------------------------------------------------------------
# SPC

def spc_file(pc, ram_blocks, dsp_regs, text=None, regs=(0, 0, 0, 0x02, 0xEF), trailer=b'',
             size=0x10200, has_id666=26):
    """An SPC file: its header (with the ID666 `text`, 210 bytes), the 64K
    RAM made of `ram_blocks` (address, data), the DSP registers, the
    unused bytes and the IPL ROM's, then any `trailer`."""
    a, x, y, psw, sp = regs
    h = b'SNES-SPC700 Sound File Data v0.30\x1A\x1A' + bytes([has_id666, 30])
    h += struct.pack('<H', pc) + bytes([a, x, y, psw, sp]) + bytes(2)
    text = text if text is not None else bytes(210)
    assert len(text) == 210
    h += text
    assert len(h) == 0x100
    ram = bytearray(0x10000)
    for addr, data in ram_blocks:
        ram[addr:addr + len(data)] = data
    dsp = bytearray(128)
    for r, v in dsp_regs.items():
        dsp[r] = v
    data = h + bytes(ram) + bytes(dsp) + bytes(0x40) + bytes(0x40)
    return data[:size] + trailer


def id666_text(song, game, dumper, comment, date, length, fade, author, binary=False):
    def f(s, n):
        b = s.encode('latin-1') if isinstance(s, str) else s
        return b[:n] + bytes(n - len(b[:n]))
    t = f(song, 32) + f(game, 32) + f(dumper, 16) + f(comment, 32)
    if binary:
        t += f(date, 4) + bytes(7) + struct.pack('<H', length) + bytes(1) + struct.pack('<I', fade)
    else:
        t += f(date, 11) + f(str(length), 3) + f(str(fade), 4)
    t += f(author, 32) + bytes([0, 0]) + bytes(46)
    assert len(t) == 210, len(t)
    return t


def brr_block(samples, shift, filt, end=False, loop=False):
    """One BRR block: 16 four-bit samples, with its header."""
    header = shift << 4 | filt << 2 | (2 if loop else 0) | (1 if end else 0)
    out = bytearray([header])
    for i in range(0, 16, 2):
        out.append((samples[i] & 0xF) << 4 | (samples[i + 1] & 0xF))
    return bytes(out)


# The SPC-700 exercise: most instructions and addressing modes, leaving a
# value in $10 (and the stack as it found it), which the song uses, so a
# mistake in any of them changes the sound.
SPC_CPU_EXERCISE = '''
cpu_test:
    MOV A,#$5A
    MOV $20,A
    MOV $21,#$00
    MOV X,#$07
ct_loop:
    MOV A,$20
    ASL A
    ROL $21
    BCC ct_nc
    EOR A,#$1D
ct_nc:
    MOV $20,A
    CLRC
    ADC A,$21
    MOV $30+X,A
    SETC
    SBC A,#$37
    BVS ct_ov
    OR A,#$01
ct_ov:
    DEC X
    BPL ct_loop
    ; indirect modes
    MOV $40,#$30
    MOV $41,#$00
    MOV Y,#$03
    MOV A,[$40]+Y
    MOV X,#$02
    EOR A,[$3E+X]
    ADC A,(X)
    MOV X,#$30
    AND A,(X)
    OR A,!$0031
    SBC A,!$0030+X
    CMP A,!$0030+Y
    MOV $22,A
    MOV [$40]+Y,A
    MOV [$3E+X],A
    MOV (X),A
    MOV (X)+,A
    MOV A,(X)+
    MOV !$0640,A
    MOV !$0640+X,A
    MOV !$0640+Y,A
    MOV $23,X
    ; dp to dp
    MOV $24,$22
    OR $24,$23
    AND $25,#$F0
    EOR $25,$24
    ADC $25,#$11
    SBC $24,$25
    CMP $24,$25
    CMP $24,#$40
    MOV X,#$24
    MOV Y,#$25
    OR (X),(Y)
    AND (X),(Y)
    EOR (X),(Y)
    ADC (X),(Y)
    SBC (X),(Y)
    CMP (X),(Y)
    ; 16 bits
    MOVW YA,$24
    ADDW YA,$20
    SUBW YA,$22
    CMPW YA,$24
    MOVW $26,YA
    INCW $26
    DECW $27
    MOV A,#$9C
    MOV Y,#$13
    MUL YA
    MOV X,#$07
    DIV YA,X
    MOV $28,A
    MOV $29,Y
    MOV A,#$90
    MOV Y,#$45
    MOV X,#$21
    DIV YA,X
    XCN A
    ADC A,$29
    DAA A
    DAS A
    MOV $29,A
    ; bits
    SET1 $2A.0
    SET1 $2A.3
    SET1 $2A.7
    CLR1 $2A.3
    BBS $2A.0,ct_b0
    INC $2A
ct_b0:
    BBC $2A.1,ct_b1
    INC $2A
ct_b1:
    BBS $2A.1,ct_b2
    INC $2A
ct_b2:
    MOV1 C,!$002A.7
    AND1 C,!$002A.0
    OR1 C,/!$002A.2
    EOR1 C,!$002A.5
    AND1 C,/!$002A.6
    OR1 C,!$002A.6
    MOV1 !$002B.4,C
    NOT1 !$002B.1
    MOV A,#$0F
    TSET1 !$002B
    TCLR1 !$002C
    ; read-modify-write
    MOV A,#$81
    MOV !$0600,A
    MOV !$0605,A
    ASL !$0600
    LSR !$0605
    ROL !$0605
    ROR !$0600
    INC !$0600
    DEC !$0605
    ASL $30
    ROL $31
    MOV X,#$05
    LSR $32+X
    ROR $33+X
    INC $34+X
    DEC $35+X
    ASL $30+X
    ROL $31+X
    LSR $36
    ROR $37
    INC $36
    DEC $37
    MOV A,!$0600
    CLRC
    ADC A,!$0605
    ADC A,$30
    ADC A,$37
    MOV $2D,A
    ; registers
    MOV X,#$11
    MOV Y,#$22
    MOV A,X
    MOV X,A
    MOV A,Y
    MOV Y,A
    INC A
    INC X
    INC Y
    DEC A
    DEC X
    DEC Y
    MOV $2E,X
    MOV $2F,Y
    MOV X,$2E
    MOV Y,$2F
    MOV X,$2E+Y
    MOV Y,$2E+X
    MOV $2E+Y,X
    MOV $2F+X,Y
    MOV X,!$002E
    MOV Y,!$002F
    MOV !$0641,X
    MOV !$0642,Y
    CMP X,#$12
    CMP X,$2E
    CMP X,!$002F
    CMP Y,#$23
    CMP Y,$2F
    CMP Y,!$002E
    CBNE $2E,ct_c1
    INC $2D
ct_c1:
    MOV X,#$01
    CBNE $2D+X,ct_c2
    INC $2D
ct_c2:
    MOV $2C,#$03
ct_dbnz:
    INC $2D
    DBNZ $2C,ct_dbnz
    MOV Y,#$03
ct_dbnz_y:
    DEC $2D
    DBNZ Y,ct_dbnz_y
    ; flags, the stack and the direct page
    PUSH PSW
    PUSH A
    PUSH X
    PUSH Y
    SETC
    NOTC
    CLRV
    SETP
    MOV $10,#$77
    MOV A,$10
    CLRP
    POP Y
    POP X
    POP A
    POP PSW
    MOV X,SP
    MOV SP,X
    EI
    DI
    ; calls and jumps
    MOV X,#$02
    JMP [!ct_vec+X]
ct_back:
    CALL !ct_sub
    TCALL 1
    PCALL $80
    BRK
    BRA ct_fin
ct_sub:
    MOV A,#$7F
    CLRC
    ADC A,#$01
    BVC ct_sub_done
    INC $2D
ct_sub_done:
    RET
ct_fin:
    MOV A,$20
    EOR A,$22
    EOR A,$24
    EOR A,$25
    EOR A,$26
    EOR A,$27
    EOR A,$28
    EOR A,$29
    EOR A,$2A
    EOR A,$2B
    EOR A,$2C
    EOR A,$2D
    EOR A,$0F
    MOV $10,A
    RET
ct_jmp:
    JMP !ct_back
ct_vec:
    .dw ct_back, ct_jmp
'''

# The vectors and routines the exercise reaches: TCALL 1 at $FFDC, PCALL
# $80 at $FF80, BRK at $FFDE (and RETI back), in RAM (the IPL ROM is
# disabled).
SPC_HIGH = '''
    .org $FF80
pcall80:
    INC $0F
    RET
    .org $FFC0
brk_routine:
    INC $0F
    INC $0F
    RETI
tcall1:
    MOV A,$0F
    ADC A,#$10
    MOV $0F,A
    RET
    .org $FFDC
    .dw tcall1, brk_routine
'''


def spc_dsp():
    # The DSP: eight voices of BRR samples (every filter and shift range,
    # looped), ADSR and every GAIN mode, noise, pitch modulation, echo with
    # its FIR filter and feedback, key on and off, driven by timer 0; and
    # through the CPU exercise, the SPC-700
    a = SpcAsm(0x0200)
    a('''
start:
    MOV X,#$EF
    MOV SP,X
    MOV $0F,#$00
    CALL !cpu_test
    MOV X,#$00
init_dsp:
    MOV A,!dsp_init+X
    MOV $F2,A
    INC X
    MOV A,!dsp_init+X
    MOV $F3,A
    INC X
    CMP X,#dsp_init_end-dsp_init
    BNE init_dsp
    MOV A,$10
    AND A,#$3F
    MOV $F2,#$02
    MOV $F3,A        ; voice 0's pitch from the exercise
    MOV $F2,#$4C
    MOV $F3,#$FF     ; KON
    MOV $FA,#$40     ; timer 0: 125 Hz
    MOV $F1,#$01
    MOV $50,#$00
main:
    MOV A,$FD
    BEQ main
    INC $50
    MOV A,$50
    AND A,#$07
    BNE main
    MOV A,$50
    LSR A
    LSR A
    LSR A
    AND A,#$07
    MOV Y,A
    MOV A,!notes+Y
    MOV $F2,#$13
    MOV $F3,A        ; voice 1's pitch
    MOV $F2,#$22
    MOV $F3,A        ; voice 2's
    MOV A,$50
    AND A,#$3F
    BNE main_kon
    MOV $F2,#$5C
    MOV $F3,#$0C     ; KOFF voices 2 and 3
    MOV $F2,#$5C
    MOV $F3,#$00
main_kon:
    MOV A,$50
    AND A,#$3F
    CMP A,#$20
    BNE main_kon2
    MOV $F2,#$57
    MOV $F3,#$9A     ; voice 5: GAIN linear dec
    MOV $F2,#$67
    MOV $F3,#$B8     ; voice 6: GAIN exp dec
main_kon2:
    MOV A,$50
    AND A,#$1F
    BNE main_read
    MOV $F2,#$4C
    MOV $F3,#$0E     ; KON voices 1-3
main_read:
    MOV $F2,#$08
    MOV A,$F3        ; voice 0's ENVX
    MOV $F2,#$7C
    MOV A,$F3        ; ENDX
    MOV $F2,#$7C
    MOV $F3,A
    MOV $F2,#$6C
    MOV A,$50
    AND A,#$C0
    BNE main_noise
    MOV $F3,#$02     ; FLG: noise clock
    BRA main
main_noise:
    MOV $F3,#$3C     ; noise clock up, echo writes off
    BRA main

notes:
    .db $08, $09, $0A, $0B, $0C, $0E, $10, $12
dsp_init:
    .db $0C, $60, $1C, $50, $2C, $30, $3C, $D0 ; main and echo volumes (surround)
    .db $5D, $30, $6D, $80, $7D, $02, $0D, $40 ; DIR, ESA, EDL, EFB
    .db $0F, $7F, $1F, $00, $2F, $10, $3F, $F0, $4F, $08, $5F, $00, $6F, $FC, $7F, $04 ; FIR
    .db $2D, $40, $3D, $80, $4D, $C3, $6C, $00 ; PMON, NON, EON, FLG
    .db $00, $40, $01, $40, $03, $10, $04, $00, $05, $8F, $06, $E0 ; voice 0: ADSR
    .db $10, $38, $11, $28, $13, $08, $14, $01, $15, $FA, $16, $8B ; voice 1: ADSR, slow
    .db $20, $30, $21, $D0, $23, $0C, $24, $02, $25, $00, $27, $7F ; voice 2: GAIN direct
    .db $30, $20, $31, $20, $33, $06, $34, $03, $35, $00, $37, $DA ; voice 3: GAIN linear inc
    .db $40, $28, $41, $18, $43, $09, $44, $02, $45, $00, $47, $FC ; voice 4: GAIN bent inc
    .db $50, $18, $51, $28, $53, $07, $54, $03, $55, $00, $57, $DF ; voice 5: GAIN linear inc, then dec
    .db $60, $20, $61, $20, $63, $10, $64, $01, $65, $00, $67, $DF ; voice 6: GAIN linear inc, then exp dec
    .db $70, $10, $71, $30, $73, $04, $74, $00, $75, $9F, $76, $11 ; voice 7: noise
dsp_init_end:
''')
    a(SPC_CPU_EXERCISE)
    a(SPC_HIGH)
    code = a.assemble()
    # the sample directory and the samples: a square, a saw, a sine-ish
    # wave, a pulse and a noise-like one, with every filter and shift range
    import math
    dir_ = bytearray()
    samples = bytearray()
    base = 0x3100

    def add(blocks, loop_at=0):
        nonlocal samples
        start = base + len(samples)
        dir_.extend(struct.pack('<HH', start, start + 9 * loop_at))
        samples += b''.join(blocks)

    add([brr_block([7] * 8 + [-8] * 8, 10, 0, end=True, loop=True)])
    add([brr_block([(i - 8) for i in range(16)], 9, 1),
         brr_block([(7 - i) for i in range(16)], 8, 2, end=True, loop=True)], 1)
    add([brr_block([int(7 * math.sin(i * math.pi / 8)) for i in range(16)], 11, 0),
         brr_block([int(7 * math.sin(i * math.pi / 8 + 0.5)) for i in range(16)], 12, 3),
         brr_block([int(-7 * math.sin(i * math.pi / 8)) for i in range(16)], 0, 0,
                   end=True, loop=True)], 0)
    add([brr_block([7, 7, 7, 7, -8, -8, -8, -8] * 2, 13, 1),
         brr_block([3, -4] * 8, 14, 2),
         brr_block([1, -2] * 8, 15, 3, end=True, loop=True)], 1)
    add([brr_block([((i * 5) % 16) - 8 for i in range(16)], 7, 0),
         brr_block([((i * 11) % 16) - 8 for i in range(16)], 6, 2, end=True)], 0)
    add([brr_block([0] * 16, 1, 0, end=True, loop=True)])
    text = id666_text('DSP Test', 'gen_gme_testdata.py', 'gen', 'every DSP feature',
                      '10/17/2026', 60, 5000, 'gen_gme_testdata.py')
    blocks = [(0x0200, code[:0x10000 - 0x0200])]
    spc = spc_file(0x0200, blocks + [(0x3000, bytes(dir_)), (base, bytes(samples)),
                                     (0x00F0, bytes([0x0A, 0x00, 0x00, 0x00, 0, 0, 0, 0,
                                                     0, 0, 0, 0, 0, 0x0F, 0x0F, 0x0F]))],
                   {0x6C: 0xE0, 0x5D: 0x30}, text=text)
    write('spc_dsp.spc', spc)


def xid6_block(id_, data=None, value=0, pad=True, pad_byte=0):
    """One xid6 block: a string or data (type 1 or 4) or a value (type 0)."""
    if data is None:
        return bytes([id_, 0]) + struct.pack('<H', value)
    b = data.encode('latin-1') if isinstance(data, str) else data
    out = bytes([id_, 1 if isinstance(data, str) else 4]) + struct.pack('<H', len(b)) + b
    if pad:
        while len(out) & 3:
            out += bytes([pad_byte])
    return out


def spc_odd():
    # a binary ID666 header with a length too long to use (and an author
    # starting with a digit), an xid6 trailer (every field, a year and a
    # copyright, a block without padding, one with bad padding, unknown
    # blocks, data after its size, a block past its end); the timers and
    # their registers, the ports, the IPL ROM switched in (and written
    # under), a DSP register past $7F, the sample directory in the last
    # page (with a source number past it), the CPU stopping (an error)
    a = SpcAsm(0x0200)
    a('''
start:
    MOV X,#$CF
    MOV SP,X
    MOV $F2,#$5D
    MOV $F3,#$FF     ; DIR in the last page
    MOV $F2,#$04
    MOV $F3,#$FF     ; voice 0's SRCN past it
    MOV $F2,#$14
    MOV $F3,#$01
    MOV $F2,#$4C
    MOV $F3,#$03     ; KON 0, 1
    MOV $F2,#$80
    MOV $F3,#$12     ; past the registers
    MOV $F2,#$80
    MOV A,$F3
    MOV $F0,#$0A     ; test
    MOV $F8,#$55
    MOV $F9,$F8
    MOV A,$F9
    MOV $F4,A        ; ports
    MOV $F5,#$66
    MOV A,$F6
    MOV $F7,A
    MOV $F1,#$B0     ; clear the ports, the IPL ROM in
    MOV A,!$FFC0
    MOV !$FFC1,A     ; under the ROM
    MOV $F1,#$07     ; the ROM out, the timers on
    MOV A,!$FFC1
    MOV $FA,#$10
    MOV $FB,#$00
    MOV $FC,#$80
    MOV $FD,A        ; a counter write
    MOV $51,#$00
    MOV $52,#$00
odd_loop:
    MOV A,$FD
    ADC A,$FE
    ADC A,$FF
    BEQ odd_loop
    MOV $F2,#$02
    MOV $F3,A
    INC $51
    MOV A,$51
    CMP A,#$40
    BNE odd_t
    MOV $FA,#$20     ; a new target while running
odd_t:
    CMP A,#$00
    BNE odd_loop
    INC $52
    MOV A,$52
    CMP A,#$06
    BNE odd_loop
    STOP
''')
    code = a.assemble()
    text = id666_text('Odd SPC', 'gen_gme_testdata.py', 'gen', 'odd', b'\x17\x10\xEA\x07',
                      0xFFFF, 0x12345678, '9 the author', binary=True)
    trailer = b''.join([
        xid6_block(0x01, 'xid6 song'),
        xid6_block(0x02, 'xid6 game'),
        xid6_block(0x03, 'xid6 author'),
        xid6_block(0x04, 'xid6 dumper'),
        xid6_block(0x07, 'xid6 comment!', pad=False),
        xid6_block(0x14, value=2026),
        xid6_block(0x30, struct.pack('<I', 64000 * 3)),
        xid6_block(0x50, value=7),
        xid6_block(0x09, b'\x01\x02\x03'),
        xid6_block(0x13, 'gen_gme_testdata.py', pad_byte=0x20),
    ])
    info = b'xid6' + struct.pack('<I', len(trailer) + 4) + trailer + xid6_block(0x01, 'past')
    spc = spc_file(0x0200, [(0x0200, code), (0xFF00, bytes([0x40, 0xFF, 0x40, 0xFF]) * 4),
                            (0xFF40, brr_block([7, 3, -2, -8, -4, 0, 5, 7] * 2, 11, 1,
                                               end=True, loop=True)),
                            (0x00F0, bytes([0x0A, 0x00, 0, 0, 0x11, 0x22, 0x33, 0x44,
                                            0, 0, 0, 0, 0, 0, 0, 0]))],
                   {0x6C: 0x00, 0x0C: 0x7F, 0x1C: 0x7F, 0x00: 0x7F, 0x01: 0x7F, 0x03: 0x10,
                    0x10: 0x40, 0x11: 0x40, 0x13: 0x08, 0x17: 0x7F, 0x07: 0x7F},
                   text=text, trailer=info, has_id666=26)
    write('spc_odd.spc', spc)


# ---------------------------------------------------------------------------
# VGM: command streams for the Sega Master System's and Game Gear's SN76489
# (the PSG) and the Mega Drive's YM2612, with its DAC.

class Vgm:
    """A VGM command stream, with the samples it has waited."""

    def __init__(self):
        self.b = bytearray()
        self.samples = 0
        self.loop_pos = None
        self.loop_samples = 0
        self.waits = 0

    def psg(self, *vals, chip=0):
        for v in vals:
            self.b += bytes([0x50 if chip == 0 else 0x30, v])

    def gg(self, v, chip=0):
        self.b += bytes([0x4F if chip == 0 else 0x3F, v])

    def ym(self, port, a, d, chip=0):
        self.b += bytes([(0x52, 0x53, 0xA2, 0xA3)[chip * 2 + port], a, d])

    def ym2413(self, a, d, chip=0):
        self.b += bytes([0x51 if chip == 0 else 0xA1, a, d])

    def raw(self, *vals):
        self.b += bytes(vals)

    def wait(self, n):
        """Wait n samples, with every kind of delay command in turn."""
        self.samples += n
        while n > 0:
            self.waits += 1
            if n == 735:
                self.raw(0x62)
                n = 0
            elif n == 882:
                self.raw(0x63)
                n = 0
            elif n <= 16 and self.waits % 2:
                self.raw(0x70 | (n - 1))
                n = 0
            elif n <= 255 and self.waits % 3 == 0:
                self.raw(0x64, n)
                n = 0
            else:
                k = min(n, 65535)
                self.raw(0x61, k & 0xFF, k >> 8)
                n -= k

    def pcm(self, n):
        """Play the next PCM sample and wait n (0 to 15) samples."""
        self.samples += n
        self.raw(0x80 | n)

    def data_block(self, data, type_=0):
        self.raw(0x67, 0x66, type_)
        self.b += struct.pack('<I', len(data)) + bytes(data)

    def pcm_seek(self, at):
        self.raw(0xE0)
        self.b += struct.pack('<I', at)

    def mark_loop(self):
        self.loop_pos = len(self.b)
        self.loop_samples = self.samples

    def end(self):
        self.raw(0x66)


def gd3(*strings, version=0x100):
    """A GD3 tag with these strings (track, its Japanese, game, ..., notes)."""
    data = b''.join(s.encode('utf-16-le') + b'\0\0' for s in strings)
    return b'Gd3 ' + struct.pack('<II', version, len(data)) + data


def vgm_file(v, version=0x150, psg=3579545, ym2413=0, ym2612=0, ym2151=0, tag=b'',
             extra=b'', noise=(0x0009, 16), rate=60, total=None, loop_samples=None,
             data_offset=None):
    """A VGM file of the stream v, with the header's clocks and its tag;
    extra is put between the header and the data (version 1.50 and later)."""
    if version < 0x150:
        extra = b''
    start = 0x40 + len(extra)
    body = bytes(v.b)
    f = bytearray(0x40) + extra + body
    tag_at = len(f)
    f += tag
    h = bytearray(0x40)
    h[0:4] = b'Vgm '
    total = v.samples if total is None else total
    loop_pos = v.loop_pos
    if loop_samples is None:
        loop_samples = v.samples - v.loop_samples if loop_pos is not None else 0
    struct.pack_into('<IIIIIIIII', h, 4, len(f) - 4, version, psg, ym2413,
                     tag_at - 0x14 if tag else 0,
                     total, loop_pos + start - 0x1C if loop_pos is not None else 0,
                     loop_samples, rate)
    struct.pack_into('<HBB', h, 0x28, noise[0], noise[1], 0)
    if data_offset is None:
        data_offset = start - 0x34 if version >= 0x150 else 0
    struct.pack_into('<III', h, 0x2C, ym2612, ym2151, data_offset)
    f[0:0x40] = h
    return f


YM_CLOCK = 7670453
# the register offsets of operators 1 to 4
YM_OPS = [0, 8, 4, 12]


def ym_fnum(freq, block, clock=YM_CLOCK):
    return min(0x7FF, round(freq * 144 * (1 << 20) / clock / (1 << (block - 1))))


def ym_patch(v, ch, alg, fb, ops, ams=0, pms=0, pan=3, chip=0):
    """Set channel ch (0 to 5)'s instrument: ops are the operators'
    (DT/MUL, TL, KS/AR, AM/DR, SR, SL/RR, SSG-EG)."""
    port, c = divmod(ch, 3)
    for k, op in enumerate(ops):
        for base, val in zip((0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90), op):
            v.ym(port, base + YM_OPS[k] + c, val, chip)
    v.ym(port, 0xB0 + c, fb << 3 | alg, chip)
    v.ym(port, 0xB4 + c, pan << 6 | ams << 4 | pms, chip)


def ym_note(v, ch, block, fnum, chip=0):
    port, c = divmod(ch, 3)
    v.ym(port, 0xA4 + c, block << 3 | fnum >> 8, chip)
    v.ym(port, 0xA0 + c, fnum & 0xFF, chip)


def ym_key(v, ch, ops=0xF, chip=0):
    v.ym(0, 0x28, ops << 4 | (ch % 3) | (4 if ch >= 3 else 0), chip)


def psg_tone(v, ch, period, vol, chip=0):
    v.psg(0x80 | ch << 5 | (period & 15), period >> 4 & 0x3F, 0x90 | ch << 5 | vol, chip=chip)


# Six instruments, one an algorithm (0 to 5): (alg, fb, ops, ams, pms, pan)
YM_PATCHES = [
    (0, 7, [(0x71, 0x23, 0x1F, 0x05, 0x02, 0x11, 0), (0x0D, 0x2D, 0x99, 0x05, 0x02, 0x11, 0),
            (0x33, 0x26, 0x5F, 0x05, 0x02, 0x11, 0), (0x01, 0x00, 0x94, 0x87, 0x02, 0xA6, 0)], 2, 3, 3),
    (1, 5, [(0x32, 0x1E, 0x1F, 0x0E, 0x00, 0x1F, 0x08), (0x14, 0x24, 0x1F, 0x0B, 0x00, 0x1F, 0x09),
            (0x51, 0x18, 0x1F, 0x0E, 0x00, 0x1F, 0x0A), (0x02, 0x04, 0x1F, 0x0A, 0x00, 0x1F, 0x0B)], 0, 0, 2),
    (2, 3, [(0x02, 0x16, 0x1A, 0x12, 0x04, 0x4F, 0), (0x22, 0x30, 0x58, 0x0C, 0x03, 0x3F, 0),
            (0x46, 0x22, 0x1C, 0x0D, 0x05, 0x2F, 0), (0x01, 0x02, 0x1F, 0x05, 0x02, 0x5C, 0)], 1, 6, 1),
    (3, 0, [(0x0F, 0x18, 0xDF, 0x1F, 0x1F, 0x0F, 0x0C), (0x61, 0x1A, 0x9F, 0x12, 0x10, 0x2F, 0x0D),
            (0x13, 0x28, 0x5F, 0x14, 0x08, 0x3F, 0x0E), (0x03, 0x06, 0x1F, 0x10, 0x05, 0x4F, 0x0F)], 0, 0, 3),
    (4, 6, [(0x00, 0x1C, 0x1F, 0x8C, 0x0A, 0x2F, 0), (0x01, 0x10, 0x1F, 0x0C, 0x0A, 0x2F, 0),
            (0x70, 0x22, 0x1F, 0x8C, 0x0A, 0x2F, 0), (0x02, 0x0A, 0x1F, 0x0C, 0x0A, 0x2F, 0)], 3, 7, 3),
    (5, 2, [(0x04, 0x20, 0x14, 0x16, 0x01, 0x4A, 0), (0x51, 0x0C, 0x14, 0x16, 0x01, 0x4A, 0),
            (0x22, 0x0E, 0x14, 0x16, 0x01, 0x4A, 0), (0x13, 0x10, 0x14, 0x16, 0x01, 0x4A, 0)], 2, 1, 3),
]

# A sine-ish 8-bit sample for the DAC
PCM_WAVE = [128 + round(100 * ((i % 32) - 16) / 16 * (1 - abs((i % 32) - 16) / 32)) for i in range(256)]


def ym_setup(v, chip=0):
    v.ym(0, 0x22, 0x0B, chip)     # LFO on
    v.ym(0, 0x27, 0x00, chip)
    v.ym(0, 0x2B, 0x00, chip)
    for ch in range(6):
        ym_key(v, ch, 0, chip)
        alg, fb, ops, ams, pms, pan = YM_PATCHES[ch]
        ym_patch(v, ch, alg, fb, ops, ams, pms, pan, chip)


def vgm_fm():
    """The YM2612 (every algorithm, feedback, detune and multiple, the
    envelopes and SSG-EG, the LFO, channel 3's special and CSM modes, the
    timers, the DAC from a data block and by its register, the test
    registers, panning) with the PSG."""
    v = Vgm()
    ym_setup(v)
    psg_tone(v, 0, 0x1FC, 6)
    psg_tone(v, 1, 0x0FE, 10)
    v.psg(0x9F | 2 << 5, 0xFF)
    v.wait(300)
    # an arpeggio over the six channels, keyed in turn
    notes = [262, 330, 392, 523, 659, 784, 988, 1175]
    for step in range(12):
        for ch in range(6):
            f = notes[(step + ch) % 8] / (2 if ch < 2 else 1)
            block = 3 + ch % 3
            ym_note(v, ch, block, ym_fnum(f, block))
            if step % 3 == 0 or ch == step % 6:
                ym_key(v, ch, 0)
                ym_key(v, ch, 0xF if ch != 4 else 0x5)
            v.wait(40 + ch * 7)
        if step == 5:
            v.ym(0, 0x22, 0x0F)  # LFO faster
            psg_tone(v, 1, 0x150, 4)
            v.psg(0xE4, 0xF3)    # white noise
        if step == 8:
            v.ym(0, 0x22, 0x00)  # LFO off
            v.psg(0xE1, 0xF5)    # periodic noise
        v.wait(1500)
    # key off, releasing
    for ch in range(6):
        ym_key(v, ch, 0)
    v.wait(4000)
    # channel 3's special mode: an operator frequency each
    v.ym(0, 0x27, 0x40)
    for reg, block, f in ((0xAD, 4, 440), (0xAE, 5, 554), (0xAC, 3, 659)):
        v.ym(0, reg, block << 3 | ym_fnum(f, block) >> 8)
        v.ym(0, reg - 4, ym_fnum(f, block) & 0xFF)
    ym_note(v, 2, 4, ym_fnum(880, 4))
    ym_key(v, 2)
    v.wait(6000)
    # CSM: timer A keying channel 3, timer B running
    v.ym(0, 0x24, 0xF8)
    v.ym(0, 0x25, 0x02)
    v.ym(0, 0x26, 0xC0)
    v.ym(0, 0x27, 0x80 | 0x3F)
    v.wait(6000)
    v.ym(0, 0x27, 0x30)          # reset the flags, the timers off
    ym_key(v, 2, 0)
    v.wait(1000)
    # the DAC: a data block played with the PCM commands, a seek, and its
    # register written directly
    v.data_block(PCM_WAVE)
    v.data_block([1, 2, 3, 4], type_=1)  # not PCM
    v.ym(0, 0x2B, 0x80)
    v.pcm_seek(0)
    for i in range(400):
        v.pcm(i % 4 + 1)
    v.pcm_seek(64)
    for i in range(200):
        v.pcm(2)
    for i in range(300):
        v.ym(0, 0x2A, PCM_WAVE[(i * 3) % 256])
        v.wait(2)
    v.ym(0, 0x2B, 0x00)          # DAC off: the writes after are ignored
    for i in range(40):
        v.ym(0, 0x2A, PCM_WAVE[i])
        v.wait(3)
    v.ym(0, 0x2B, 0x80)
    for i in range(40):
        v.ym(0, 0x2A, 255 - PCM_WAVE[i])
        v.wait(3)
    v.ym(0, 0x2B, 0x00)
    v.mark_loop()
    # the other two algorithms, extreme notes, the test registers
    ym_patch(v, 0, 6, 4, YM_PATCHES[0][2], 1, 4, 3)
    ym_patch(v, 1, 7, 1, YM_PATCHES[2][2], 0, 0, 3)
    ym_note(v, 0, 7, 0x7FF)
    ym_note(v, 1, 1, 0x050)
    ym_note(v, 3, 6, 0x6A0)
    ym_key(v, 0)
    ym_key(v, 1)
    ym_key(v, 3)
    v.ym(0, 0x28, 0xF3)          # an invalid channel
    v.ym(0, 0x28, 0xF7)
    v.wait(3000)
    v.ym(0, 0x21, 0x04)          # timer test
    v.wait(200)
    v.ym(0, 0x21, 0x00)
    v.ym(0, 0x2C, 0x28)          # the DAC test
    v.wait(150)
    v.ym(0, 0x2C, 0x00)
    v.ym(0, 0x21, 0x10)          # FM output inverted
    v.wait(150)
    v.ym(0, 0x21, 0x00)
    v.wait(2000)
    psg_tone(v, 0, 0x0A0, 0)
    v.gg(0x5A)
    v.wait(2500)
    v.gg(0xFF)
    for ch in range(6):
        ym_key(v, ch, 0)
    v.wait(5000)
    v.end()
    tag = gd3('FM test', '', 'gen_gme_testdata.py', '', 'Sega Mega Drive', '', 'gen', '',
              '2026', 'gen_gme_testdata.py', 'every YM2612 feature')
    write('vgm_fm.vgm', vgm_file(v, version=0x150, ym2612=YM_CLOCK, tag=tag))


def vgm_psg():
    """The PSG alone (as Classic_Emu plays it): the squares, both noises
    and their rates, volumes, Game Gear stereo; a header's noise feedback
    and width, a data offset past junk, a loop and a GD3 tag in English and
    Japanese; with FM, PCM and unknown commands for chips not there."""
    v = Vgm()
    for ch, period, vol in ((0, 0x11C, 2), (1, 0x0E2, 5), (2, 0x0BE, 8)):
        psg_tone(v, ch, period, vol)
    v.psg(0xFF)
    v.wait(735)
    v.wait(882)
    v.gg(0xF0)
    v.wait(441)
    v.gg(0x0F)
    v.wait(441)
    v.gg(0xA5)
    v.wait(600)
    v.gg(0xFF)
    # sweeps, latching only the low bits sometimes
    for i in range(30):
        v.psg(0x80 | (i & 15))
        if i % 3:
            v.psg(0x08 + i)
        v.wait(13 + i % 4)
        v.wait(200)
    # the noises: periodic and white at each rate, and at square 3's
    for n in (0xE0, 0xE1, 0xE2, 0xE4, 0xE5, 0xE6):
        v.psg(n, 0xF1)
        v.wait(1200)
    psg_tone(v, 2, 0x020, 15)
    v.psg(0xE3, 0xF0)
    v.wait(1500)
    psg_tone(v, 2, 0x3FF, 15)
    v.psg(0xE7)
    v.wait(1500)
    v.mark_loop()
    # FM, PCM and unknown commands (for no chip, the YM2612 being off)
    v.ym(0, 0x2A, 0x80)
    v.ym(0, 0x28, 0xF0)
    v.ym(1, 0x30, 0x01)
    v.ym2413(0x10, 0x20)
    v.ym(0, 0x2B, 0x80)
    v.pcm(3)
    v.pcm(5)
    v.raw(0x40, 0x00)
    v.raw(0x55, 0x01, 0x02)
    v.raw(0xB0, 0x01, 0x02)
    v.raw(0xC0, 0x01, 0x02, 0x03)
    v.raw(0xD5, 0x01, 0x02, 0x03)
    v.raw(0xE1, 0x01, 0x02, 0x03, 0x04)
    v.raw(0xF0, 0x01, 0x02, 0x03, 0x04)
    v.raw(0x6F)
    v.data_block([9, 9, 9], type_=0x40)
    for i in range(20):
        psg_tone(v, i % 3, 0x80 + i * 20, i % 16)
        v.psg(0xF0 | (15 - i % 16))
        v.wait(1100)
    v.end()
    tag = gd3('PSG test', 'テスト', 'gen_gme_testdata.py', 'ゲーム',
              'Sega Game Gear', '', 'gen', '', '2026', 'gen_gme_testdata.py',
              'squares, noises, stereo')
    write('vgm_psg.vgm', vgm_file(v, version=0x150, noise=(0x0006, 15), tag=tag,
                                  extra=bytes(range(0x40, 0x54))))


def vgm_dual():
    """Two YM2612s and the second PSG's commands (a dual PSG's clock is
    limited before its flag is read); a YM2413 clock and a YM2151's
    alongside, data at its 1.51 offset 0."""
    v = Vgm()
    ym_setup(v, 0)
    ym_setup(v, 1)
    for i in range(16):
        for chip in (0, 1):
            ch = (i + chip * 3) % 6
            block = 4 + chip
            ym_note(v, ch, block, ym_fnum((330, 392, 494, 587)[i % 4], block), chip)
            ym_key(v, ch, 0, chip)
            ym_key(v, ch, 0xF, chip)
        v.psg(0x80 | (i & 15), 0x10, 0x91, chip=1)
        v.gg(0x33 if i % 2 else 0xCC, chip=1)
        if i == 6:
            v.ym(0, 0x2B, 0x80, chip=1)
            for k in range(60):
                v.ym(0, 0x2A, PCM_WAVE[k * 4 % 256], chip=1)
                v.wait(5)
            v.ym(0, 0x2B, 0x00, chip=1)
        v.wait(2500)
    v.end()
    write('vgm_dual.vgm', vgm_file(v, version=0x151, psg=0xC0000000 | 3579545,
                                   ym2612=0x40000000 | YM_CLOCK, ym2413=3579545,
                                   ym2151=3579545, rate=50, data_offset=0))


def vgm_ym2413():
    """A YM2413 (which SDL_mixer's build has no emulator for: opening it to
    play fails, its information reads)."""
    v = Vgm()
    v.ym2413(0x30, 0x10)
    v.ym2413(0x10, 0x80)
    v.ym2413(0x20, 0x15)
    v.wait(44100)
    v.end()
    tag = gd3('YM2413', '', 'gen_gme_testdata.py', '', 'Sega Master System', '', 'gen', '',
              '2026', 'gen', '')
    write('vgm_ym2413.vgm', vgm_file(v, version=0x110, ym2413=3579545, tag=tag))


def vgm_badrate():
    """A YM2612 whose clock is only a flag (0 Hz, dual)."""
    v = Vgm()
    ym_setup(v)
    ym_note(v, 0, 4, 0x400)
    ym_key(v, 0)
    v.wait(20000)
    v.end()
    write('vgm_badrate.vgm', vgm_file(v, version=0x150, ym2612=0x40000000))


def vgm_odd():
    """A 1.01 file (its YM2413 clock found to be a YM2612's by scanning the
    commands), a YM2612 at 100 kHz and a PSG at 1 Hz, a noise width past 32,
    a GD3 tag of an unknown version, a duration past 32 bits in
    milliseconds, a PCM seek past the data, and a loop to its own end."""
    v = Vgm()
    v.psg(0x9F, 0xBF)
    v.wait(100)
    v.gg(0x0F)
    v.data_block(PCM_WAVE[:64])
    v.raw(0x40, 0x00)
    ym_setup(v)
    for ch in range(6):
        ym_patch(v, ch, 7, 0, [(0x01, 0x00, 0x1F, 0x00, 0x00, 0x0F, 0)] * 4, 0, 0, 3)
        ym_note(v, ch, 4, 0x200 + ch * 0x40, )
        ym_key(v, ch)
    v.psg(0xE4, 0xF0)
    v.wait(12000)
    v.ym(0, 0x2B, 0x80)
    v.pcm_seek(0x10000)
    for i in range(50):
        v.pcm(3)
    v.pcm_seek(8)
    for i in range(50):
        v.pcm(3)
    v.wait(8000)
    v.ym(0, 0x2B, 0x00)
    for i in range(10):
        v.ym(0, 0x2A, 0x10 * i)
        v.wait(50)
    v.wait(30000)
    v.mark_loop()
    v.end()
    tag = gd3('Odd VGM', '', 'gen_gme_testdata.py', '', 'Sega', '', 'gen', '', '2026', 'gen',
              'unknown version', version=0x200)
    write('vgm_odd.vgm', vgm_file(v, version=0x101, psg=1, ym2413=100000, noise=(0x8005, 40),
                                  tag=tag, total=0xFFFFFFF0, loop_samples=1000))


def main():
    nsf_apu()
    nsf_banks()
    nsf_exp()
    nsf_vrc7()
    nsf_odd()
    nsfe_files()
    gbs_apu()
    gbs_timer()
    gbs_odd()
    ay_spectrum()
    ay_cpc()
    ay_odd()
    kss_scc()
    kss_sms()
    kss_odd()
    hes_psg()
    hes_odd()
    hes_multi()
    sap_b()
    sap_c()
    sap_odd()
    spc_dsp()
    spc_odd()
    vgm_fm()
    vgm_psg()
    vgm_dual()
    vgm_ym2413()
    vgm_badrate()
    vgm_odd()


if __name__ == '__main__':
    main()
