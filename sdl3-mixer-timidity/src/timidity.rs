/*
    TiMidity -- Experimental MIDI to WAVE converter
    Copyright (C) 1995 Tuukka Toivonen <toivonen@clinet.fi>

    This program is free software; you can redistribute it and/or modify
    it under the terms of the Perl Artistic License, available in COPYING.
*/
// Modified 2026-10-07: translated into Rust from timidity.c in SDL_mixer 3.3.0's TiMidity (see NOTICE).

//! The configuration file, and loading and freeing songs. Translation of
//! `timidity.c`.

use std::sync::{Mutex, MutexGuard};

use sdl3::audio::{AudioFormat, AudioSpec};
use sdl3::error::{Error, Result};
use sdl3::io::IoStream;

use crate::common::{atoi, timi_add_pathlist, timi_free_pathlist, timi_openfile};
use crate::instrum::{free_instruments, load_missing_instruments, set_default_instrument};
use crate::options::*;
use crate::output::{PE_16BIT, PE_32BIT, PE_MONO, PE_SIGNED};
use crate::readmidi::read_midi_file;
use crate::sndfont::{
    end_sbk, end_soundfont, exclude_soundfont, init_sbk, init_soundfont, order_soundfont, SfState,
};
use crate::{
    Channel, MidiSong, ToneBank, ToneBankElement, ToneSrc, Voice, WriteFn, MAXBANK, MAXCHAN,
    MAX_VOICES,
};

/// The configuration and the SoundFont: upstream's `static` variables in
/// `timidity.c` and `sndfont.c`, behind one lock.
pub(crate) struct Globals {
    /// `master_tonebank[i]->tone` (the configuration's banks have no
    /// instruments of their own).
    pub master_tonebank: [Option<Vec<ToneBankElement>>; MAXBANK],
    /// `master_drumset[i]->tone`.
    pub master_drumset: [Option<Vec<ToneBankElement>>; MAXBANK],
    pub def_instr_name: Vec<u8>,
    pub sf_file: Option<Vec<u8>>,
    pub sf_order: i32,
    pub sf: SfState,
}

static GLOBALS: Mutex<Globals> = Mutex::new(Globals {
    master_tonebank: [const { None }; MAXBANK],
    master_drumset: [const { None }; MAXBANK],
    def_instr_name: Vec::new(),
    sf_file: None,
    sf_order: 0,
    sf: SfState::new(),
});

fn globals() -> MutexGuard<'static, Globals> {
    GLOBALS.lock().unwrap_or_else(|e| e.into_inner())
}

/// A song's bank's `tone` array: its own, or the configuration's.
pub(crate) fn tones<'a>(
    bank: &'a mut ToneBank,
    master: &'a mut Option<Vec<ToneBankElement>>,
) -> Option<&'a mut Vec<ToneBankElement>> {
    match &mut bank.tone {
        ToneSrc::Own(tone) => Some(tone),
        ToneSrc::Master => master.as_mut(),
    }
}

/// `SDL_calloc(128, sizeof(ToneBankElement))`.
fn new_tones() -> Vec<ToneBankElement> {
    vec![ToneBankElement::default(); 128]
}

const MAXWORDS: usize = 10;
const MAX_RCFCOUNT: i32 = 50;

/* Quick-and-dirty fgets() replacement. */

/// Translation of `IOgets()`: reads a line (without its newline, at most
/// `size - 1` bytes) into `s`, NUL-terminated.
fn iogets(io: &mut IoStream<'_>, s: &mut [u8]) -> bool {
    let mut num_read = 0;
    let mut p = 0;
    let size = s.len() - 1; /* so that we nul terminate properly */

    while num_read < size {
        let mut c = [0u8; 1];
        if io.read(&mut c) != 1 {
            break;
        }
        s[p] = c[0];

        num_read += 1;

        /* Unlike fgets(), don't store newline. Under Windows/DOS we'll
         * probably get an extra blank line for every line that's being
         * read, but that should be ok.
         */
        if s[p] == b'\n' || s[p] == b'\r' {
            s[p] = 0;
            return true;
        }
        p += 1;
    }

    s[p] = 0;

    num_read != 0
}

