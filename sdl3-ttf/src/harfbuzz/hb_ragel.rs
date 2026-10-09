// The table-driven scanner that Ragel generates for HarfBuzz's syllable
// machines (hb-ot-shaper-*-machine.hh, from HarfBuzz 8.5.0, as SDL_ttf's
// external/harfbuzz pins it), written once for all of them.
// Copyright © 2011,2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! The Ragel scanner.
//!
//! Translation notes: Ragel emits the same driver for every machine
//! (`-T1` tables with a scanner's `ts`/`te`/`act` and EOF transitions);
//! only the tables, the codes of the "ts = p" and "ts = 0" state actions,
//! and the transition actions differ. This runs the driver with a
//! machine's tables and calls back for its transition actions, keeping
//! the `goto` flow of the generated C: `_resume`, `_eof_trans`, `_again`,
//! `_test_eof`.

/// A machine's tables (all widened to `u16`).
#[derive(Debug)]
pub(crate) struct RagelTables {
    pub(crate) trans_keys: &'static [u16],
    pub(crate) key_spans: &'static [u16],
    pub(crate) index_offsets: &'static [u16],
    pub(crate) indicies: &'static [u16],
    pub(crate) trans_targs: &'static [u16],
    pub(crate) trans_actions: &'static [u16],
    pub(crate) to_state_actions: &'static [u16],
    pub(crate) from_state_actions: &'static [u16],
    pub(crate) eof_trans: &'static [u16],
    pub(crate) start: i32,
    /// the from-state action code of `ts = p`
    pub(crate) from_state_action_ts: u16,
    /// the to-state action code of `ts = 0`
    pub(crate) to_state_action_ts: u16,
}

/// The scanner's variables the actions use (`p` may be moved back:
/// `p--`, `p = te - 1`, so it is signed).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RagelState {
    pub(crate) p: i64,
    pub(crate) ts: i64,
    pub(crate) te: i64,
    pub(crate) act: u32,
}

/// Runs the machine over `len` symbols, `symbol (p)` giving the symbol at
/// `p`, calling `action (code, state)` for each non-zero transition action.
pub(crate) fn ragel_exec(
    t: &RagelTables,
    len: usize,
    symbol: impl Fn(usize) -> u16,
    mut action: impl FnMut(u16, &mut RagelState),
) {
    let mut st = RagelState::default();
    let mut cs = t.start as usize;
    let pe = len as i64;
    let eof = pe;

    let mut trans: usize;

    if st.p == pe {
        /* goto _test_eof */
        if st.p == eof && t.eof_trans[cs] > 0 {
            trans = t.eof_trans[cs] as usize - 1;
        } else {
            return;
        }
    } else {
        trans = resume(t, cs, &mut st, &symbol);
    }

    loop {
        /* _eof_trans: */
        cs = t.trans_targs[trans] as usize;

        let a = t.trans_actions[trans];
        if a != 0 {
            action(a, &mut st);
        }

        /* _again: */
        if t.to_state_actions[cs] == t.to_state_action_ts {
            st.ts = 0;
        }

        st.p += 1;
        if st.p != pe {
            /* goto _resume */
            trans = resume(t, cs, &mut st, &symbol);
            continue;
        }
        /* _test_eof: */
        if st.p == eof && t.eof_trans[cs] > 0 {
            trans = t.eof_trans[cs] as usize - 1;
            continue;
        }
        break;
    }
}

/// `_resume`: the from-state action and the transition on the symbol at
/// `p`.
#[inline]
fn resume(
    t: &RagelTables,
    cs: usize,
    st: &mut RagelState,
    symbol: &impl Fn(usize) -> u16,
) -> usize {
    if t.from_state_actions[cs] == t.from_state_action_ts {
        st.ts = st.p;
    }

    let keys = &t.trans_keys[cs << 1..];
    let inds = &t.indicies[t.index_offsets[cs] as usize..];

    let slen = t.key_spans[cs];
    let c = symbol(st.p as usize);
    let i = if slen > 0 && keys[0] <= c && c <= keys[1] {
        c - keys[0]
    } else {
        slen
    };
    inds[i as usize] as usize
}
