#!/usr/bin/env python3
"""Generate sdl3-mixer-gme's test data (sdl3-mixer-gme/testdata/).

Every file here is synthetic, made by this script: tiny game music rips
whose sound programs are assembled here (for the 6502 of the NSF and NSFE
files, the Game Boy CPU of the GBS files, the Z80 of the AY and KSS
files), driving the sound chips the emulators emulate, with their
metadata, track lists and timing. Nothing comes from a real game.

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


if __name__ == '__main__':
    main()