/// `strlen()` from `i`.
fn cstr(buf: &[u8], i: usize) -> &[u8] {
    let end = buf[i..]
        .iter()
        .position(|&c| c == 0)
        .map_or(buf.len(), |n| i + n);
    &buf[i..end]
}

/// `strchr(s, c)` from `i` (the index of `c`, before the NUL).
fn strchr(buf: &[u8], i: usize, c: u8) -> Option<usize> {
    cstr(buf, i).iter().position(|&x| x == c).map(|n| i + n)
}

fn is_delim(c: u8) -> bool {
    c == b' ' || c == b'\t' || c == 0xA0 /* '\240' */
}

/// `strtok_r(s, " \t\240", &endp)` (glibc's): the token's start, if any,
/// and the new `endp`.
fn strtok_r(buf: &mut [u8], mut s: usize) -> (Option<usize>, usize) {
    while s < buf.len() && buf[s] != 0 && is_delim(buf[s]) {
        s += 1;
    }
    if s >= buf.len() || buf[s] == 0 {
        return (None, s);
    }
    let token = s;
    while s < buf.len() && buf[s] != 0 && !is_delim(buf[s]) {
        s += 1;
    }
    if s >= buf.len() || buf[s] == 0 {
        return (Some(token), s);
    }
    buf[s] = 0;
    (Some(token), s + 1)
}

/// Which bank `read_config_file()`'s `bank` points to.
#[derive(Clone, Copy)]
enum BankRef {
    Tone(usize),
    Drum(usize),
}

fn read_config_file(g: &mut Globals, name: &[u8], rcf_count: i32) -> i32 {
    if rcf_count >= MAX_RCFCOUNT {
        crate::snddbg!("Probable source loop in configuration files\n");
        return -1;
    }

    let Some(mut io) = timi_openfile(name) else {
        return -1;
    };

    let mut tmp = [0u8; 1024];
    let mut bank: Option<BankRef> = None;
    let mut line = 0;
    let mut r = -1; /* start by assuming failure, */

    // (`break 'lines true` is `goto fail`.)
    let failed = 'lines: loop {
        if !iogets(&mut io, &mut tmp) {
            break 'lines false;
        }
        line += 1;
        let mut words: usize = 0;
        let mut w: Vec<usize> = Vec::with_capacity(MAXWORDS);
        let (w0, mut endp) = strtok_r(&mut tmp, 0);
        let Some(mut w0) = w0 else {
            continue;
        };

        /* Originally the TiMidity++ extensions were prefixed like this */
        if cstr(&tmp, w0) == b"#extension" {
            let (t, e) = strtok_r(&mut tmp, endp);
            endp = e;
            let Some(t) = t else {
                continue;
            };
            w0 = t;
        }

        if tmp[w0] == b'#' {
            continue;
        }
        w.push(w0);

        // FIXME (upstream): a line with nine or more arguments writes its
        // terminating NULL past the end of w[].
        while words < MAXWORDS - 1 {
            /* -1 : next arg */
            while is_delim(tmp[endp]) {
                endp += 1;
            }

            if tmp[endp] == 0 || tmp[endp] == b'#' {
                break;
            }

            if tmp[endp] == b'"' || tmp[endp] == b'\'' {
                /* quoted string */
                let quote = tmp[endp];
                if let Some(terminator) = strchr(&tmp, endp + 1, quote) {
                    /* terminated */
                    let after = tmp[terminator + 1];
                    if is_delim(after) || after == 0 {
                        let other = if quote == b'"' { b'\'' } else { b'"' };
                        if let Some(extra_quote) = strchr(&tmp, endp + 1, other) {
                            if extra_quote < terminator {
                                crate::snddbg!(
                                    "{}: line {}: Quote characters are not allowed inside a quoted string",
                                    String::from_utf8_lossy(name), line
                                );
                                break 'lines true;
                            }
                        }
                        words += 1;
                        w.push(endp + 1);
                        endp = terminator + 1;
                        tmp[terminator] = 0;
                    } else {
                        /* no space after quoted string */
                        crate::snddbg!("{}: line {}: There must be at least one whitespace between string terminator ({}) and the next parameter", String::from_utf8_lossy(name), line, quote as char);
                        break 'lines true;
                    }
                } else {
                    /* not terminated */
                    crate::snddbg!(
                        "{}: line {}: The quoted string is not terminated",
                        String::from_utf8_lossy(name),
                        line
                    );
                    break 'lines true;
                }
            } else {
                /* not quoted string */
                words += 1;
                w.push(endp);
                while !(is_delim(tmp[endp]) || tmp[endp] == 0) {
                    if tmp[endp] == b'"' || tmp[endp] == b'\'' {
                        /* no space before quoted string */
                        crate::snddbg!("{}: line {}: There must be at least one whitespace between previous parameter and a beginning of the quoted string ({})", String::from_utf8_lossy(name), line, tmp[endp] as char);
                        break 'lines true;
                    }
                    endp += 1;
                }
                if tmp[endp] != 0 {
                    /* unless at the end-of-string (i.e. EOF) */
                    tmp[endp] = 0; /* terminate the token */
                    endp += 1;
                }
            }
        }
        words += 1; // w[++words] = NULL;

        let wd = |i: usize| -> Vec<u8> { cstr(&tmp, w[i]).to_vec() };
        let w0 = wd(0);
        let w0 = w0.as_slice();

        /* TiMidity++ adds a number of extensions to the config file format.
         * Many of them are completely irrelevant to SDL_sound, but at least
         * we shouldn't choke on them.
         *
         * Unfortunately the documentation for these extensions is often quite
         * vague, gramatically strange or completely absent.
         */
        if w0 == b"comm"      /* "comm" program second        */
            || w0 == b"HTTPproxy" /* "HTTPproxy" hostname:port    */
            || w0 == b"FTPproxy"  /* "FTPproxy" hostname:port     */
            || w0 == b"mailaddr"  /* "mailaddr" your-mail-address */
            || w0 == b"opt"
        /* "opt" timidity-options       */
        {
            /* + "comm" sets some kind of comment -- the documentation is too
             *   vague for me to understand at this time.
             * + "HTTPproxy", "FTPproxy" and "mailaddr" are for reading data
             *   over a network, rather than from the file system.
             * + "opt" specifies default options for TiMidity++.
             *
             * Quite useless for us, so they can safely remain no-ops.
             */
        } else if w0 == b"timeout" {
            /* "timeout" program second */
            /* Specifies a timeout value of the program. A number of seconds
             * before TiMidity kills the note. No urgent need for it.
             */
            crate::snddbg!("FIXME: Implement \"timeout\" in TiMidity config.\n");
        } else if w0 == b"copydrumset"  /* "copydrumset" drumset */
            || w0 == b"copybank"
        /* "copybank" bank       */
        {
            /* Copies all the settings of the specified drumset or bank to
             * the current drumset or bank. May be useful later, but not a
             * high priority.
             */
            crate::snddbg!(
                "FIXME: Implement \"{}\" in TiMidity config.\n",
                String::from_utf8_lossy(w0)
            );
        } else if w0 == b"undef" {
            /* "undef" progno */
            /* Undefines the tone "progno" of the current tone bank (or
             * drum set?). Not a high priority.
             */
            crate::snddbg!("FIXME: Implement \"undef\" in TiMidity config.\n");
        } else if w0 == b"altassign" {
            /* "altassign" prog1 prog2 ... */
            /* Sets the alternate assign for drum set. Whatever that's
             * supposed to mean.
             */
            crate::snddbg!("FIXME: Implement \"altassign\" in TiMidity config.\n");
        } else if w0 == b"progbase" {
            /* The documentation for this makes absolutely no sense to me, but
             * apparently it sets some sort of base offset for tone numbers.
             */
            crate::snddbg!("FIXME: Implement \"progbase\" in TiMidity config.\n");
        } else if w0 == b"map" {
            /* "map" name set1 elem1 set2 elem2 */
            /* This one is used by the "eawpats". Looks like it's used
             * for remapping one instrument to another somehow.
             */
            crate::snddbg!("FIXME: Implement \"map\" in TiMidity config.\n");
        }
        /* Standard TiMidity config */
        else if w0 == b"dir" {
            if words < 2 {
                crate::snddbg!(
                    "{}: line {}: No directory given\n",
                    String::from_utf8_lossy(name),
                    line
                );
                break 'lines true;
            }
            for i in 1..words {
                if timi_add_pathlist(&wd(i)) < 0 {
                    break 'lines true;
                }
            }
        } else if w0 == b"source" {
            if words < 2 {
                crate::snddbg!(
                    "{}: line {}: No file name given\n",
                    String::from_utf8_lossy(name),
                    line
                );
                break 'lines true;
            }
            for i in 1..words {
                r = read_config_file(g, &wd(i), rcf_count + 1);
                if r != 0 {
                    break 'lines true;
                }
            }
            r = -1; /* not finished yet, */
        } else if w0 == b"default" {
            if words != 2 {
                crate::snddbg!(
                    "{}: line {}: Must specify exactly one patch name\n",
                    String::from_utf8_lossy(name),
                    line
                );
                break 'lines true;
            }
            // SDL_strlcpy(def_instr_name, w[1], 256)
            let mut n = wd(1);
            n.truncate(255);
            g.def_instr_name = n;
        } else if w0 == b"drumset" {
            if words < 2 {
                crate::snddbg!(
                    "{}: line {}: No drum set number given\n",
                    String::from_utf8_lossy(name),
                    line
                );
                break 'lines true;
            }
            let i = atoi(&wd(1));
            if i < 0 || i > (MAXBANK as i32 - 1) {
                crate::snddbg!(
                    "{}: line {}: Drum set must be between 0 and {}\n",
                    String::from_utf8_lossy(name),
                    line,
                    MAXBANK - 1
                );
                break 'lines true;
            }
            if g.master_drumset[i as usize].is_none() {
                g.master_drumset[i as usize] = Some(new_tones());
            }
            bank = Some(BankRef::Drum(i as usize));
        } else if w0 == b"bank" {
            if words < 2 {
                crate::snddbg!(
                    "{}: line {}: No bank number given\n",
                    String::from_utf8_lossy(name),
                    line
                );
                break 'lines true;
            }
            let i = atoi(&wd(1));
            if i < 0 || i > (MAXBANK as i32 - 1) {
                crate::snddbg!(
                    "{}: line {}: Tone bank must be between 0 and {}\n",
                    String::from_utf8_lossy(name),
                    line,
                    MAXBANK - 1
                );
                break 'lines true;
            }
            if g.master_tonebank[i as usize].is_none() {
                g.master_tonebank[i as usize] = Some(new_tones());
            }
            bank = Some(BankRef::Tone(i as usize));
        } else if w0 == b"soundfont" {
            if words < 2 {
                crate::snddbg!(
                    "{}: line {}: No soundfont file given\n",
                    String::from_utf8_lossy(name),
                    line
                );
                break 'lines true;
            }
            if g.sf_file.is_some() {
                crate::snddbg!(
                    "{}: line {}: Ignoring multiple \"soundfont\" directives.\n",
                    String::from_utf8_lossy(name),
                    line
                );
            } else {
                if set_soundfont_locked(g, Some(&wd(1))) < 0 {
                    break 'lines true;
                }
                for j in 2..words {
                    let wj = wd(j);
                    let Some(eq) = wj.iter().position(|&c| c == b'=') else {
                        crate::snddbg!(
                            "{}: line {}: bad patch option {}\n",
                            String::from_utf8_lossy(name),
                            line,
                            String::from_utf8_lossy(&wj)
                        );
                        end_sbk(g);
                        break 'lines true;
                    };
                    let (key, cp) = (&wj[..eq], &wj[eq + 1..]);
                    if key == b"order" {
                        let k = atoi(cp);
                        let c0 = cp.first().copied().unwrap_or(0);
                        if k < 0 || !c0.is_ascii_digit() {
                            crate::snddbg!(
                                "{}: line {}: order must be a digit",
                                String::from_utf8_lossy(name),
                                line
                            );
                            end_sbk(g);
                            break 'lines true;
                        }
                        g.sf_order = k;
                    }
                }
            }
        } else if w0 == b"font" {
            if words < 2 {
                crate::snddbg!(
                    "{}: line {}: no font command\n",
                    String::from_utf8_lossy(name),
                    line
                );
                break 'lines true;
            }
            let w1 = wd(1);
            if w1 == b"exclude" {
                if words < 3 {
                    crate::snddbg!(
                        "{}: line {}: No bank/preset/key is given\n",
                        String::from_utf8_lossy(name),
                        line
                    );
                    break 'lines true;
                }
                let bank = atoi(&wd(2));
                let preset = if words >= 4 { atoi(&wd(3)) } else { -1 };
                let keynote = if words >= 5 { atoi(&wd(4)) } else { -1 };
                if exclude_soundfont(g, bank, preset, keynote) < 0 {
                    break 'lines true;
                }
            } else if w1 == b"order" {
                if words < 4 {
                    crate::snddbg!(
                        "{}: line {}: No order/bank is given\n",
                        String::from_utf8_lossy(name),
                        line
                    );
                    break 'lines true;
                }
                let order = atoi(&wd(2));
                let bank = atoi(&wd(3));
                let preset = if words >= 5 { atoi(&wd(4)) } else { -1 };
                let keynote = if words >= 6 { atoi(&wd(5)) } else { -1 };
                if order_soundfont(g, bank, preset, keynote, order) < 0 {
                    break 'lines true;
                }
            }
        } else {
            if (words < 2) || !w0[0].is_ascii_digit() {
                crate::snddbg!(
                    "{}: line {}: syntax error\n",
                    String::from_utf8_lossy(name),
                    line
                );
                break 'lines true;
            }
            let i = atoi(w0);
            if !(0..=127).contains(&i) {
                crate::snddbg!(
                    "{}: line {}: Program must be between 0 and 127\n",
                    String::from_utf8_lossy(name),
                    line
                );
                break 'lines true;
            }
            let Some(bank) = bank else {
                crate::snddbg!(
                    "{}: line {}: Must specify tone bank or drum set before assignment\n",
                    String::from_utf8_lossy(name),
                    line
                );
                break 'lines true;
            };
            let tones = match bank {
                BankRef::Tone(b) => g.master_tonebank[b].get_or_insert_with(new_tones),
                BankRef::Drum(b) => g.master_drumset[b].get_or_insert_with(new_tones),
            };
            let tone = &mut tones[i as usize];
            tone.name = Some(wd(1));
            tone.note = -1;
            tone.amp = -1;
            tone.pan = -1;
            tone.strip_loop = -1;
            tone.strip_envelope = -1;
            tone.strip_tail = -1;

            for j in 2..words {
                let wj = wd(j);
                let Some(eq) = wj.iter().position(|&c| c == b'=') else {
                    crate::snddbg!(
                        "{}: line {}: bad patch option {}\n",
                        String::from_utf8_lossy(name),
                        line,
                        String::from_utf8_lossy(&wj)
                    );
                    break 'lines true;
                };
                let (key, cp) = (&wj[..eq], &wj[eq + 1..]);
                let c0 = cp.first().copied().unwrap_or(0);
                if key == b"amp" {
                    let k = atoi(cp);
                    if !(0..=MAX_AMPLIFICATION).contains(&k) || !c0.is_ascii_digit() {
                        crate::snddbg!(
                            "{}: line {}: amplification must be between 0 and {}\n",
                            String::from_utf8_lossy(name),
                            line,
                            MAX_AMPLIFICATION
                        );
                        break 'lines true;
                    }
                    tone.amp = k;
                } else if key == b"note" {
                    let k = atoi(cp);
                    if !(0..=127).contains(&k) || !c0.is_ascii_digit() {
                        crate::snddbg!(
                            "{}: line {}: note must be between 0 and 127\n",
                            String::from_utf8_lossy(name),
                            line
                        );
                        break 'lines true;
                    }
                    tone.note = k;
                } else if key == b"pan" {
                    let k = if cp == b"center" {
                        64
                    } else if cp == b"left" {
                        0
                    } else if cp == b"right" {
                        127
                    } else {
                        (atoi(cp).wrapping_add(100)).wrapping_mul(100) / 157
                    };
                    if !(0..=127).contains(&k) || (k == 0 && c0 != b'-' && !c0.is_ascii_digit()) {
                        crate::snddbg!("{}: line {}: panning must be left, right, center, or between -100 and 100\n", String::from_utf8_lossy(name), line);
                        break 'lines true;
                    }
                    tone.pan = k;
                } else if key == b"keep" {
                    if cp == b"env" {
                        tone.strip_envelope = 0;
                    } else if cp == b"loop" {
                        tone.strip_loop = 0;
                    } else {
                        crate::snddbg!(
                            "{}: line {}: keep must be env or loop\n",
                            String::from_utf8_lossy(name),
                            line
                        );
                        break 'lines true;
                    }
                } else if key == b"strip" {
                    if cp == b"env" {
                        tone.strip_envelope = 1;
                    } else if cp == b"loop" {
                        tone.strip_loop = 1;
                    } else if cp == b"tail" {
                        tone.strip_tail = 1;
                    } else {
                        crate::snddbg!(
                            "{}: line {}: strip must be env, loop, or tail\n",
                            String::from_utf8_lossy(name),
                            line
                        );
                        break 'lines true;
                    }
                } else {
                    crate::snddbg!(
                        "{}: line {}: bad patch option {}\n",
                        String::from_utf8_lossy(name),
                        line,
                        String::from_utf8_lossy(&wj)
                    );
                    break 'lines true;
                }
            }
        }
    };

    let _ = line;

    if !failed {
        r = 0; /* we're good. */
    }
    // fail: SDL_CloseIO(io);
    r
}

#[cfg(windows)]
/* FIXME: What about C:FOO ? */
fn get_last_dirsep(p: &[u8]) -> Option<usize> {
    p.iter().rposition(|&c| c == b'/' || c == b'\\')
}
#[cfg(not(windows))] /* assumed UNIX-ish : */
fn get_last_dirsep(p: &[u8]) -> Option<usize> {
    p.iter().rposition(|&c| c == b'/')
}

fn init_alloc_banks(g: &mut Globals) -> i32 {
    /* Allocate memory for the standard tonebank and drumset */
    g.master_tonebank[0] = Some(new_tones());
    g.master_drumset[0] = Some(new_tones());
    0
}

fn init_begin_config(cf: &[u8]) -> i32 {
    if let Some(p) = get_last_dirsep(cf) {
        return timi_add_pathlist(&cf[..p + 1]); /* including DIRSEP */
    }
    0
}

fn init_with_config(g: &mut Globals, cf: &[u8]) -> i32 {
    let mut rc = init_begin_config(cf);
    if rc != 0 {
        exit_locked(g);
        return rc;
    }
    rc = read_config_file(g, cf, 0);
    if rc != 0 {
        exit_locked(g);
    }
    rc
}

/// `Timidity_*`'s return codes as a `Result`.
fn result(rc: i32) -> Result<()> {
    match rc {
        0 => Ok(()),
        -2 => Err(Error::out_of_memory()),
        _ => Err(Error::new("TiMidity: couldn't read the configuration")),
    }
}

fn init_no_config_locked(g: &mut Globals) -> i32 {
    g.master_tonebank[0] = None;
    g.master_drumset[0] = None;
    init_alloc_banks(g)
}

/// Initialize without a configuration file (with empty tone bank and
/// drum set 0). Translation of `Timidity_Init_NoConfig()`.
pub fn init_no_config() -> Result<()> {
    result(init_no_config_locked(&mut globals()))
}

/// Initialize: read the configuration file (`"timidity.cfg"` when `None`
/// or empty), unless a SoundFont was set with [`set_soundfont`]. The
/// configuration file's directory is searched for the files it names.
/// Translation of `Timidity_Init()`.
///
/// On failure everything is reset, as by [`exit`].
pub fn init(config_file: Option<&str>) -> Result<()> {
    let mut g = globals();
    let rc = init_no_config_locked(&mut g);
    if rc != 0 {
        return result(rc);
    }
    if g.sf_file.is_some() {
        /* a soundfont specified by mid_set_soundfont().
         * skip config parsing. */
        return Ok(());
    }
    match config_file {
        None | Some("") => result(init_with_config(&mut g, TIMIDITY_CFG.as_bytes())),
        Some(cf) => result(init_with_config(&mut g, cf.as_bytes())),
    }
}

fn set_soundfont_locked(g: &mut Globals, file: Option<&[u8]>) -> i32 {
    if g.sf_file.is_some() {
        /* just in case ... */
        end_sbk(g);
        g.sf_file = None;
    }
    if let Some(file) = file {
        if init_sbk(g, file) < 0 {
            return -1;
        }
        g.sf_file = Some(file.to_vec());
    }
    0
}

/// Set the full path of a SoundFont (SF2, or SBK) to use, and do a
/// preliminary load of it. This must be called before [`init`], which
/// then doesn't read a configuration file. If loading fails, the
/// SoundFont is not set. Translation of `Timidity_SetSoundfont()`.
pub fn set_soundfont(sf2_file: Option<&str>) -> Result<()> {
    match set_soundfont_locked(&mut globals(), sf2_file.map(str::as_bytes)) {
        0 => Ok(()),
        _ => Err(Error::new("TiMidity: couldn't load the SoundFont")),
    }
}

impl MidiSong {
    /// `SDL_calloc(1, sizeof(*song))`.
    fn new() -> MidiSong {
        MidiSong {
            oom: 0,
            playing: 0,
            rate: 0,
            encoding: 0,
            master_volume: 0.0,
            amplification: 0,
            tonebank: [const { None }; MAXBANK],
            drumset: [const { None }; MAXBANK],
            default_instrument: None,
            default_program: 0,
            write: WriteFn::S16L,
            buffer_size: 0,
            resample_buffer: Vec::new(),
            common_buffer: Vec::new(),
            buffer_pointer: 0,
            sample_increment: 0,
            sample_correction: 0,
            channel: [Channel::default(); MAXCHAN],
            voice: vec![Voice::default(); MAX_VOICES],
            voices: 0,
            drumchannels: 0,
            buffered_count: 0,
            control_ratio: 0,
            lost_notes: 0,
            cut_notes: 0,
            samples: 0,
            events: Vec::new(),
            current_event: 0,
            evlist: Vec::new(),
            current_sample: 0,
            event_count: 0,
            at: 0,
            groomed_event_count: 0,
        }
    }
}

/// `SDL_malloc(n * sizeof(T))`, zeroed (upstream's isn't), failing for a
/// negative or impossible size.
fn alloc<T: Clone + Default>(n: i64) -> Option<Vec<T>> {
    let n = usize::try_from(n).ok()?;
    let mut v = Vec::new();
    v.try_reserve_exact(n).ok()?;
    v.resize(n, T::default());
    Some(v)
}

fn do_song_load(
    io: &mut IoStream<'_>,
    audio: &AudioSpec,
    samples: i32,
) -> std::result::Result<MidiSong, Option<Error>> {
    let mut g = globals();
    let g = &mut *g;

    /* Allocate memory for the song */
    let mut song = MidiSong::new();

    for i in 0..MAXBANK {
        if g.master_tonebank[i].is_some() {
            song.tonebank[i] = Some(ToneBank::new(ToneSrc::Master));
        }
        if g.master_drumset[i].is_some() {
            song.drumset[i] = Some(ToneBank::new(ToneSrc::Master));
        }
    }

    song.amplification = DEFAULT_AMPLIFICATION;
    song.voices = DEFAULT_VOICES;
    song.drumchannels = DEFAULT_DRUMCHANNELS;

    song.rate = audio.freq;
    if song.rate <= 0 {
        // FIXME (upstream): a rate of 0 divides by zero (and a negative
        // one reads outside the samples).
        return Err(Some(Error::invalid_param("audio->freq")));
    }
    song.encoding = 0;
    let format = audio.format.0;
    if (format & 0xFF) == 16 {
        song.encoding |= PE_16BIT;
    } else if (format & 0xFF) == 32 {
        song.encoding |= PE_32BIT;
    }
    if format & 0x8000 != 0 {
        song.encoding |= PE_SIGNED;
    }
    if audio.channels == 1 {
        song.encoding |= PE_MONO;
    } else if audio.channels > 2 {
        return Err(Some(Error::new("Surround sound not supported")));
    }
    song.write = match audio.format {
        AudioFormat::S8 => WriteFn::S8,
        AudioFormat::U8 => WriteFn::U8,
        AudioFormat::S16LE => WriteFn::S16L,
        AudioFormat::S16BE => WriteFn::S16B,
        AudioFormat::S32LE => WriteFn::S32L,
        AudioFormat::S32BE => WriteFn::S32B,
        AudioFormat::F32LE => WriteFn::F32L,
        AudioFormat::F32BE => WriteFn::F32B,
        _ => return Err(Some(Error::new("Unsupported audio format"))),
    };

    song.buffer_size = samples;
    // FIXME (upstream): rs_plain() can write one sample past the resample
    // buffer; it has room for it here.
    song.resample_buffer = alloc(samples as i64 + 1).ok_or(None)?;
    song.common_buffer = alloc(samples as i64 * 2).ok_or(None)?;

    song.control_ratio = audio.freq / CONTROLS_PER_SECOND;
    if song.control_ratio < 1 {
        song.control_ratio = 1;
    } else if song.control_ratio > MAX_CONTROL_RATIO {
        song.control_ratio = MAX_CONTROL_RATIO;
    }

    song.lost_notes = 0;
    song.cut_notes = 0;

    let mut count = 0;
    let mut nsamples = 0;
    let events = read_midi_file(&mut song, io, &mut count, &mut nsamples);
    song.groomed_event_count = count;
    song.samples = nsamples;

    /* Make sure everything is okay */
    let Some(events) = events else {
        return Err(oom_error(&song));
    };
    song.events = events;

    song.default_instrument = None;
    song.default_program = DEFAULT_PROGRAM;

    if g.sf_file.is_some() {
        let order = g.sf_order;
        if init_soundfont(&mut song, g, order) < 0 {
            return Err(oom_error(&song));
        }
    }

    if !g.def_instr_name.is_empty() {
        let name = g.def_instr_name.clone();
        set_default_instrument(&mut song, &name);
    }

    load_missing_instruments(&mut song, g);

    if song.oom == 0 {
        Ok(song)
    } else {
        Err(oom_error(&song))
    }
}

/// The error a failed load reports: upstream sets none (but when it runs
/// out of memory).
fn oom_error(song: &MidiSong) -> Option<Error> {
    if song.oom != 0 {
        Some(Error::out_of_memory())
    } else {
        None
    }
}

impl MidiSong {
    /// Read a MIDI file (or a RIFF RMID file) from `io`, and load the
    /// instruments it plays, to render `audio`'s format (8, 16 or 32-bit
    /// integers or 32-bit floats, mono or stereo) `samples` sample frames
    /// at a time. Translation of `Timidity_LoadSong()`.
    ///
    /// [`init`] must have been called. Fails with "Surround sound not
    /// supported" or "Unsupported audio format" for those, and otherwise
    /// (where upstream sets no error) with an error saying the song
    /// couldn't be loaded.
    pub fn load(io: &mut IoStream<'_>, audio: &AudioSpec, samples: i32) -> Result<MidiSong> {
        do_song_load(io, audio, samples)
            .map_err(|e| e.unwrap_or_else(|| Error::new("TiMidity: couldn't load the MIDI file")))
    }
}

/// Translation of `Timidity_FreeSong()`.
impl Drop for MidiSong {
    fn drop(&mut self) {
        free_instruments(self);
    }
}

// (Everything else the song has is
// freed with it; the configuration's tone banks aren't the song's.)

fn exit_locked(g: &mut Globals) {
    for i in 0..MAXBANK {
        g.master_tonebank[i] = None;
        g.master_drumset[i] = None;
    }

    end_soundfont(g);
    end_sbk(g);
    g.sf_file = None;
    g.sf_order = 0;

    timi_free_pathlist();
}

/// Free the configuration (and the SoundFont, and the search path).
/// Translation of `Timidity_Exit()`.
pub fn exit() {
    exit_locked(&mut globals());
}
