// Rust translation of src/truetype/ttinterp.c (and ttinterp.h) from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! TrueType bytecode interpreter (body).
//!
//! Greg Hitchcock from Microsoft has helped a lot in resolving unclear
//! issues; many thanks!
//!
//! Translation notes:
//!
//! - C's execution context shares the size object's arrays (function and
//!   instruction definitions, CVT, storage, twilight zone) by pointer.
//!   Here [`tt_load_context`] moves them from the size into the context
//!   and [`tt_unload_context`] (which has no C counterpart) moves them
//!   back once a program has run; the size's twilight point count, which
//!   `TT_RunIns` may lower in the context's copy only, is restored then.
//! - The zone pointers `zp0`, `zp1`, and `zp2` are [`ZoneRef`] selectors
//!   of the context's twilight or glyph (`pts`) zone; C copies the zone
//!   records, which share the point arrays.
//! - The code ranges hold reference-counted copies of the programs.
//! - What the interpreter needs from the face (the interpreter version,
//!   the glyph count, and the blend coordinates) is copied into the
//!   context by [`tt_load_context`].
//! - `TT_MulFix14` and `TT_DotFix14` are the `long long` versions GCC uses
//!   on x86 and x86_64; like C, they take `FT_Int32` arguments.

use std::sync::Arc;

use super::super::base::ftcalc::*;
use super::super::fttypes::*;
use super::super::tttypes::*;
use super::ttobjs::*;

/*
 *
 * Rounding mode constants.
 */
pub const TT_ROUND_OFF: FtInt = 5;
pub const TT_ROUND_TO_HALF_GRID: FtInt = 0;
pub const TT_ROUND_TO_GRID: FtInt = 1;
pub const TT_ROUND_TO_DOUBLE_GRID: FtInt = 2;
pub const TT_ROUND_UP_TO_GRID: FtInt = 4;
pub const TT_ROUND_DOWN_TO_GRID: FtInt = 3;
pub const TT_ROUND_SUPER: FtInt = 6;
pub const TT_ROUND_SUPER_45: FtInt = 7;

/*
 *
 * Function types used by the interpreter, depending on various modes
 * (e.g. the rounding mode, whether to render a vertical or horizontal
 * line etc).
 *
 */

/// `TT_Round_Func`: Rounding function
pub type TtRoundFunc = fn(exc: &TtExecContextRec, distance: FtF26Dot6, color: FtInt) -> FtF26Dot6;

/// `TT_Move_Func`: Point displacement along the freedom vector routine
pub type TtMoveFunc =
    fn(exc: &mut TtExecContextRec, zone: ZoneRef, point: FtUShort, distance: FtF26Dot6);

/// `TT_Project_Func`: Distance projection along one of the projection
/// vectors
pub type TtProjectFunc = fn(exc: &TtExecContextRec, dx: FtPos, dy: FtPos) -> FtF26Dot6;

/// `TT_Cur_Ppem_Func`: getting current ppem.  Take care of non-square
/// pixels if necessary
pub type TtCurPpemFunc = fn(exc: &mut TtExecContextRec) -> FtLong;

/// `TT_Get_CVT_Func`: reading a cvt value.  Take care of non-square
/// pixels if necessary
pub type TtGetCvtFunc = fn(exc: &mut TtExecContextRec, idx: FtULong) -> FtF26Dot6;

/// `TT_Set_CVT_Func`: setting or moving a cvt value.  Take care of
/// non-square pixels if necessary
pub type TtSetCvtFunc = fn(exc: &mut TtExecContextRec, idx: FtULong, value: FtF26Dot6);

/// A definition record in a call (C's `TT_DefRecord*`, either an FDEF or
/// an IDEF).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefRef {
    /// `exc->FDefs + n`
    F(usize),
    /// `exc->IDefs + n`
    I(usize),
}

/// `TT_CallRec`: This structure defines a call record, used to manage
/// function calls.
#[derive(Debug, Clone, Copy)]
pub struct TtCallRec {
    pub caller_range: FtInt,
    pub caller_ip: FtLong,
    pub cur_count: FtLong,

    pub def: DefRef, /* either FDEF or IDEF */
}

impl Default for TtCallRec {
    fn default() -> Self {
        TtCallRec {
            caller_range: 0,
            caller_ip: 0,
            cur_count: 0,
            def: DefRef::F(0),
        }
    }
}

/// A zone pointer (`zp0`, `zp1`, `zp2`): which of the context's zones it
/// refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneRef {
    /// `exc->twilight`
    Twilight,
    /// `exc->pts`
    Pts,
}

/// `TT_ExecContextRec`: The main structure for the interpreter which
/// collects all necessary variables and states.
///
/// Members that are initialized by `TT_Load_Context` are marked with '!'.
/// Members that are initialized by `TT_Run_Context` are marked with '@'.
pub struct TtExecContextRec {
    /* (from the face: `exc->face`) */
    /// the driver's interpreter version (`TT_Driver->interpreter_version`)
    pub interpreter_version: FtUInt, /* ! */
    /// `face->root.num_glyphs`
    pub num_glyphs: FtLong, /* ! */
    /// `face->blend->num_axis` if the face has a blend
    pub blend_num_axis: Option<FtUInt>, /* ! */
    /// `face->blend->normalizedcoords`
    pub blend_coords: Option<Vec<FtFixed>>, /* ! */

    /* instructions state */
    pub error: FtError, /* last execution error */

    pub top: FtLong, /* @ top of exec. stack */

    pub stack_size: FtLong, /* ! size of exec. stack */
    pub stack: Vec<FtLong>, /* ! current exec. stack */

    pub args: FtLong,
    pub new_top: FtLong, /* new top after exec. */

    pub zp0: ZoneRef,             /* @! zone records */
    pub zp1: ZoneRef,             /* @!              */
    pub zp2: ZoneRef,             /* @!              */
    pub pts: TtGlyphZoneRec,      /*  !              */
    pub twilight: TtGlyphZoneRec, /*  !              */
    /// the size's twilight point count, restored by [`tt_unload_context`]
    pub twilight_n_points: FtUShort,

    pub point_size: FtLong,        /* ! in 26.6 format */
    pub metrics: FtSizeMetrics,    /* !                */
    pub tt_metrics: TtSizeMetrics, /* ! size metrics   */

    pub gs: TtGraphicsState, /* !@ current graphics state */

    pub ini_range: FtInt,  /* initial code range number   */
    pub cur_range: FtInt,  /* current code range number   */
    pub code: Arc<[u8]>,   /* current code range          */
    pub ip: FtLong,        /* current instruction pointer */
    pub code_size: FtLong, /* size of current range       */

    pub opcode: FtByte, /* current opcode              */
    pub length: FtInt,  /* length of current opcode    */

    pub step_ins: bool, /* true if the interpreter must */
    /* increment IP after ins. exec */
    pub cvt_size: FtULong, /* ! */
    /// `cvt` (the size's control value table; C switches the `cvt` pointer
    /// to `glyfCvt` in glyph programs, see `cvt_is_glyf`)
    pub cvt: Vec<FtLong>, /* ! */
    pub glyf_cvt_size: FtULong,
    pub glyf_cvt: Vec<FtLong>, /* cvt working copy for glyph */
    /// whether C's `exc->cvt` points to `glyfCvt`
    pub cvt_is_glyf: bool,

    pub glyph_size: FtUInt, /* ! glyph instructions buffer size */
    pub glyph_ins: Vec<u8>, /* ! glyph instructions buffer      */

    pub num_fdefs: FtUInt,       /* ! number of function defs         */
    pub max_fdefs: FtUInt,       /* ! maximum number of function defs */
    pub fdefs: Vec<TtDefRecord>, /*   table of FDefs entries          */

    pub num_idefs: FtUInt,       /* ! number of instruction defs */
    pub max_idefs: FtUInt,       /* ! maximum number of ins defs */
    pub idefs: Vec<TtDefRecord>, /*   table of IDefs entries     */

    pub max_func: FtUInt, /* ! maximum function index    */
    pub max_ins: FtUInt,  /* ! maximum instruction index */

    pub call_top: FtInt,            /* @ top of call stack during execution */
    pub call_size: FtInt,           /*   size of call stack                 */
    pub call_stack: Vec<TtCallRec>, /*   call stack                         */

    pub max_points: FtUShort,  /* capacity of this context's `pts' */
    pub max_contours: FtShort, /* record, expressed in points and  */
    /* contours.                        */
    pub code_range_table: TtCodeRangeTable, /* ! table of valid code ranges */
    /*   useful for the debugger    */
    pub store_size: FtUShort, /* ! size of current storage */
    /// `storage` (the size's storage area; see `storage_is_glyf`)
    pub storage: Vec<FtLong>, /* ! storage area            */
    pub glyf_store_size: FtUShort,
    pub glyf_storage: Vec<FtLong>, /* storage working copy for glyph */
    /// whether C's `exc->storage` points to `glyfStorage`
    pub storage_is_glyf: bool,

    pub period: FtF26Dot6, /* values used for the */
    pub phase: FtF26Dot6,  /* `SuperRounding'     */
    pub threshold: FtF26Dot6,

    pub instruction_trap: bool, /* ! If `True', the interpreter   */
    /*   exits after each instruction */
    pub default_gs: TtGraphicsState, /* graphics state resulting from   */
    /* the prep program                */
    pub is_composite: bool,     /* true if the glyph is composite  */
    pub pedantic_hinting: bool, /* true if pedantic interpretation */

    /* latest interpreter additions */
    pub f_dot_p: FtLong, /* dot product of freedom and projection */
    /* vectors                               */
    pub func_round: TtRoundFunc, /* current rounding function             */

    pub func_project: TtProjectFunc, /* current projection function */
    pub func_dualproj: TtProjectFunc, /* current dual proj. function */
    pub func_free_proj: TtProjectFunc, /* current freedom proj. func  */

    pub func_move: TtMoveFunc,      /* current point move function     */
    pub func_move_orig: TtMoveFunc, /* move original position function */

    pub func_cur_ppem: TtCurPpemFunc, /* get current proj. ppem value  */

    pub func_read_cvt: TtGetCvtFunc, /* read a cvt entry              */
    pub func_write_cvt: TtSetCvtFunc, /* write a cvt entry (in pixels) */
    pub func_move_cvt: TtSetCvtFunc, /* incr a cvt entry (in pixels)  */

    pub grayscale: bool, /* bi-level hinting and */
    /* grayscale rendering  */

    /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL (see the long comment in       */
    /* ttinterp.h about FreeType's ClearType-like v40 interpreter hacks)  */

    /* Using v40 implies subpixel hinting, unless FT_RENDER_MODE_MONO has been
     * requested.  Used to detect interpreter */
    /* version switches.  `_lean' to differentiate from the Infinality */
    /* `subpixel_hinting', which is managed differently.               */
    pub subpixel_hinting_lean: bool,

    /* Long side of a LCD subpixel is vertical (e.g., screen is rotated). */
    /* `_lean' to differentiate from the Infinality `vertical_lcd', which */
    /* is managed differently.                                            */
    pub vertical_lcd_lean: bool,

    /* Default to backward compatibility mode in v40 interpreter.  If   */
    /* this is false, it implies the interpreter is in v35 or in native */
    /* ClearType mode.                                                  */
    pub backward_compatibility: bool,

    /* Useful for detecting and denying post-IUP trickery that is usually */
    /* used to fix pixel patterns (`superhinting').                       */
    pub iupx_called: bool,
    pub iupy_called: bool,

    /* ClearType hinting and grayscale rendering, as used by Universal */
    /* Windows Platform apps (Windows 8 and above).  Like the standard */
    /* colorful ClearType mode, it utilizes a vastly increased virtual */
    /* resolution on the x axis.  Different from bi-level hinting and  */
    /* grayscale rendering, the old mode from Win9x days that roughly  */
    /* adheres to the physical pixel grid on both axes.                */
    pub grayscale_cleartype: bool,

    /* We maintain two counters (in addition to the instruction counter) */
    /* that act as loop detectors for LOOPCALL and jump opcodes with     */
    /* negative arguments.                                               */
    pub loopcall_counter: FtULong,
    pub loopcall_counter_max: FtULong,
    pub neg_jump_counter: FtULong,
    pub neg_jump_counter_max: FtULong,
}

impl std::fmt::Debug for TtExecContextRec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TtExecContextRec")
            .field("ip", &self.ip)
            .field("top", &self.top)
            .finish()
    }
}

impl TtExecContextRec {
    /// The zone a zone pointer refers to.
    #[inline]
    pub fn zone(&self, z: ZoneRef) -> &TtGlyphZoneRec {
        match z {
            ZoneRef::Twilight => &self.twilight,
            ZoneRef::Pts => &self.pts,
        }
    }

    /// The zone a zone pointer refers to.
    #[inline]
    pub fn zone_mut(&mut self, z: ZoneRef) -> &mut TtGlyphZoneRec {
        match z {
            ZoneRef::Twilight => &mut self.twilight,
            ZoneRef::Pts => &mut self.pts,
        }
    }

    /// `exc->cvt[idx]`
    #[inline]
    fn cvt_get(&self, idx: usize) -> FtLong {
        if self.cvt_is_glyf {
            self.glyf_cvt[idx]
        } else {
            self.cvt[idx]
        }
    }

    /// `exc->cvt[idx] = value`
    #[inline]
    fn cvt_set(&mut self, idx: usize, value: FtLong) {
        if self.cvt_is_glyf {
            self.glyf_cvt[idx] = value;
        } else {
            self.cvt[idx] = value;
        }
    }

    /// `exc->storage[idx]`
    #[inline]
    fn storage_get(&self, idx: usize) -> FtLong {
        if self.storage_is_glyf {
            self.glyf_storage[idx]
        } else {
            self.storage[idx]
        }
    }

    /// `exc->stack[exc->args + i]` (C's `args[i]`)
    #[inline]
    fn arg(&self, i: usize) -> FtLong {
        self.stack[self.args as usize + i]
    }

    /// `args[i] = value`
    #[inline]
    fn set_arg(&mut self, i: usize, value: FtLong) {
        let a = self.args as usize;
        self.stack[a + i] = value;
    }

    /// `exc->code[i]`
    #[inline]
    fn code_at(&self, i: FtLong) -> FtByte {
        self.code[i as usize]
    }

    /// The definition record `def` refers to.
    #[inline]
    fn def(&self, def: DefRef) -> &TtDefRecord {
        match def {
            DefRef::F(n) => &self.fdefs[n],
            DefRef::I(n) => &self.idefs[n],
        }
    }
}

/// `NO_SUBPIXEL_HINTING`
#[inline]
fn no_subpixel_hinting(exc: &TtExecContextRec) -> bool {
    exc.interpreter_version == TT_INTERPRETER_VERSION_35
}

/// `SUBPIXEL_HINTING_MINIMAL`
#[inline]
fn subpixel_hinting_minimal(exc: &TtExecContextRec) -> bool {
    exc.interpreter_version == TT_INTERPRETER_VERSION_40
}

/// `PROJECT`
#[inline]
fn project(exc: &TtExecContextRec, v1: FtVector, v2: FtVector) -> FtF26Dot6 {
    (exc.func_project)(exc, sub_long(v1.x, v2.x), sub_long(v1.y, v2.y))
}

/// `DUALPROJ`
#[inline]
fn dualproj(exc: &TtExecContextRec, v1: FtVector, v2: FtVector) -> FtF26Dot6 {
    (exc.func_dualproj)(exc, sub_long(v1.x, v2.x), sub_long(v1.y, v2.y))
}

/// `FAST_PROJECT`
#[inline]
fn fast_project(exc: &TtExecContextRec, v: FtVector) -> FtF26Dot6 {
    (exc.func_project)(exc, v.x, v.y)
}

/// `FAST_DUALPROJ`
#[inline]
fn fast_dualproj(exc: &TtExecContextRec, v: FtVector) -> FtF26Dot6 {
    (exc.func_dualproj)(exc, v.x, v.y)
}

/*
 *
 * Two simple bounds-checking macros.
 */
/// `BOUNDS`
#[inline]
fn bounds(x: FtLong, n: FtLong) -> bool {
    (x as FtUInt) >= (n as FtUInt)
}

/// `BOUNDSL`
#[inline]
fn boundsl(x: FtLong, n: FtLong) -> bool {
    (x as FtULong) >= (n as FtULong)
}

const SUCCESS: bool = false;
const FAILURE: bool = true;

/*
 *
 *                       CODERANGE FUNCTIONS
 *
 */

/// `TT_Goto_CodeRange`: Switches to a new code range (updates the code
/// related elements in `exec', and `IP').
pub fn tt_goto_code_range(exec: &mut TtExecContextRec, range: FtInt, ip: FtLong) {
    let coderange = &exec.code_range_table[(range - 1) as usize];

    /* NOTE: Because the last instruction of a program may be a CALL */
    /*       which will return to the first byte *after* the code    */
    /*       range, we test for IP <= Size instead of IP < Size.     */
    /*                                                               */
    exec.code = coderange
        .base
        .clone()
        .unwrap_or_else(|| Arc::from(Vec::new()));
    exec.code_size = coderange.size;
    exec.ip = ip;
    exec.cur_range = range;
}

/// `TT_Set_CodeRange`: Sets a code range.
pub fn tt_set_code_range(
    exec: &mut TtExecContextRec,
    range: FtInt,
    base: Arc<[u8]>,
    length: FtLong,
) {
    exec.code_range_table[(range - 1) as usize].base = Some(base);
    exec.code_range_table[(range - 1) as usize].size = length;
}

/// `TT_Clear_CodeRange`: Clears a code range.
pub fn tt_clear_code_range(exec: &mut TtExecContextRec, range: FtInt) {
    exec.code_range_table[(range - 1) as usize].base = None;
    exec.code_range_table[(range - 1) as usize].size = 0;
}

/*
 *
 *                  EXECUTION CONTEXT ROUTINES
 *
 */

/// `TT_Done_Context`: Destroys a given context.
pub fn tt_done_context(exec: Box<TtExecContextRec>) {
    drop(exec);
}

/// `TT_Load_Context`: Prepare an execution context for glyph hinting.
///
/// The size's definition tables, CVT, storage, and twilight zone move
/// into the context until [`tt_unload_context`] is called.
pub fn tt_load_context(
    exec: &mut TtExecContextRec,
    face: &mut TtFaceRec,
    interpreter_version: FtUInt,
) -> FtResult<()> {
    exec.interpreter_version = interpreter_version;
    exec.num_glyphs = face.root.num_glyphs;
    exec.blend_num_axis = face.blend.as_ref().map(|b| b.num_axis);
    exec.blend_coords = face.blend.as_ref().and_then(|b| b.normalizedcoords.clone());

    let maxp = face.max_profile;
    let metrics = *face.size_metrics();
    let size = &mut face.size;

    {
        exec.num_fdefs = size.num_function_defs;
        exec.max_fdefs = size.max_function_defs;
        exec.num_idefs = size.num_instruction_defs;
        exec.max_idefs = size.max_instruction_defs;
        exec.fdefs = std::mem::take(&mut size.function_defs);
        exec.idefs = std::mem::take(&mut size.instruction_defs);
        exec.point_size = size.point_size;
        exec.tt_metrics = size.ttmetrics;
        exec.metrics = metrics;

        exec.max_func = size.max_func;
        exec.max_ins = size.max_ins;

        exec.code_range_table = size.code_range_table.clone();

        /* set graphics state */
        exec.gs = size.gs;

        exec.cvt_size = size.cvt_size;
        exec.cvt = std::mem::take(&mut size.cvt);
        exec.cvt_is_glyf = false;

        exec.store_size = size.storage_size;
        exec.storage = std::mem::take(&mut size.storage);
        exec.storage_is_glyf = false;

        exec.twilight = std::mem::take(&mut size.twilight);
        exec.twilight_n_points = exec.twilight.n_points;

        /* In case of multi-threading it can happen that the old size object */
        /* no longer exists, thus we must clear all glyph zone references.   */
    }

    /* XXX: We reserve a little more elements on the stack to deal safely */
    /*      with broken fonts like arialbs, courbs, timesbs, etc.         */
    let new_size = maxp.maxStackElements as FtLong + 32;
    if (exec.stack.len() as FtLong) < new_size {
        if exec
            .stack
            .try_reserve_exact(new_size as usize - exec.stack.len())
            .is_err()
        {
            return Err(FT_ERR_OUT_OF_MEMORY);
        }
    }
    exec.stack.resize(new_size as usize, 0);
    exec.stack_size = new_size;

    /* free previous glyph code range */
    exec.glyph_ins = Vec::new();
    exec.glyph_size = 0;

    exec.pts.n_points = 0;
    exec.pts.n_contours = 0;

    exec.zp1 = ZoneRef::Pts;
    exec.zp2 = ZoneRef::Pts;
    exec.zp0 = ZoneRef::Pts;

    exec.instruction_trap = false;

    Ok(())
}

/// Moves the size's arrays back from the context (the counterpart of
/// [`tt_load_context`]; C shares them by pointer).
pub fn tt_unload_context(exec: &mut TtExecContextRec, size: &mut TtSizeRec) {
    size.function_defs = std::mem::take(&mut exec.fdefs);
    size.instruction_defs = std::mem::take(&mut exec.idefs);
    size.cvt = std::mem::take(&mut exec.cvt);
    exec.cvt_is_glyf = false;
    size.storage = std::mem::take(&mut exec.storage);
    exec.storage_is_glyf = false;
    let mut twilight = std::mem::take(&mut exec.twilight);
    twilight.n_points = exec.twilight_n_points;
    size.twilight = twilight;
}

/// `TT_Save_Context`: Saves the code ranges in a `size' object.
pub fn tt_save_context(exec: &TtExecContextRec, size: &mut TtSizeRec) {
    /* XXX: Will probably disappear soon with all the code range */
    /*      management, which is now rather obsolete.            */
    /*                                                           */
    size.num_function_defs = exec.num_fdefs;
    size.num_instruction_defs = exec.num_idefs;

    size.max_func = exec.max_func;
    size.max_ins = exec.max_ins;

    size.code_range_table = exec.code_range_table.clone();
}

/// `TT_Run_Context`: Executes one or more instructions in the execution
/// context.
pub fn tt_run_context(exec: &mut TtExecContextRec) -> FtResult<()> {
    tt_goto_code_range(exec, TT_CODERANGE_GLYPH, 0);

    exec.zp0 = ZoneRef::Pts;
    exec.zp1 = ZoneRef::Pts;
    exec.zp2 = ZoneRef::Pts;

    exec.gs.gep0 = 1;
    exec.gs.gep1 = 1;
    exec.gs.gep2 = 1;

    exec.gs.proj_vector.x = 0x4000;
    exec.gs.proj_vector.y = 0x0000;

    exec.gs.free_vector = exec.gs.proj_vector;
    exec.gs.dual_vector = exec.gs.proj_vector;

    exec.gs.round_state = 1;
    exec.gs.loop_ = 1;

    /* some glyphs leave something on the stack. so we clean it */
    /* before a new execution.                                  */
    exec.top = 0;
    exec.call_top = 0;

    tt_run_ins(exec)
}

/* The default value for `scan_control' is documented as FALSE in the */
/* TrueType specification.  This is confusing since it implies a      */
/* Boolean value.  However, this is not the case, thus both the       */
/* default values of our `scan_type' and `scan_control' fields (which */
/* the documentation's `scan_control' variable is split into) are     */
/* zero.                                                              */
/// `tt_default_graphics_state`
pub const TT_DEFAULT_GRAPHICS_STATE: TtGraphicsState = TtGraphicsState {
    rp0: 0,
    rp1: 0,
    rp2: 0,
    dual_vector: FtUnitVector { x: 0x4000, y: 0 },
    proj_vector: FtUnitVector { x: 0x4000, y: 0 },
    free_vector: FtUnitVector { x: 0x4000, y: 0 },
    loop_: 1,
    minimum_distance: 64,
    round_state: 1,
    auto_flip: true,
    control_value_cutin: 68,
    single_width_cutin: 0,
    single_width_value: 0,
    delta_base: 9,
    delta_shift: 3,
    instruct_control: 0,
    scan_control: false,
    scan_type: 0,
    gep0: 1,
    gep1: 1,
    gep2: 1,
};

/// `TT_New_Context` (documentation is in ttinterp.h)
pub fn tt_new_context() -> Option<Box<TtExecContextRec>> {
    /* allocate object and zero everything inside */
    let mut call_stack = Vec::new();

    /* create callStack here, other allocations delayed */
    if call_stack.try_reserve_exact(32).is_err() {
        return None;
    }
    call_stack.resize(32, TtCallRec::default());

    Some(Box::new(TtExecContextRec {
        interpreter_version: TT_INTERPRETER_VERSION_40,
        num_glyphs: 0,
        blend_num_axis: None,
        blend_coords: None,
        error: 0,
        top: 0,
        stack_size: 0,
        stack: Vec::new(),
        args: 0,
        new_top: 0,
        zp0: ZoneRef::Pts,
        zp1: ZoneRef::Pts,
        zp2: ZoneRef::Pts,
        pts: TtGlyphZoneRec::default(),
        twilight: TtGlyphZoneRec::default(),
        twilight_n_points: 0,
        point_size: 0,
        metrics: FtSizeMetrics::default(),
        tt_metrics: TtSizeMetrics::default(),
        gs: TtGraphicsState::default(),
        ini_range: 0,
        cur_range: 0,
        code: Arc::from(Vec::new()),
        ip: 0,
        code_size: 0,
        opcode: 0,
        length: 0,
        step_ins: false,
        cvt_size: 0,
        cvt: Vec::new(),
        glyf_cvt_size: 0,
        glyf_cvt: Vec::new(),
        cvt_is_glyf: false,
        glyph_size: 0,
        glyph_ins: Vec::new(),
        num_fdefs: 0,
        max_fdefs: 0,
        fdefs: Vec::new(),
        num_idefs: 0,
        max_idefs: 0,
        idefs: Vec::new(),
        max_func: 0,
        max_ins: 0,
        call_top: 0,
        call_size: 32,
        call_stack,
        max_points: 0,
        max_contours: 0,
        code_range_table: TtCodeRangeTable::default(),
        store_size: 0,
        storage: Vec::new(),
        glyf_store_size: 0,
        glyf_storage: Vec::new(),
        storage_is_glyf: false,
        period: 0,
        phase: 0,
        threshold: 0,
        instruction_trap: false,
        default_gs: TtGraphicsState::default(),
        is_composite: false,
        pedantic_hinting: false,
        f_dot_p: 0,
        func_round: round_none,
        func_project: project_x,
        func_dualproj: project_x,
        func_free_proj: project_x,
        func_move: direct_move,
        func_move_orig: direct_move_orig,
        func_cur_ppem: current_ppem,
        func_read_cvt: read_cvt,
        func_write_cvt: write_cvt,
        func_move_cvt: move_cvt,
        grayscale: false,
        subpixel_hinting_lean: false,
        vertical_lcd_lean: false,
        backward_compatibility: false,
        iupx_called: false,
        iupy_called: false,
        grayscale_cleartype: false,
        loopcall_counter: 0,
        loopcall_counter_max: 0,
        neg_jump_counter: 0,
        neg_jump_counter_max: 0,
    }))
}

/*
 *
 * Before an opcode is executed, the interpreter verifies that there are
 * enough arguments on the stack, with the help of the `Pop_Push_Count'
 * table.
 *
 * For each opcode, the first column gives the number of arguments that
 * are popped from the stack; the second one gives the number of those
 * that are pushed in result.
 *
 * Opcodes which have a varying number of parameters in the data stream
 * (NPUSHB, NPUSHW) are handled specially; they have a negative value in
 * the `opcode_length' table, and the value in `Pop_Push_Count' is set
 * to zero.
 *
 */

/// `PACK`
const fn pack(x: u8, y: u8) -> u8 {
    (x << 4) | y
}

#[rustfmt::skip]
static POP_PUSH_COUNT: [u8; 256] = [
    /* opcodes are gathered in groups of 16 */
    /* please keep the spaces as they are   */

    /* 0x00 */
    /*  SVTCA[0]  */  pack( 0, 0 ),
    /*  SVTCA[1]  */  pack( 0, 0 ),
    /*  SPVTCA[0] */  pack( 0, 0 ),
    /*  SPVTCA[1] */  pack( 0, 0 ),
    /*  SFVTCA[0] */  pack( 0, 0 ),
    /*  SFVTCA[1] */  pack( 0, 0 ),
    /*  SPVTL[0]  */  pack( 2, 0 ),
    /*  SPVTL[1]  */  pack( 2, 0 ),
    /*  SFVTL[0]  */  pack( 2, 0 ),
    /*  SFVTL[1]  */  pack( 2, 0 ),
    /*  SPVFS     */  pack( 2, 0 ),
    /*  SFVFS     */  pack( 2, 0 ),
    /*  GPV       */  pack( 0, 2 ),
    /*  GFV       */  pack( 0, 2 ),
    /*  SFVTPV    */  pack( 0, 0 ),
    /*  ISECT     */  pack( 5, 0 ),

    /* 0x10 */
    /*  SRP0      */  pack( 1, 0 ),
    /*  SRP1      */  pack( 1, 0 ),
    /*  SRP2      */  pack( 1, 0 ),
    /*  SZP0      */  pack( 1, 0 ),
    /*  SZP1      */  pack( 1, 0 ),
    /*  SZP2      */  pack( 1, 0 ),
    /*  SZPS      */  pack( 1, 0 ),
    /*  SLOOP     */  pack( 1, 0 ),
    /*  RTG       */  pack( 0, 0 ),
    /*  RTHG      */  pack( 0, 0 ),
    /*  SMD       */  pack( 1, 0 ),
    /*  ELSE      */  pack( 0, 0 ),
    /*  JMPR      */  pack( 1, 0 ),
    /*  SCVTCI    */  pack( 1, 0 ),
    /*  SSWCI     */  pack( 1, 0 ),
    /*  SSW       */  pack( 1, 0 ),

    /* 0x20 */
    /*  DUP       */  pack( 1, 2 ),
    /*  POP       */  pack( 1, 0 ),
    /*  CLEAR     */  pack( 0, 0 ),
    /*  SWAP      */  pack( 2, 2 ),
    /*  DEPTH     */  pack( 0, 1 ),
    /*  CINDEX    */  pack( 1, 1 ),
    /*  MINDEX    */  pack( 1, 0 ),
    /*  ALIGNPTS  */  pack( 2, 0 ),
    /*  INS_$28   */  pack( 0, 0 ),
    /*  UTP       */  pack( 1, 0 ),
    /*  LOOPCALL  */  pack( 2, 0 ),
    /*  CALL      */  pack( 1, 0 ),
    /*  FDEF      */  pack( 1, 0 ),
    /*  ENDF      */  pack( 0, 0 ),
    /*  MDAP[0]   */  pack( 1, 0 ),
    /*  MDAP[1]   */  pack( 1, 0 ),

    /* 0x30 */
    /*  IUP[0]    */  pack( 0, 0 ),
    /*  IUP[1]    */  pack( 0, 0 ),
    /*  SHP[0]    */  pack( 0, 0 ), /* loops */
    /*  SHP[1]    */  pack( 0, 0 ), /* loops */
    /*  SHC[0]    */  pack( 1, 0 ),
    /*  SHC[1]    */  pack( 1, 0 ),
    /*  SHZ[0]    */  pack( 1, 0 ),
    /*  SHZ[1]    */  pack( 1, 0 ),
    /*  SHPIX     */  pack( 1, 0 ), /* loops */
    /*  IP        */  pack( 0, 0 ), /* loops */
    /*  MSIRP[0]  */  pack( 2, 0 ),
    /*  MSIRP[1]  */  pack( 2, 0 ),
    /*  ALIGNRP   */  pack( 0, 0 ), /* loops */
    /*  RTDG      */  pack( 0, 0 ),
    /*  MIAP[0]   */  pack( 2, 0 ),
    /*  MIAP[1]   */  pack( 2, 0 ),

    /* 0x40 */
    /*  NPUSHB    */  pack( 0, 0 ),
    /*  NPUSHW    */  pack( 0, 0 ),
    /*  WS        */  pack( 2, 0 ),
    /*  RS        */  pack( 1, 1 ),
    /*  WCVTP     */  pack( 2, 0 ),
    /*  RCVT      */  pack( 1, 1 ),
    /*  GC[0]     */  pack( 1, 1 ),
    /*  GC[1]     */  pack( 1, 1 ),
    /*  SCFS      */  pack( 2, 0 ),
    /*  MD[0]     */  pack( 2, 1 ),
    /*  MD[1]     */  pack( 2, 1 ),
    /*  MPPEM     */  pack( 0, 1 ),
    /*  MPS       */  pack( 0, 1 ),
    /*  FLIPON    */  pack( 0, 0 ),
    /*  FLIPOFF   */  pack( 0, 0 ),
    /*  DEBUG     */  pack( 1, 0 ),

    /* 0x50 */
    /*  LT        */  pack( 2, 1 ),
    /*  LTEQ      */  pack( 2, 1 ),
    /*  GT        */  pack( 2, 1 ),
    /*  GTEQ      */  pack( 2, 1 ),
    /*  EQ        */  pack( 2, 1 ),
    /*  NEQ       */  pack( 2, 1 ),
    /*  ODD       */  pack( 1, 1 ),
    /*  EVEN      */  pack( 1, 1 ),
    /*  IF        */  pack( 1, 0 ),
    /*  EIF       */  pack( 0, 0 ),
    /*  AND       */  pack( 2, 1 ),
    /*  OR        */  pack( 2, 1 ),
    /*  NOT       */  pack( 1, 1 ),
    /*  DELTAP1   */  pack( 1, 0 ),
    /*  SDB       */  pack( 1, 0 ),
    /*  SDS       */  pack( 1, 0 ),

    /* 0x60 */
    /*  ADD       */  pack( 2, 1 ),
    /*  SUB       */  pack( 2, 1 ),
    /*  DIV       */  pack( 2, 1 ),
    /*  MUL       */  pack( 2, 1 ),
    /*  ABS       */  pack( 1, 1 ),
    /*  NEG       */  pack( 1, 1 ),
    /*  FLOOR     */  pack( 1, 1 ),
    /*  CEILING   */  pack( 1, 1 ),
    /*  ROUND[0]  */  pack( 1, 1 ),
    /*  ROUND[1]  */  pack( 1, 1 ),
    /*  ROUND[2]  */  pack( 1, 1 ),
    /*  ROUND[3]  */  pack( 1, 1 ),
    /*  NROUND[0] */  pack( 1, 1 ),
    /*  NROUND[1] */  pack( 1, 1 ),
    /*  NROUND[2] */  pack( 1, 1 ),
    /*  NROUND[3] */  pack( 1, 1 ),

    /* 0x70 */
    /*  WCVTF     */  pack( 2, 0 ),
    /*  DELTAP2   */  pack( 1, 0 ),
    /*  DELTAP3   */  pack( 1, 0 ),
    /*  DELTAC1   */  pack( 1, 0 ),
    /*  DELTAC2   */  pack( 1, 0 ),
    /*  DELTAC3   */  pack( 1, 0 ),
    /*  SROUND    */  pack( 1, 0 ),
    /*  S45ROUND  */  pack( 1, 0 ),
    /*  JROT      */  pack( 2, 0 ),
    /*  JROF      */  pack( 2, 0 ),
    /*  ROFF      */  pack( 0, 0 ),
    /*  INS_$7B   */  pack( 0, 0 ),
    /*  RUTG      */  pack( 0, 0 ),
    /*  RDTG      */  pack( 0, 0 ),
    /*  SANGW     */  pack( 1, 0 ),
    /*  AA        */  pack( 1, 0 ),

    /* 0x80 */
    /*  FLIPPT    */  pack( 0, 0 ), /* loops */
    /*  FLIPRGON  */  pack( 2, 0 ),
    /*  FLIPRGOFF */  pack( 2, 0 ),
    /*  INS_$83   */  pack( 0, 0 ),
    /*  INS_$84   */  pack( 0, 0 ),
    /*  SCANCTRL  */  pack( 1, 0 ),
    /*  SDPVTL[0] */  pack( 2, 0 ),
    /*  SDPVTL[1] */  pack( 2, 0 ),
    /*  GETINFO   */  pack( 1, 1 ),
    /*  IDEF      */  pack( 1, 0 ),
    /*  ROLL      */  pack( 3, 3 ),
    /*  MAX       */  pack( 2, 1 ),
    /*  MIN       */  pack( 2, 1 ),
    /*  SCANTYPE  */  pack( 1, 0 ),
    /*  INSTCTRL  */  pack( 2, 0 ),
    /*  INS_$8F   */  pack( 0, 0 ),

    /* 0x90 */
    /*  INS_$90  */   pack( 0, 0 ),
    /*  GETVAR   */   pack( 0, 0 ), /* will be handled specially */
    /*  GETDATA  */   pack( 0, 1 ),
    /*  INS_$93  */   pack( 0, 0 ),
    /*  INS_$94  */   pack( 0, 0 ),
    /*  INS_$95  */   pack( 0, 0 ),
    /*  INS_$96  */   pack( 0, 0 ),
    /*  INS_$97  */   pack( 0, 0 ),
    /*  INS_$98  */   pack( 0, 0 ),
    /*  INS_$99  */   pack( 0, 0 ),
    /*  INS_$9A  */   pack( 0, 0 ),
    /*  INS_$9B  */   pack( 0, 0 ),
    /*  INS_$9C  */   pack( 0, 0 ),
    /*  INS_$9D  */   pack( 0, 0 ),
    /*  INS_$9E  */   pack( 0, 0 ),
    /*  INS_$9F  */   pack( 0, 0 ),

    /* 0xA0 */
    /*  INS_$A0  */   pack( 0, 0 ),
    /*  INS_$A1  */   pack( 0, 0 ),
    /*  INS_$A2  */   pack( 0, 0 ),
    /*  INS_$A3  */   pack( 0, 0 ),
    /*  INS_$A4  */   pack( 0, 0 ),
    /*  INS_$A5  */   pack( 0, 0 ),
    /*  INS_$A6  */   pack( 0, 0 ),
    /*  INS_$A7  */   pack( 0, 0 ),
    /*  INS_$A8  */   pack( 0, 0 ),
    /*  INS_$A9  */   pack( 0, 0 ),
    /*  INS_$AA  */   pack( 0, 0 ),
    /*  INS_$AB  */   pack( 0, 0 ),
    /*  INS_$AC  */   pack( 0, 0 ),
    /*  INS_$AD  */   pack( 0, 0 ),
    /*  INS_$AE  */   pack( 0, 0 ),
    /*  INS_$AF  */   pack( 0, 0 ),

    /* 0xB0 */
    /*  PUSHB[0]  */  pack( 0, 1 ),
    /*  PUSHB[1]  */  pack( 0, 2 ),
    /*  PUSHB[2]  */  pack( 0, 3 ),
    /*  PUSHB[3]  */  pack( 0, 4 ),
    /*  PUSHB[4]  */  pack( 0, 5 ),
    /*  PUSHB[5]  */  pack( 0, 6 ),
    /*  PUSHB[6]  */  pack( 0, 7 ),
    /*  PUSHB[7]  */  pack( 0, 8 ),
    /*  PUSHW[0]  */  pack( 0, 1 ),
    /*  PUSHW[1]  */  pack( 0, 2 ),
    /*  PUSHW[2]  */  pack( 0, 3 ),
    /*  PUSHW[3]  */  pack( 0, 4 ),
    /*  PUSHW[4]  */  pack( 0, 5 ),
    /*  PUSHW[5]  */  pack( 0, 6 ),
    /*  PUSHW[6]  */  pack( 0, 7 ),
    /*  PUSHW[7]  */  pack( 0, 8 ),

    /* 0xC0 */
    /*  MDRP[00]  */  pack( 1, 0 ),
    /*  MDRP[01]  */  pack( 1, 0 ),
    /*  MDRP[02]  */  pack( 1, 0 ),
    /*  MDRP[03]  */  pack( 1, 0 ),
    /*  MDRP[04]  */  pack( 1, 0 ),
    /*  MDRP[05]  */  pack( 1, 0 ),
    /*  MDRP[06]  */  pack( 1, 0 ),
    /*  MDRP[07]  */  pack( 1, 0 ),
    /*  MDRP[08]  */  pack( 1, 0 ),
    /*  MDRP[09]  */  pack( 1, 0 ),
    /*  MDRP[10]  */  pack( 1, 0 ),
    /*  MDRP[11]  */  pack( 1, 0 ),
    /*  MDRP[12]  */  pack( 1, 0 ),
    /*  MDRP[13]  */  pack( 1, 0 ),
    /*  MDRP[14]  */  pack( 1, 0 ),
    /*  MDRP[15]  */  pack( 1, 0 ),

    /* 0xD0 */
    /*  MDRP[16]  */  pack( 1, 0 ),
    /*  MDRP[17]  */  pack( 1, 0 ),
    /*  MDRP[18]  */  pack( 1, 0 ),
    /*  MDRP[19]  */  pack( 1, 0 ),
    /*  MDRP[20]  */  pack( 1, 0 ),
    /*  MDRP[21]  */  pack( 1, 0 ),
    /*  MDRP[22]  */  pack( 1, 0 ),
    /*  MDRP[23]  */  pack( 1, 0 ),
    /*  MDRP[24]  */  pack( 1, 0 ),
    /*  MDRP[25]  */  pack( 1, 0 ),
    /*  MDRP[26]  */  pack( 1, 0 ),
    /*  MDRP[27]  */  pack( 1, 0 ),
    /*  MDRP[28]  */  pack( 1, 0 ),
    /*  MDRP[29]  */  pack( 1, 0 ),
    /*  MDRP[30]  */  pack( 1, 0 ),
    /*  MDRP[31]  */  pack( 1, 0 ),

    /* 0xE0 */
    /*  MIRP[00]  */  pack( 2, 0 ),
    /*  MIRP[01]  */  pack( 2, 0 ),
    /*  MIRP[02]  */  pack( 2, 0 ),
    /*  MIRP[03]  */  pack( 2, 0 ),
    /*  MIRP[04]  */  pack( 2, 0 ),
    /*  MIRP[05]  */  pack( 2, 0 ),
    /*  MIRP[06]  */  pack( 2, 0 ),
    /*  MIRP[07]  */  pack( 2, 0 ),
    /*  MIRP[08]  */  pack( 2, 0 ),
    /*  MIRP[09]  */  pack( 2, 0 ),
    /*  MIRP[10]  */  pack( 2, 0 ),
    /*  MIRP[11]  */  pack( 2, 0 ),
    /*  MIRP[12]  */  pack( 2, 0 ),
    /*  MIRP[13]  */  pack( 2, 0 ),
    /*  MIRP[14]  */  pack( 2, 0 ),
    /*  MIRP[15]  */  pack( 2, 0 ),

    /* 0xF0 */
    /*  MIRP[16]  */  pack( 2, 0 ),
    /*  MIRP[17]  */  pack( 2, 0 ),
    /*  MIRP[18]  */  pack( 2, 0 ),
    /*  MIRP[19]  */  pack( 2, 0 ),
    /*  MIRP[20]  */  pack( 2, 0 ),
    /*  MIRP[21]  */  pack( 2, 0 ),
    /*  MIRP[22]  */  pack( 2, 0 ),
    /*  MIRP[23]  */  pack( 2, 0 ),
    /*  MIRP[24]  */  pack( 2, 0 ),
    /*  MIRP[25]  */  pack( 2, 0 ),
    /*  MIRP[26]  */  pack( 2, 0 ),
    /*  MIRP[27]  */  pack( 2, 0 ),
    /*  MIRP[28]  */  pack( 2, 0 ),
    /*  MIRP[29]  */  pack( 2, 0 ),
    /*  MIRP[30]  */  pack( 2, 0 ),
    /*  MIRP[31]  */  pack( 2, 0 ),
];

/* (`opcode_name' is only used for tracing) */

#[rustfmt::skip]
static OPCODE_LENGTH: [i8; 256] = [
    1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,
    1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,
    1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,
    1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,

   -1,-2, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,
    1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,
    1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,
    1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,

    1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,
    1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,
    1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,
    2, 3, 4, 5,  6, 7, 8, 9,  3, 5, 7, 9, 11,13,15,17,

    1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,
    1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,
    1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,
    1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,  1, 1, 1, 1,
];

/// `TT_MulFix14` (`TT_MulFix14_long_long`)
#[inline]
fn tt_mul_fix14(a: FtInt32, b: FtInt) -> FtInt32 {
    let mut ret = a as i64 * b as i64;

    /* The following line assumes that right shifting of signed values */
    /* will actually preserve the sign bit.  The exact behaviour is    */
    /* undefined, but this is true on x86 and x86_64.                  */
    let tmp = ret >> 63;

    ret += 0x2000 + tmp;

    (ret >> 14) as FtInt32
}

/// `TT_DotFix14` (`TT_DotFix14_long_long`)
#[inline]
fn tt_dot_fix14(ax: FtInt32, ay: FtInt32, bx: FtInt, by: FtInt) -> FtInt32 {
    /* Temporarily disable the warning that C90 doesn't support */
    /* `long long'.                                             */
    let mut temp1 = ax as i64 * bx as i64;
    let mut temp2 = ay as i64 * by as i64;

    temp1 = temp1.wrapping_add(temp2);
    temp2 = temp1 >> 63;
    temp1 = temp1.wrapping_add(0x2000 + temp2);

    (temp1 >> 14) as FtInt32
}

/// `Current_Ratio`: Returns the current aspect ratio scaling factor
/// depending on the projection vector's state and device resolutions.
fn current_ratio(exc: &mut TtExecContextRec) -> FtLong {
    if exc.tt_metrics.ratio == 0 {
        if exc.gs.proj_vector.y == 0 {
            exc.tt_metrics.ratio = exc.tt_metrics.x_ratio;
        } else if exc.gs.proj_vector.x == 0 {
            exc.tt_metrics.ratio = exc.tt_metrics.y_ratio;
        } else {
            let x = tt_mul_fix14(
                exc.tt_metrics.x_ratio as FtInt32,
                exc.gs.proj_vector.x as FtInt,
            ) as FtF26Dot6;
            let y = tt_mul_fix14(
                exc.tt_metrics.y_ratio as FtInt32,
                exc.gs.proj_vector.y as FtInt,
            ) as FtF26Dot6;
            exc.tt_metrics.ratio = ft_hypot_fixed(x, y);
        }
    }
    exc.tt_metrics.ratio
}

/// `Current_Ppem`
fn current_ppem(exc: &mut TtExecContextRec) -> FtLong {
    exc.tt_metrics.ppem as FtLong
}

/// `Current_Ppem_Stretched`
fn current_ppem_stretched(exc: &mut TtExecContextRec) -> FtLong {
    let r = current_ratio(exc);
    ft_mul_fix(exc.tt_metrics.ppem as FtLong, r)
}

/*
 *
 * Functions related to the control value table (CVT).
 *
 */

/// `Read_CVT`
fn read_cvt(exc: &mut TtExecContextRec, idx: FtULong) -> FtF26Dot6 {
    exc.cvt_get(idx as usize)
}

/// `Read_CVT_Stretched`
fn read_cvt_stretched(exc: &mut TtExecContextRec, idx: FtULong) -> FtF26Dot6 {
    let r = current_ratio(exc);
    ft_mul_fix(exc.cvt_get(idx as usize), r)
}

/// `Modify_CVT_Check`
fn modify_cvt_check(exc: &mut TtExecContextRec) {
    if exc.ini_range == TT_CODERANGE_GLYPH && !exc.cvt_is_glyf {
        let n = exc.cvt_size as usize;
        if exc.glyf_cvt.len() < n
            && exc
                .glyf_cvt
                .try_reserve_exact(n - exc.glyf_cvt.len())
                .is_err()
        {
            exc.error = FT_ERR_OUT_OF_MEMORY;
            return;
        }
        exc.error = 0;
        exc.glyf_cvt.resize(n, 0);

        exc.glyf_cvt_size = exc.cvt_size;
        exc.glyf_cvt[..n].copy_from_slice(&exc.cvt[..n]);
        exc.cvt_is_glyf = true;
    }
}

/// `Write_CVT`
fn write_cvt(exc: &mut TtExecContextRec, idx: FtULong, value: FtF26Dot6) {
    modify_cvt_check(exc);
    if exc.error != 0 {
        return;
    }

    exc.cvt_set(idx as usize, value);
}

/// `Write_CVT_Stretched`
fn write_cvt_stretched(exc: &mut TtExecContextRec, idx: FtULong, value: FtF26Dot6) {
    modify_cvt_check(exc);
    if exc.error != 0 {
        return;
    }

    let r = current_ratio(exc);
    exc.cvt_set(idx as usize, ft_div_fix(value, r));
}

/// `Move_CVT`
fn move_cvt(exc: &mut TtExecContextRec, idx: FtULong, value: FtF26Dot6) {
    modify_cvt_check(exc);
    if exc.error != 0 {
        return;
    }

    let v = add_long(exc.cvt_get(idx as usize), value);
    exc.cvt_set(idx as usize, v);
}

/// `Move_CVT_Stretched`
fn move_cvt_stretched(exc: &mut TtExecContextRec, idx: FtULong, value: FtF26Dot6) {
    modify_cvt_check(exc);
    if exc.error != 0 {
        return;
    }

    let r = current_ratio(exc);
    let v = add_long(exc.cvt_get(idx as usize), ft_div_fix(value, r));
    exc.cvt_set(idx as usize, v);
}

/// `GetShortIns`: Returns a short integer taken from the instruction
/// stream at address IP.
fn get_short_ins(exc: &mut TtExecContextRec) -> FtShort {
    /* Reading a byte stream so there is no endianness (DaveP) */
    exc.ip += 2;
    (((exc.code_at(exc.ip - 2) as i32) << 8) + exc.code_at(exc.ip - 1) as i32) as FtShort
}

/// `Ins_Goto_CodeRange`: Goes to a certain code range in the instruction
/// stream.
fn ins_goto_code_range(exc: &mut TtExecContextRec, a_range: FtInt, a_ip: FtLong) -> bool {
    if !(1..=3).contains(&a_range) {
        exc.error = FT_ERR_BAD_ARGUMENT;
        return FAILURE;
    }

    let range = &exc.code_range_table[(a_range - 1) as usize];

    let base = match &range.base {
        Some(b) => b.clone(),
        None => {
            /* invalid coderange */
            exc.error = FT_ERR_INVALID_CODERANGE;
            return FAILURE;
        }
    };

    /* NOTE: Because the last instruction of a program may be a CALL */
    /*       which will return to the first byte *after* the code    */
    /*       range, we test for aIP <= Size, instead of aIP < Size.  */

    if a_ip > range.size {
        exc.error = FT_ERR_CODE_OVERFLOW;
        return FAILURE;
    }

    exc.code_size = range.size;
    exc.code = base;
    exc.ip = a_ip;
    exc.cur_range = a_range;

    SUCCESS
}

/*
 *
 * Apple's TrueType specification at
 *
 *   https://developer.apple.com/fonts/TrueType-Reference-Manual/RM02/Chap2.html#order
 *
 * gives the following order of operations in instructions that move
 * points.
 *
 *   - check single width cut-in (MIRP, MDRP)
 *
 *   - check control value cut-in (MIRP, MIAP)
 *
 *   - apply engine compensation (MIRP, MDRP)
 *
 *   - round distance (MIRP, MDRP) or value (MIAP, MDAP)
 *
 *   - check minimum distance (MIRP,MDRP)
 *
 *   - move point (MIRP, MDRP, MIAP, MSIRP, MDAP)
 *
 * For rounding instructions, engine compensation happens before rounding.
 *
 */

/// `Direct_Move`: Moves a point by a given distance along the freedom
/// vector.  The point will be `touched'.
fn direct_move(exc: &mut TtExecContextRec, zone: ZoneRef, point: FtUShort, distance: FtF26Dot6) {
    let p = point as usize;
    let f_dot_p = exc.f_dot_p;

    let v = exc.gs.free_vector.x as FtF26Dot6;

    if v != 0 {
        /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
        /* Exception to the post-IUP curfew: Allow the x component of */
        /* diagonal moves, but only post-IUP.  DejaVu tries to adjust */
        /* diagonal stems like on `Z' and `z' post-IUP.               */
        if (subpixel_hinting_minimal(exc) && !exc.backward_compatibility)
            || no_subpixel_hinting(exc)
        {
            let z = exc.zone_mut(zone);
            z.cur[p].x = add_long(z.cur[p].x, ft_mul_div(distance, v, f_dot_p));
        }

        exc.zone_mut(zone).tags[p] |= FT_CURVE_TAG_TOUCH_X;
    }

    let v = exc.gs.free_vector.y as FtF26Dot6;

    if v != 0 {
        /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
        if !(subpixel_hinting_minimal(exc)
            && exc.backward_compatibility
            && exc.iupx_called
            && exc.iupy_called)
        {
            let z = exc.zone_mut(zone);
            z.cur[p].y = add_long(z.cur[p].y, ft_mul_div(distance, v, f_dot_p));
        }

        exc.zone_mut(zone).tags[p] |= FT_CURVE_TAG_TOUCH_Y;
    }
}

/// `Direct_Move_Orig`: Moves the *original* position of a point by a given
/// distance along the freedom vector.  Obviously, the point will not be
/// `touched'.
fn direct_move_orig(
    exc: &mut TtExecContextRec,
    zone: ZoneRef,
    point: FtUShort,
    distance: FtF26Dot6,
) {
    let p = point as usize;
    let f_dot_p = exc.f_dot_p;

    let v = exc.gs.free_vector.x as FtF26Dot6;

    if v != 0 {
        let z = exc.zone_mut(zone);
        z.org[p].x = add_long(z.org[p].x, ft_mul_div(distance, v, f_dot_p));
    }

    let v = exc.gs.free_vector.y as FtF26Dot6;

    if v != 0 {
        let z = exc.zone_mut(zone);
        z.org[p].y = add_long(z.org[p].y, ft_mul_div(distance, v, f_dot_p));
    }
}

/*
 *
 * Special versions of Direct_Move()
 *
 *   The following versions are used whenever both vectors are both
 *   along one of the coordinate unit vectors, i.e. in 90% of the cases.
 *   See `ttinterp.h' for details on backward compatibility mode.
 *
 */

/// `Direct_Move_X`
fn direct_move_x(exc: &mut TtExecContextRec, zone: ZoneRef, point: FtUShort, distance: FtF26Dot6) {
    let p = point as usize;

    /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
    if (subpixel_hinting_minimal(exc) && !exc.backward_compatibility) || no_subpixel_hinting(exc) {
        let z = exc.zone_mut(zone);
        z.cur[p].x = add_long(z.cur[p].x, distance);
    }

    exc.zone_mut(zone).tags[p] |= FT_CURVE_TAG_TOUCH_X;
}

/// `Direct_Move_Y`
fn direct_move_y(exc: &mut TtExecContextRec, zone: ZoneRef, point: FtUShort, distance: FtF26Dot6) {
    let p = point as usize;

    /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
    if !(subpixel_hinting_minimal(exc)
        && exc.backward_compatibility
        && exc.iupx_called
        && exc.iupy_called)
    {
        let z = exc.zone_mut(zone);
        z.cur[p].y = add_long(z.cur[p].y, distance);
    }

    exc.zone_mut(zone).tags[p] |= FT_CURVE_TAG_TOUCH_Y;
}

/*
 *
 * Special versions of Direct_Move_Orig()
 *
 *   The following versions are used whenever both vectors are both
 *   along one of the coordinate unit vectors, i.e. in 90% of the cases.
 *
 */

/// `Direct_Move_Orig_X`
fn direct_move_orig_x(
    exc: &mut TtExecContextRec,
    zone: ZoneRef,
    point: FtUShort,
    distance: FtF26Dot6,
) {
    let z = exc.zone_mut(zone);
    let p = point as usize;
    z.org[p].x = add_long(z.org[p].x, distance);
}

/// `Direct_Move_Orig_Y`
fn direct_move_orig_y(
    exc: &mut TtExecContextRec,
    zone: ZoneRef,
    point: FtUShort,
    distance: FtF26Dot6,
) {
    let z = exc.zone_mut(zone);
    let p = point as usize;
    z.org[p].y = add_long(z.org[p].y, distance);
}

/// `Round_None`: Does not round, but adds engine compensation.
fn round_none(exc: &TtExecContextRec, distance: FtF26Dot6, color: FtInt) -> FtF26Dot6 {
    let compensation = exc.tt_metrics.compensations[color as usize];
    let mut val;

    if distance >= 0 {
        val = add_long(distance, compensation);
        if val < 0 {
            val = 0;
        }
    } else {
        val = sub_long(distance, compensation);
        if val > 0 {
            val = 0;
        }
    }
    val
}

/// `Round_To_Grid`: Rounds value to grid after adding engine compensation.
fn round_to_grid(exc: &TtExecContextRec, distance: FtF26Dot6, color: FtInt) -> FtF26Dot6 {
    let compensation = exc.tt_metrics.compensations[color as usize];
    let mut val;

    if distance >= 0 {
        val = ft_pix_round_long(add_long(distance, compensation));
        if val < 0 {
            val = 0;
        }
    } else {
        val = neg_long(ft_pix_round_long(sub_long(compensation, distance)));
        if val > 0 {
            val = 0;
        }
    }

    val
}

/// `Round_To_Half_Grid`: Rounds value to half grid after adding engine
/// compensation.
fn round_to_half_grid(exc: &TtExecContextRec, distance: FtF26Dot6, color: FtInt) -> FtF26Dot6 {
    let compensation = exc.tt_metrics.compensations[color as usize];
    let mut val;

    if distance >= 0 {
        val = add_long(ft_pix_floor(add_long(distance, compensation)), 32);
        if val < 0 {
            val = 32;
        }
    } else {
        val = neg_long(add_long(ft_pix_floor(sub_long(compensation, distance)), 32));
        if val > 0 {
            val = -32;
        }
    }

    val
}

/// `Round_Down_To_Grid`: Rounds value down to grid after adding engine
/// compensation.
fn round_down_to_grid(exc: &TtExecContextRec, distance: FtF26Dot6, color: FtInt) -> FtF26Dot6 {
    let compensation = exc.tt_metrics.compensations[color as usize];
    let mut val;

    if distance >= 0 {
        val = ft_pix_floor(add_long(distance, compensation));
        if val < 0 {
            val = 0;
        }
    } else {
        val = neg_long(ft_pix_floor(sub_long(compensation, distance)));
        if val > 0 {
            val = 0;
        }
    }

    val
}

/// `Round_Up_To_Grid`: Rounds value up to grid after adding engine
/// compensation.
fn round_up_to_grid(exc: &TtExecContextRec, distance: FtF26Dot6, color: FtInt) -> FtF26Dot6 {
    let compensation = exc.tt_metrics.compensations[color as usize];
    let mut val;

    if distance >= 0 {
        val = ft_pix_ceil_long(add_long(distance, compensation));
        if val < 0 {
            val = 0;
        }
    } else {
        val = neg_long(ft_pix_ceil_long(sub_long(compensation, distance)));
        if val > 0 {
            val = 0;
        }
    }

    val
}

/// `Round_To_Double_Grid`: Rounds value to double grid after adding engine
/// compensation.
fn round_to_double_grid(exc: &TtExecContextRec, distance: FtF26Dot6, color: FtInt) -> FtF26Dot6 {
    let compensation = exc.tt_metrics.compensations[color as usize];
    let mut val;

    if distance >= 0 {
        val = ft_pad_round(add_long(distance, compensation), 32);
        if val < 0 {
            val = 0;
        }
    } else {
        val = neg_long(ft_pad_round(sub_long(compensation, distance), 32));
        if val > 0 {
            val = 0;
        }
    }

    val
}

/// `Round_Super`: Super-rounds value to grid after adding engine
/// compensation.
///
/// The TrueType specification says very little about the relationship
/// between rounding and engine compensation.  However, it seems from the
/// description of super round that we should add the compensation before
/// rounding.
fn round_super(exc: &TtExecContextRec, distance: FtF26Dot6, color: FtInt) -> FtF26Dot6 {
    let compensation = exc.tt_metrics.compensations[color as usize];
    let mut val;

    if distance >= 0 {
        val = add_long(
            distance,
            exc.threshold
                .wrapping_sub(exc.phase)
                .wrapping_add(compensation),
        ) & exc.period.wrapping_neg();
        val = add_long(val, exc.phase);
        if val < 0 {
            val = exc.phase;
        }
    } else {
        val = neg_long(
            sub_long(
                exc.threshold
                    .wrapping_sub(exc.phase)
                    .wrapping_add(compensation),
                distance,
            ) & exc.period.wrapping_neg(),
        );
        val = sub_long(val, exc.phase);
        if val > 0 {
            val = exc.phase.wrapping_neg();
        }
    }

    val
}

/// `Round_Super_45`: Super-rounds value to grid after adding engine
/// compensation.
///
/// There is a separate function for Round_Super_45() as we may need
/// greater precision.
fn round_super_45(exc: &TtExecContextRec, distance: FtF26Dot6, color: FtInt) -> FtF26Dot6 {
    let compensation = exc.tt_metrics.compensations[color as usize];
    let mut val;

    if distance >= 0 {
        val = (add_long(
            distance,
            exc.threshold
                .wrapping_sub(exc.phase)
                .wrapping_add(compensation),
        )
        .wrapping_div(exc.period))
        .wrapping_mul(exc.period);
        val = add_long(val, exc.phase);
        if val < 0 {
            val = exc.phase;
        }
    } else {
        val = neg_long(
            (sub_long(
                exc.threshold
                    .wrapping_sub(exc.phase)
                    .wrapping_add(compensation),
                distance,
            )
            .wrapping_div(exc.period))
            .wrapping_mul(exc.period),
        );
        val = sub_long(val, exc.phase);
        if val > 0 {
            val = exc.phase.wrapping_neg();
        }
    }

    val
}

/// `Compute_Round`: Sets the rounding mode.
fn compute_round(exc: &mut TtExecContextRec, round_mode: FtByte) {
    match round_mode as FtInt {
        TT_ROUND_OFF => exc.func_round = round_none,
        TT_ROUND_TO_GRID => exc.func_round = round_to_grid,
        TT_ROUND_UP_TO_GRID => exc.func_round = round_up_to_grid,
        TT_ROUND_DOWN_TO_GRID => exc.func_round = round_down_to_grid,
        TT_ROUND_TO_HALF_GRID => exc.func_round = round_to_half_grid,
        TT_ROUND_TO_DOUBLE_GRID => exc.func_round = round_to_double_grid,
        TT_ROUND_SUPER => exc.func_round = round_super,
        TT_ROUND_SUPER_45 => exc.func_round = round_super_45,
        _ => {}
    }
}

/// `SetSuperRound`: Sets Super Round parameters.
fn set_super_round(exc: &mut TtExecContextRec, grid_period: FtF2Dot14, selector: FtLong) {
    let grid_period = grid_period as FtLong;
    match (selector & 0xC0) as FtInt {
        0 => exc.period = grid_period / 2,
        0x40 => exc.period = grid_period,
        0x80 => exc.period = grid_period * 2,
        /* This opcode is reserved, but... */
        0xC0 => exc.period = grid_period,
        _ => {}
    }

    match (selector & 0x30) as FtInt {
        0 => exc.phase = 0,
        0x10 => exc.phase = exc.period / 4,
        0x20 => exc.phase = exc.period / 2,
        0x30 => exc.phase = exc.period * 3 / 4,
        _ => {}
    }

    if (selector & 0x0F) == 0 {
        exc.threshold = exc.period - 1;
    } else {
        exc.threshold = (((selector & 0x0F) as FtInt - 4) as FtLong) * exc.period / 8;
    }

    /* convert to F26Dot6 format */
    exc.period >>= 8;
    exc.phase >>= 8;
    exc.threshold >>= 8;
}

/// `Project`: Computes the projection of vector given by (v2-v1) along the
/// current projection vector.
fn project_fn(exc: &TtExecContextRec, dx: FtPos, dy: FtPos) -> FtF26Dot6 {
    tt_dot_fix14(
        dx as FtInt32,
        dy as FtInt32,
        exc.gs.proj_vector.x as FtInt,
        exc.gs.proj_vector.y as FtInt,
    ) as FtF26Dot6
}

/// `Dual_Project`: Computes the projection of the vector given by (v2-v1)
/// along the current dual vector.
fn dual_project(exc: &TtExecContextRec, dx: FtPos, dy: FtPos) -> FtF26Dot6 {
    tt_dot_fix14(
        dx as FtInt32,
        dy as FtInt32,
        exc.gs.dual_vector.x as FtInt,
        exc.gs.dual_vector.y as FtInt,
    ) as FtF26Dot6
}

/// `Project_x`: Computes the projection of the vector given by (v2-v1)
/// along the horizontal axis.
fn project_x(_exc: &TtExecContextRec, dx: FtPos, _dy: FtPos) -> FtF26Dot6 {
    dx
}

/// `Project_y`: Computes the projection of the vector given by (v2-v1)
/// along the vertical axis.
fn project_y(_exc: &TtExecContextRec, _dx: FtPos, dy: FtPos) -> FtF26Dot6 {
    dy
}

/// `Compute_Funcs`: Computes the projection and movement function pointers
/// according to the current graphics state.
fn compute_funcs(exc: &mut TtExecContextRec) {
    if exc.gs.free_vector.x == 0x4000 {
        exc.f_dot_p = exc.gs.proj_vector.x as FtLong;
    } else if exc.gs.free_vector.y == 0x4000 {
        exc.f_dot_p = exc.gs.proj_vector.y as FtLong;
    } else {
        exc.f_dot_p = (exc.gs.proj_vector.x as FtLong * exc.gs.free_vector.x as FtLong
            + exc.gs.proj_vector.y as FtLong * exc.gs.free_vector.y as FtLong)
            >> 14;
    }

    if exc.gs.proj_vector.x == 0x4000 {
        exc.func_project = project_x;
    } else if exc.gs.proj_vector.y == 0x4000 {
        exc.func_project = project_y;
    } else {
        exc.func_project = project_fn;
    }

    if exc.gs.dual_vector.x == 0x4000 {
        exc.func_dualproj = project_x;
    } else if exc.gs.dual_vector.y == 0x4000 {
        exc.func_dualproj = project_y;
    } else {
        exc.func_dualproj = dual_project;
    }

    exc.func_move = direct_move;
    exc.func_move_orig = direct_move_orig;

    if exc.f_dot_p == 0x4000 {
        if exc.gs.free_vector.x == 0x4000 {
            exc.func_move = direct_move_x;
            exc.func_move_orig = direct_move_orig_x;
        } else if exc.gs.free_vector.y == 0x4000 {
            exc.func_move = direct_move_y;
            exc.func_move_orig = direct_move_orig_y;
        }
    }

    /* at small sizes, F_dot_P can become too small, resulting   */
    /* in overflows and `spikes' in a number of glyphs like `w'. */

    if ft_abs(exc.f_dot_p) < 0x400 {
        exc.f_dot_p = 0x4000;
    }

    /* Disable cached aspect ratio */
    exc.tt_metrics.ratio = 0;
}

/// `Normalize`: Norms a vector.
///
/// In case Vx and Vy are both zero, `Normalize' returns SUCCESS, and R is
/// undefined.
fn normalize(vx: FtF26Dot6, vy: FtF26Dot6, r: &mut FtUnitVector) -> bool {
    if vx == 0 && vy == 0 {
        /* XXX: UNDOCUMENTED! It seems that it is possible to try   */
        /*      to normalize the vector (0,0).  Return immediately. */
        return SUCCESS;
    }

    let mut v = FtVector { x: vx, y: vy };

    ft_vector_norm_len(&mut v);

    r.x = (v.x / 4) as FtF2Dot14;
    r.y = (v.y / 4) as FtF2Dot14;

    SUCCESS
}

/*
 *
 * Here we start with the implementation of the various opcodes.
 *
 */

/// `ARRAY_BOUND_ERROR`
macro_rules! array_bound_error {
    ($exc:expr) => {{
        $exc.error = FT_ERR_INVALID_REFERENCE;
        return;
    }};
}

/// `MPPEM[]`: Measure Pixel Per EM
fn ins_mppem(exc: &mut TtExecContextRec) {
    let v = (exc.func_cur_ppem)(exc);
    exc.set_arg(0, v);
}

/// `MPS[]`: Measure Point Size
fn ins_mps(exc: &mut TtExecContextRec) {
    if no_subpixel_hinting(exc) {
        /* Microsoft's GDI bytecode interpreter always returns value 12; */
        /* we return the current PPEM value instead.                     */
        let v = (exc.func_cur_ppem)(exc);
        exc.set_arg(0, v);
    } else {
        /* A possible practical application of the MPS instruction is to   */
        /* implement optical scaling and similar features, which should be */
        /* based on perceptual attributes, thus independent of the         */
        /* resolution.                                                     */
        let v = exc.point_size;
        exc.set_arg(0, v);
    }
}

/// `DUP[]`: DUPlicate the stack's top element
fn ins_dup(exc: &mut TtExecContextRec) {
    let a = exc.arg(0);
    exc.set_arg(1, a);
}

/// `POP[]`: POP the stack's top element
fn ins_pop() {
    /* nothing to do */
}

/// `CLEAR[]`: CLEAR the entire stack
fn ins_clear(exc: &mut TtExecContextRec) {
    exc.new_top = 0;
}

/// `SWAP[]`: SWAP the stack's top two elements
fn ins_swap(exc: &mut TtExecContextRec) {
    let l = exc.arg(0);
    let b = exc.arg(1);
    exc.set_arg(0, b);
    exc.set_arg(1, l);
}

/// `DEPTH[]`: return the stack DEPTH
fn ins_depth(exc: &mut TtExecContextRec) {
    let t = exc.top;
    exc.set_arg(0, t);
}

/// `LT[]`: Less Than
fn ins_lt(exc: &mut TtExecContextRec) {
    let r = (exc.arg(0) < exc.arg(1)) as FtLong;
    exc.set_arg(0, r);
}

/// `LTEQ[]`: Less Than or EQual
fn ins_lteq(exc: &mut TtExecContextRec) {
    let r = (exc.arg(0) <= exc.arg(1)) as FtLong;
    exc.set_arg(0, r);
}

/// `GT[]`: Greater Than
fn ins_gt(exc: &mut TtExecContextRec) {
    let r = (exc.arg(0) > exc.arg(1)) as FtLong;
    exc.set_arg(0, r);
}

/// `GTEQ[]`: Greater Than or EQual
fn ins_gteq(exc: &mut TtExecContextRec) {
    let r = (exc.arg(0) >= exc.arg(1)) as FtLong;
    exc.set_arg(0, r);
}

/// `EQ[]`: EQual
fn ins_eq(exc: &mut TtExecContextRec) {
    let r = (exc.arg(0) == exc.arg(1)) as FtLong;
    exc.set_arg(0, r);
}

/// `NEQ[]`: Not EQual
fn ins_neq(exc: &mut TtExecContextRec) {
    let r = (exc.arg(0) != exc.arg(1)) as FtLong;
    exc.set_arg(0, r);
}

/// `ODD[]`: Is ODD
fn ins_odd(exc: &mut TtExecContextRec) {
    let r = (((exc.func_round)(exc, exc.arg(0), 3) & 127) == 64) as FtLong;
    exc.set_arg(0, r);
}

/// `EVEN[]`: Is EVEN
fn ins_even(exc: &mut TtExecContextRec) {
    let r = (((exc.func_round)(exc, exc.arg(0), 3) & 127) == 0) as FtLong;
    exc.set_arg(0, r);
}

/// `AND[]`: logical AND
fn ins_and(exc: &mut TtExecContextRec) {
    let r = (exc.arg(0) != 0 && exc.arg(1) != 0) as FtLong;
    exc.set_arg(0, r);
}

/// `OR[]`: logical OR
fn ins_or(exc: &mut TtExecContextRec) {
    let r = (exc.arg(0) != 0 || exc.arg(1) != 0) as FtLong;
    exc.set_arg(0, r);
}

/// `NOT[]`: logical NOT
fn ins_not(exc: &mut TtExecContextRec) {
    let r = (exc.arg(0) == 0) as FtLong;
    exc.set_arg(0, r);
}

/// `ADD[]`: ADD
fn ins_add(exc: &mut TtExecContextRec) {
    let r = add_long(exc.arg(0), exc.arg(1));
    exc.set_arg(0, r);
}

/// `SUB[]`: SUBtract
fn ins_sub(exc: &mut TtExecContextRec) {
    let r = sub_long(exc.arg(0), exc.arg(1));
    exc.set_arg(0, r);
}

/// `DIV[]`: DIVide
fn ins_div(exc: &mut TtExecContextRec) {
    if exc.arg(1) == 0 {
        exc.error = FT_ERR_DIVIDE_BY_ZERO;
    } else {
        let r = ft_mul_div_no_round(exc.arg(0), 64, exc.arg(1));
        exc.set_arg(0, r);
    }
}

/// `MUL[]`: MULtiply
fn ins_mul(exc: &mut TtExecContextRec) {
    let r = ft_mul_div(exc.arg(0), exc.arg(1), 64);
    exc.set_arg(0, r);
}

/// `ABS[]`: ABSolute value
fn ins_abs(exc: &mut TtExecContextRec) {
    if exc.arg(0) < 0 {
        let r = neg_long(exc.arg(0));
        exc.set_arg(0, r);
    }
}

/// `NEG[]`: NEGate
fn ins_neg(exc: &mut TtExecContextRec) {
    let r = neg_long(exc.arg(0));
    exc.set_arg(0, r);
}

/// `FLOOR[]`: FLOOR
fn ins_floor(exc: &mut TtExecContextRec) {
    let r = ft_pix_floor(exc.arg(0));
    exc.set_arg(0, r);
}

/// `CEILING[]`: CEILING
fn ins_ceiling(exc: &mut TtExecContextRec) {
    let r = ft_pix_ceil_long(exc.arg(0));
    exc.set_arg(0, r);
}

/// `RS[]`: Read Store
fn ins_rs(exc: &mut TtExecContextRec) {
    let i = exc.arg(0) as FtULong;

    if boundsl(i as FtLong, exc.store_size as FtLong) {
        if exc.pedantic_hinting {
            array_bound_error!(exc);
        } else {
            exc.set_arg(0, 0);
        }
    } else {
        let v = exc.storage_get(i as usize);
        exc.set_arg(0, v);
    }
}

/// `WS[]`: Write Store
fn ins_ws(exc: &mut TtExecContextRec) {
    let i = exc.arg(0) as FtULong;

    if boundsl(i as FtLong, exc.store_size as FtLong) {
        if exc.pedantic_hinting {
            array_bound_error!(exc);
        }
    } else {
        if exc.ini_range == TT_CODERANGE_GLYPH && !exc.storage_is_glyf {
            let n = exc.store_size as usize;
            if exc.glyf_storage.len() < n
                && exc
                    .glyf_storage
                    .try_reserve_exact(n - exc.glyf_storage.len())
                    .is_err()
            {
                exc.error = FT_ERR_OUT_OF_MEMORY;
                return;
            }
            exc.error = 0;
            exc.glyf_storage.resize(n, 0);

            exc.glyf_store_size = exc.store_size;
            exc.glyf_storage[..n].copy_from_slice(&exc.storage[..n]);
            exc.storage_is_glyf = true;
        }

        let v = exc.arg(1);
        if exc.storage_is_glyf {
            exc.glyf_storage[i as usize] = v;
        } else {
            exc.storage[i as usize] = v;
        }
    }
}

/// `WCVTP[]`: Write CVT in Pixel units
fn ins_wcvtp(exc: &mut TtExecContextRec) {
    let i = exc.arg(0) as FtULong;

    if boundsl(i as FtLong, exc.cvt_size as FtLong) {
        if exc.pedantic_hinting {
            array_bound_error!(exc);
        }
    } else {
        let v = exc.arg(1);
        (exc.func_write_cvt)(exc, i, v);
    }
}

/// `WCVTF[]`: Write CVT in Funits
fn ins_wcvtf(exc: &mut TtExecContextRec) {
    let i = exc.arg(0) as FtULong;

    if boundsl(i as FtLong, exc.cvt_size as FtLong) {
        if exc.pedantic_hinting {
            array_bound_error!(exc);
        }
    } else {
        /* (C writes through the current `cvt' pointer directly) */
        let v = ft_mul_fix(exc.arg(1), exc.tt_metrics.scale);
        exc.cvt_set(i as usize, v);
    }
}

/// `RCVT[]`: Read CVT
fn ins_rcvt(exc: &mut TtExecContextRec) {
    let i = exc.arg(0) as FtULong;

    if boundsl(i as FtLong, exc.cvt_size as FtLong) {
        if exc.pedantic_hinting {
            array_bound_error!(exc);
        } else {
            exc.set_arg(0, 0);
        }
    } else {
        let v = (exc.func_read_cvt)(exc, i);
        exc.set_arg(0, v);
    }
}

/// `AA[]`: Adjust Angle
fn ins_aa() {
    /* intentionally no longer supported */
}

/// `DEBUG[]`: DEBUG.  Unsupported.
///
/// Note: The original instruction pops a value from the stack.
fn ins_debug(exc: &mut TtExecContextRec) {
    exc.error = FT_ERR_DEBUG_OPCODE;
}

/// `ROUND[ab]`: ROUND value
fn ins_round(exc: &mut TtExecContextRec) {
    let r = (exc.func_round)(exc, exc.arg(0), (exc.opcode & 3) as FtInt);
    exc.set_arg(0, r);
}

/// `NROUND[ab]`: No ROUNDing of value
fn ins_nround(exc: &mut TtExecContextRec) {
    let r = round_none(exc, exc.arg(0), (exc.opcode & 3) as FtInt);
    exc.set_arg(0, r);
}

/// `MAX[]`: MAXimum
fn ins_max(exc: &mut TtExecContextRec) {
    if exc.arg(1) > exc.arg(0) {
        let v = exc.arg(1);
        exc.set_arg(0, v);
    }
}

/// `MIN[]`: MINimum
fn ins_min(exc: &mut TtExecContextRec) {
    if exc.arg(1) < exc.arg(0) {
        let v = exc.arg(1);
        exc.set_arg(0, v);
    }
}

/// `MINDEX[]`: Move INDEXed element
fn ins_mindex(exc: &mut TtExecContextRec) {
    let l = exc.arg(0);

    if l <= 0 || l > exc.args {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
    } else {
        let a = exc.args as usize;
        let l = l as usize;
        let k = exc.stack[a - l];
        exc.stack.copy_within(a - l + 1..a, a - l);
        exc.stack[a - 1] = k;
    }
}

/// `CINDEX[]`: Copy INDEXed element
fn ins_cindex(exc: &mut TtExecContextRec) {
    let l = exc.arg(0);

    if l <= 0 || l > exc.args {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
        exc.set_arg(0, 0);
    } else {
        let v = exc.stack[(exc.args - l) as usize];
        exc.set_arg(0, v);
    }
}

/// `ROLL[]`: ROLL top three elements
fn ins_roll(exc: &mut TtExecContextRec) {
    let a = exc.arg(2);
    let b = exc.arg(1);
    let c = exc.arg(0);

    exc.set_arg(2, c);
    exc.set_arg(1, a);
    exc.set_arg(0, b);
}

/*
 *
 * MANAGING THE FLOW OF CONTROL
 *
 */

/// `SLOOP[]`: Set LOOP variable
fn ins_sloop(exc: &mut TtExecContextRec) {
    let a = exc.arg(0);
    if a < 0 {
        exc.error = FT_ERR_BAD_ARGUMENT;
    } else {
        /* we heuristically limit the number of loops to 16 bits */
        exc.gs.loop_ = if a > 0xFFFF { 0xFFFF } else { a };
    }
}

/// `SkipCode`
fn skip_code(exc: &mut TtExecContextRec) -> bool {
    exc.ip += exc.length as FtLong;

    if exc.ip < exc.code_size {
        exc.opcode = exc.code_at(exc.ip);

        exc.length = OPCODE_LENGTH[exc.opcode as usize] as FtInt;
        if exc.length < 0 {
            if exc.ip + 1 >= exc.code_size {
                exc.error = FT_ERR_CODE_OVERFLOW;
                return FAILURE;
            }
            exc.length = 2 - exc.length * exc.code_at(exc.ip + 1) as FtInt;
        }

        if exc.ip + exc.length as FtLong <= exc.code_size {
            return SUCCESS;
        }
    }

    /* Fail_Overflow: */
    exc.error = FT_ERR_CODE_OVERFLOW;
    FAILURE
}

/// `IF[]`: IF test
fn ins_if(exc: &mut TtExecContextRec) {
    if exc.arg(0) != 0 {
        return;
    }

    let mut n_ifs: FtInt = 1;
    let mut out = false;

    loop {
        if skip_code(exc) == FAILURE {
            return;
        }

        match exc.opcode {
            0x58 => {
                /* IF */
                n_ifs += 1;
            }
            0x1B => {
                /* ELSE */
                out = n_ifs == 1;
            }
            0x59 => {
                /* EIF */
                n_ifs -= 1;
                out = n_ifs == 0;
            }
            _ => {}
        }

        if out {
            break;
        }
    }
}

/// `ELSE[]`: ELSE
fn ins_else(exc: &mut TtExecContextRec) {
    let mut n_ifs: FtInt = 1;

    loop {
        if skip_code(exc) == FAILURE {
            return;
        }

        match exc.opcode {
            0x58 => {
                /* IF */
                n_ifs += 1;
            }
            0x59 => {
                /* EIF */
                n_ifs -= 1;
            }
            _ => {}
        }

        if n_ifs == 0 {
            break;
        }
    }
}

/// `EIF[]`: End IF
fn ins_eif() {
    /* nothing to do */
}

/// `JMPR[]`: JuMP Relative
fn ins_jmpr(exc: &mut TtExecContextRec) {
    let a0 = exc.arg(0);
    if a0 == 0 && exc.args == 0 {
        exc.error = FT_ERR_BAD_ARGUMENT;
        return;
    }

    exc.ip = add_long(exc.ip, a0);
    if exc.ip < 0
        || (exc.call_top > 0
            && exc.ip > exc.def(exc.call_stack[(exc.call_top - 1) as usize].def).end)
    {
        exc.error = FT_ERR_BAD_ARGUMENT;
        return;
    }

    exc.step_ins = false;

    if a0 < 0 {
        exc.neg_jump_counter += 1;
        if exc.neg_jump_counter > exc.neg_jump_counter_max {
            exc.error = FT_ERR_EXECUTION_TOO_LONG;
        }
    }
}

/// `JROT[]`: Jump Relative On True
fn ins_jrot(exc: &mut TtExecContextRec) {
    if exc.arg(1) != 0 {
        ins_jmpr(exc);
    }
}

/// `JROF[]`: Jump Relative On False
fn ins_jrof(exc: &mut TtExecContextRec) {
    if exc.arg(1) == 0 {
        ins_jmpr(exc);
    }
}

/*
 *
 * DEFINING AND USING FUNCTIONS AND INSTRUCTIONS
 *
 */

/// `FDEF[]`: Function DEFinition
fn ins_fdef(exc: &mut TtExecContextRec) {
    /* FDEF is only allowed in `prep' or `fpgm' */
    if exc.ini_range == TT_CODERANGE_GLYPH {
        exc.error = FT_ERR_DEF_IN_GLYF_BYTECODE;
        return;
    }

    /* some font programs are broken enough to redefine functions! */
    /* We will then parse the current table.                       */

    let limit = exc.num_fdefs as usize;
    let n = exc.arg(0) as FtULong;

    let mut rec = 0usize;
    while rec < limit {
        if exc.fdefs[rec].opc as FtULong == n {
            break;
        }
        rec += 1;
    }

    if rec == limit {
        /* check that there is enough room for new functions */
        if exc.num_fdefs >= exc.max_fdefs {
            exc.error = FT_ERR_TOO_MANY_FUNCTION_DEFS;
            return;
        }
        exc.num_fdefs += 1;
    }

    /* Although FDEF takes unsigned 32-bit integer,  */
    /* func # must be within unsigned 16-bit integer */
    if n > 0xFFFF {
        exc.error = FT_ERR_TOO_MANY_FUNCTION_DEFS;
        return;
    }

    {
        let cur_range = exc.cur_range;
        let start = exc.ip + 1;
        let r = &mut exc.fdefs[rec];
        r.range = cur_range;
        r.opc = n as FtUInt16 as FtUInt;
        r.start = start;
        r.active = true;
    }

    if n > exc.max_func as FtULong {
        exc.max_func = n as FtUInt16 as FtUInt;
    }

    /* Now skip the whole function definition. */
    /* We don't allow nested IDEFS & FDEFs.    */

    while skip_code(exc) == SUCCESS {
        match exc.opcode {
            0x89 | 0x2C => {
                /* IDEF */
                /* FDEF */
                exc.error = FT_ERR_NESTED_DEFS;
                return;
            }

            0x2D => {
                /* ENDF */
                exc.fdefs[rec].end = exc.ip;
                return;
            }

            _ => {}
        }
    }
}

/// `ENDF[]`: END Function definition
fn ins_endf(exc: &mut TtExecContextRec) {
    if exc.call_top <= 0 {
        /* We encountered an ENDF without a call */
        exc.error = FT_ERR_ENDF_IN_EXEC_STREAM;
        return;
    }

    exc.call_top -= 1;

    let p_rec = exc.call_top as usize;

    exc.call_stack[p_rec].cur_count -= 1;

    exc.step_ins = false;

    if exc.call_stack[p_rec].cur_count > 0 {
        exc.call_top += 1;
        exc.ip = exc.def(exc.call_stack[p_rec].def).start;
    } else {
        /* Loop through the current function */
        let (range, ip) = (
            exc.call_stack[p_rec].caller_range,
            exc.call_stack[p_rec].caller_ip,
        );
        let _ = ins_goto_code_range(exc, range, ip);
    }

    /* Exit the current call frame.                      */

    /* NOTE: If the last instruction of a program is a   */
    /*       CALL or LOOPCALL, the return address is     */
    /*       always out of the code range.  This is a    */
    /*       valid address, and it is why we do not test */
    /*       the result of Ins_Goto_CodeRange() here!    */
}

/// Looks up function `f` in the FDefs table (the shared part of `Ins_CALL`
/// and `Ins_LOOPCALL`); `None` means `goto Fail`.
fn find_fdef(exc: &TtExecContextRec, f: FtULong) -> Option<usize> {
    /* first of all, check the index */
    if boundsl(f as FtLong, exc.max_func as FtLong + 1) {
        return None;
    }

    if exc.fdefs.is_empty() {
        return None;
    }

    /* Except for some old Apple fonts, all functions in a TrueType */
    /* font are defined in increasing order, starting from 0.  This */
    /* means that we normally have                                  */
    /*                                                              */
    /*    exc->maxFunc+1 == exc->numFDefs                           */
    /*    exc->FDefs[n].opc == n for n in 0..exc->maxFunc           */
    /*                                                              */
    /* If this isn't true, we need to look up the function table.   */

    let mut def = f as usize;
    if exc.max_func + 1 != exc.num_fdefs || exc.fdefs[def].opc as FtULong != f {
        /* look up the FDefs table */
        let limit = exc.num_fdefs as usize;

        def = 0;
        while def < limit && exc.fdefs[def].opc as FtULong != f {
            def += 1;
        }

        if def == limit {
            return None;
        }
    }

    /* check that the function is active */
    if !exc.fdefs[def].active {
        return None;
    }

    Some(def)
}

/// `CALL[]`: CALL function
fn ins_call(exc: &mut TtExecContextRec) {
    let f = exc.arg(0) as FtULong;

    let def = match find_fdef(exc, f) {
        Some(d) => d,
        None => {
            /* Fail: */
            exc.error = FT_ERR_INVALID_REFERENCE;
            return;
        }
    };

    /* check the call stack */
    if exc.call_top >= exc.call_size {
        exc.error = FT_ERR_STACK_OVERFLOW;
        return;
    }

    let p_crec = exc.call_top as usize;

    exc.call_stack[p_crec] = TtCallRec {
        caller_range: exc.cur_range,
        caller_ip: exc.ip + 1,
        cur_count: 1,
        def: DefRef::F(def),
    };

    exc.call_top += 1;

    let (range, start) = (exc.fdefs[def].range, exc.fdefs[def].start);
    let _ = ins_goto_code_range(exc, range, start);

    exc.step_ins = false;
}

/// `LOOPCALL[]`: LOOP and CALL function
fn ins_loopcall(exc: &mut TtExecContextRec) {
    /* first of all, check the index */
    let f = exc.arg(1) as FtULong;

    let def = match find_fdef_unchecked_table(exc, f) {
        Some(d) => d,
        None => {
            /* Fail: */
            exc.error = FT_ERR_INVALID_REFERENCE;
            return;
        }
    };

    /* check stack */
    if exc.call_top >= exc.call_size {
        exc.error = FT_ERR_STACK_OVERFLOW;
        return;
    }

    let a0 = exc.arg(0);
    if a0 > 0 {
        let p_crec = exc.call_top as usize;

        exc.call_stack[p_crec] = TtCallRec {
            caller_range: exc.cur_range,
            caller_ip: exc.ip + 1,
            cur_count: a0 as FtInt as FtLong,
            def: DefRef::F(def),
        };

        exc.call_top += 1;

        let (range, start) = (exc.fdefs[def].range, exc.fdefs[def].start);
        let _ = ins_goto_code_range(exc, range, start);

        exc.step_ins = false;

        exc.loopcall_counter = exc.loopcall_counter.wrapping_add(a0 as FtULong);
        if exc.loopcall_counter > exc.loopcall_counter_max {
            exc.error = FT_ERR_EXECUTION_TOO_LONG;
        }
    }
}

/// The function lookup of `Ins_LOOPCALL`, which (unlike `Ins_CALL`) does
/// not test for a missing FDefs table.
fn find_fdef_unchecked_table(exc: &TtExecContextRec, f: FtULong) -> Option<usize> {
    if boundsl(f as FtLong, exc.max_func as FtLong + 1) {
        return None;
    }

    /* Except for some old Apple fonts, all functions in a TrueType */
    /* font are defined in increasing order, starting from 0.  This */
    /* means that we normally have                                  */
    /*                                                              */
    /*    exc->maxFunc+1 == exc->numFDefs                           */
    /*    exc->FDefs[n].opc == n for n in 0..exc->maxFunc           */
    /*                                                              */
    /* If this isn't true, we need to look up the function table.   */

    let mut def = f as usize;
    if exc.max_func + 1 != exc.num_fdefs || exc.fdefs[def].opc as FtULong != f {
        /* look up the FDefs table */
        let limit = exc.num_fdefs as usize;

        def = 0;
        while def < limit && exc.fdefs[def].opc as FtULong != f {
            def += 1;
        }

        if def == limit {
            return None;
        }
    }

    /* check that the function is active */
    if !exc.fdefs[def].active {
        return None;
    }

    Some(def)
}

/// `IDEF[]`: Instruction DEFinition
fn ins_idef(exc: &mut TtExecContextRec) {
    /* we enable IDEF only in `prep' or `fpgm' */
    if exc.ini_range == TT_CODERANGE_GLYPH {
        exc.error = FT_ERR_DEF_IN_GLYF_BYTECODE;
        return;
    }

    /*  First of all, look for the same function in our table */

    let limit = exc.num_idefs as usize;
    let a0 = exc.arg(0);

    let mut def = 0usize;
    while def < limit {
        if exc.idefs[def].opc as FtULong == a0 as FtULong {
            break;
        }
        def += 1;
    }

    if def == limit {
        /* check that there is enough room for a new instruction */
        if exc.num_idefs >= exc.max_idefs {
            exc.error = FT_ERR_TOO_MANY_INSTRUCTION_DEFS;
            return;
        }
        exc.num_idefs += 1;
    }

    /* opcode must be unsigned 8-bit integer */
    if 0 > a0 || a0 > 0x00FF {
        exc.error = FT_ERR_TOO_MANY_INSTRUCTION_DEFS;
        return;
    }

    {
        let start = exc.ip + 1;
        let range = exc.cur_range;
        let d = &mut exc.idefs[def];
        d.opc = a0 as FtByte as FtUInt;
        d.start = start;
        d.range = range;
        d.active = true;
    }

    if a0 as FtULong > exc.max_ins as FtULong {
        exc.max_ins = a0 as FtByte as FtUInt;
    }

    /* Now skip the whole function definition. */
    /* We don't allow nested IDEFs & FDEFs.    */

    while skip_code(exc) == SUCCESS {
        match exc.opcode {
            0x89 | 0x2C => {
                /* IDEF */
                /* FDEF */
                exc.error = FT_ERR_NESTED_DEFS;
                return;
            }
            0x2D => {
                /* ENDF */
                exc.idefs[def].end = exc.ip;
                return;
            }
            _ => {}
        }
    }
}

/*
 *
 * PUSHING DATA ONTO THE INTERPRETER STACK
 *
 */

/// `NPUSHB[]`: PUSH N Bytes
fn ins_npushb(exc: &mut TtExecContextRec) {
    let l = exc.code_at(exc.ip + 1) as FtUShort;

    if bounds(l as FtLong, exc.stack_size + 1 - exc.top) {
        exc.error = FT_ERR_STACK_OVERFLOW;
        return;
    }

    for k in 1..=l as FtLong {
        let v = exc.code_at(exc.ip + k + 1) as FtLong;
        exc.set_arg((k - 1) as usize, v);
    }

    exc.new_top += l as FtLong;
}

/// `NPUSHW[]`: PUSH N Words
fn ins_npushw(exc: &mut TtExecContextRec) {
    let l = exc.code_at(exc.ip + 1) as FtUShort;

    if bounds(l as FtLong, exc.stack_size + 1 - exc.top) {
        exc.error = FT_ERR_STACK_OVERFLOW;
        return;
    }

    exc.ip += 2;

    for k in 0..l as usize {
        let v = get_short_ins(exc) as FtLong;
        exc.set_arg(k, v);
    }

    exc.step_ins = false;
    exc.new_top += l as FtLong;
}

/// `PUSHB[abc]`: PUSH Bytes
fn ins_pushb(exc: &mut TtExecContextRec) {
    let l = (exc.opcode - 0xB0 + 1) as FtUShort;

    if bounds(l as FtLong, exc.stack_size + 1 - exc.top) {
        exc.error = FT_ERR_STACK_OVERFLOW;
        return;
    }

    for k in 1..=l as FtLong {
        let v = exc.code_at(exc.ip + k) as FtLong;
        exc.set_arg((k - 1) as usize, v);
    }
}

/// `PUSHW[abc]`: PUSH Words
fn ins_pushw(exc: &mut TtExecContextRec) {
    let l = (exc.opcode - 0xB8 + 1) as FtUShort;

    if bounds(l as FtLong, exc.stack_size + 1 - exc.top) {
        exc.error = FT_ERR_STACK_OVERFLOW;
        return;
    }

    exc.ip += 1;

    for k in 0..l as usize {
        let v = get_short_ins(exc) as FtLong;
        exc.set_arg(k, v);
    }

    exc.step_ins = false;
}

/*
 *
 * MANAGING THE GRAPHICS STATE
 *
 */

/// Which graphics state vector an instruction sets.
#[derive(Clone, Copy)]
enum GsVector {
    Proj,
    Free,
}

/// `Ins_SxVTL`
fn ins_sxvtl(
    exc: &mut TtExecContextRec,
    a_idx1: FtUShort,
    a_idx2: FtUShort,
    vec: GsVector,
) -> bool {
    let mut opcode = exc.opcode;

    if bounds(a_idx1 as FtLong, exc.zone(exc.zp2).n_points as FtLong)
        || bounds(a_idx2 as FtLong, exc.zone(exc.zp1).n_points as FtLong)
    {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
        return FAILURE;
    }

    let p1 = exc.zone(exc.zp1).cur[a_idx2 as usize];
    let p2 = exc.zone(exc.zp2).cur[a_idx1 as usize];

    let mut a = sub_long(p1.x, p2.x);
    let mut b = sub_long(p1.y, p2.y);

    /* If p1 == p2, SPvTL and SFvTL behave the same as */
    /* SPvTCA[X] and SFvTCA[X], respectively.          */
    /*                                                 */
    /* Confirmed by Greg Hitchcock.                    */

    if a == 0 && b == 0 {
        a = 0x4000;
        opcode = 0;
    }

    if (opcode & 1) != 0 {
        let c = b; /* counter-clockwise rotation */
        b = a;
        a = neg_long(c);
    }

    let v = match vec {
        GsVector::Proj => &mut exc.gs.proj_vector,
        GsVector::Free => &mut exc.gs.free_vector,
    };
    normalize(a, b, v);

    SUCCESS
}

/// `SVTCA[a]`, `SPvTCA[a]`, `SFvTCA[a]`: Set (F and P) Vectors to
/// Coordinate Axis
fn ins_sxy_tca(exc: &mut TtExecContextRec) {
    let opcode = exc.opcode;

    let aa = ((opcode & 1) as FtShort) << 14;
    let bb = aa ^ 0x4000;

    if opcode < 4 {
        exc.gs.proj_vector.x = aa;
        exc.gs.proj_vector.y = bb;

        exc.gs.dual_vector.x = aa;
        exc.gs.dual_vector.y = bb;
    }

    if (opcode & 2) == 0 {
        exc.gs.free_vector.x = aa;
        exc.gs.free_vector.y = bb;
    }

    compute_funcs(exc);
}

/// `SPvTL[a]`: Set PVector To Line
fn ins_spvtl(exc: &mut TtExecContextRec) {
    if ins_sxvtl(
        exc,
        exc.arg(1) as FtUShort,
        exc.arg(0) as FtUShort,
        GsVector::Proj,
    ) == SUCCESS
    {
        exc.gs.dual_vector = exc.gs.proj_vector;
        compute_funcs(exc);
    }
}

/// `SFvTL[a]`: Set FVector To Line
fn ins_sfvtl(exc: &mut TtExecContextRec) {
    if ins_sxvtl(
        exc,
        exc.arg(1) as FtUShort,
        exc.arg(0) as FtUShort,
        GsVector::Free,
    ) == SUCCESS
    {
        compute_funcs(exc);
    }
}

/// `SFvTPv[]`: Set FVector To PVector
fn ins_sfvtpv(exc: &mut TtExecContextRec) {
    exc.gs.free_vector = exc.gs.proj_vector;
    compute_funcs(exc);
}

/// `SPvFS[]`: Set PVector From Stack
fn ins_spvfs(exc: &mut TtExecContextRec) {
    /* Only use low 16bits, then sign extend */
    let y = exc.arg(1) as FtShort as FtLong;
    let x = exc.arg(0) as FtShort as FtLong;

    normalize(x, y, &mut exc.gs.proj_vector);

    exc.gs.dual_vector = exc.gs.proj_vector;
    compute_funcs(exc);
}

/// `SFvFS[]`: Set FVector From Stack
fn ins_sfvfs(exc: &mut TtExecContextRec) {
    /* Only use low 16bits, then sign extend */
    let y = exc.arg(1) as FtShort as FtLong;
    let x = exc.arg(0) as FtShort as FtLong;

    normalize(x, y, &mut exc.gs.free_vector);
    compute_funcs(exc);
}

/// `GPv[]`: Get Projection Vector
fn ins_gpv(exc: &mut TtExecContextRec) {
    let v = exc.gs.proj_vector;
    exc.set_arg(0, v.x as FtLong);
    exc.set_arg(1, v.y as FtLong);
}

/// `GFv[]`: Get Freedom Vector
fn ins_gfv(exc: &mut TtExecContextRec) {
    let v = exc.gs.free_vector;
    exc.set_arg(0, v.x as FtLong);
    exc.set_arg(1, v.y as FtLong);
}

/// `SRP0[]`: Set Reference Point 0
fn ins_srp0(exc: &mut TtExecContextRec) {
    exc.gs.rp0 = exc.arg(0) as FtUShort;
}

/// `SRP1[]`: Set Reference Point 1
fn ins_srp1(exc: &mut TtExecContextRec) {
    exc.gs.rp1 = exc.arg(0) as FtUShort;
}

/// `SRP2[]`: Set Reference Point 2
fn ins_srp2(exc: &mut TtExecContextRec) {
    exc.gs.rp2 = exc.arg(0) as FtUShort;
}

/// `SMD[]`: Set Minimum Distance
fn ins_smd(exc: &mut TtExecContextRec) {
    exc.gs.minimum_distance = exc.arg(0);
}

/// `SCVTCI[]`: Set Control Value Table Cut In
fn ins_scvtci(exc: &mut TtExecContextRec) {
    exc.gs.control_value_cutin = exc.arg(0) as FtF26Dot6;
}

/// `SSWCI[]`: Set Single Width Cut In
fn ins_sswci(exc: &mut TtExecContextRec) {
    exc.gs.single_width_cutin = exc.arg(0) as FtF26Dot6;
}

/// `SSW[]`: Set Single Width
fn ins_ssw(exc: &mut TtExecContextRec) {
    exc.gs.single_width_value = ft_mul_fix(exc.arg(0), exc.tt_metrics.scale);
}

/// `FLIPON[]`: Set auto-FLIP to ON
fn ins_flipon(exc: &mut TtExecContextRec) {
    exc.gs.auto_flip = true;
}

/// `FLIPOFF[]`: Set auto-FLIP to OFF
fn ins_flipoff(exc: &mut TtExecContextRec) {
    exc.gs.auto_flip = false;
}

/// `SANGW[]`: Set ANGle Weight
fn ins_sangw() {
    /* instruction not supported anymore */
}

/// `SDB[]`: Set Delta Base
fn ins_sdb(exc: &mut TtExecContextRec) {
    exc.gs.delta_base = exc.arg(0) as FtUShort;
}

/// `SDS[]`: Set Delta Shift
fn ins_sds(exc: &mut TtExecContextRec) {
    if exc.arg(0) as FtULong > 6 {
        exc.error = FT_ERR_BAD_ARGUMENT;
    } else {
        exc.gs.delta_shift = exc.arg(0) as FtUShort;
    }
}

/// `RTHG[]`: Round To Half Grid
fn ins_rthg(exc: &mut TtExecContextRec) {
    exc.gs.round_state = TT_ROUND_TO_HALF_GRID;
    exc.func_round = round_to_half_grid;
}

/// `RTG[]`: Round To Grid
fn ins_rtg(exc: &mut TtExecContextRec) {
    exc.gs.round_state = TT_ROUND_TO_GRID;
    exc.func_round = round_to_grid;
}

/// `RTDG[]`: Round To Double Grid
fn ins_rtdg(exc: &mut TtExecContextRec) {
    exc.gs.round_state = TT_ROUND_TO_DOUBLE_GRID;
    exc.func_round = round_to_double_grid;
}

/// `RUTG[]`: Round Up To Grid
fn ins_rutg(exc: &mut TtExecContextRec) {
    exc.gs.round_state = TT_ROUND_UP_TO_GRID;
    exc.func_round = round_up_to_grid;
}

/// `RDTG[]`: Round Down To Grid
fn ins_rdtg(exc: &mut TtExecContextRec) {
    exc.gs.round_state = TT_ROUND_DOWN_TO_GRID;
    exc.func_round = round_down_to_grid;
}

/// `ROFF[]`: Round OFF
fn ins_roff(exc: &mut TtExecContextRec) {
    exc.gs.round_state = TT_ROUND_OFF;
    exc.func_round = round_none;
}

/// `SROUND[]`: Super ROUND
fn ins_sround(exc: &mut TtExecContextRec) {
    set_super_round(exc, 0x4000, exc.arg(0));

    exc.gs.round_state = TT_ROUND_SUPER;
    exc.func_round = round_super;
}

/// `S45ROUND[]`: Super ROUND 45 degrees
fn ins_s45round(exc: &mut TtExecContextRec) {
    set_super_round(exc, 0x2D41, exc.arg(0));

    exc.gs.round_state = TT_ROUND_SUPER_45;
    exc.func_round = round_super_45;
}

/// `GC[a]`: Get Coordinate projected onto
///
/// XXX: UNDOCUMENTED: Measures from the original glyph must be taken
///      along the dual projection vector!
fn ins_gc(exc: &mut TtExecContextRec) {
    let l = exc.arg(0) as FtULong;
    let r: FtF26Dot6;

    let zp2 = exc.zone(exc.zp2);
    if boundsl(l as FtLong, zp2.n_points as FtLong) {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
        r = 0;
    } else if exc.opcode & 1 != 0 {
        r = fast_dualproj(exc, zp2.org[l as usize]);
    } else {
        r = fast_project(exc, zp2.cur[l as usize]);
    }

    exc.set_arg(0, r);
}

/// `SCFS[]`: Set Coordinate From Stack
///
/// Formula:
///
///   OA := OA + ( value - OA.p )/( f.p ) * f
fn ins_scfs(exc: &mut TtExecContextRec) {
    let l = exc.arg(0) as FtUShort;

    if bounds(l as FtLong, exc.zone(exc.zp2).n_points as FtLong) {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
        return;
    }

    let k = fast_project(exc, exc.zone(exc.zp2).cur[l as usize]);

    let d = sub_long(exc.arg(1), k);
    (exc.func_move)(exc, exc.zp2, l, d);

    /* UNDOCUMENTED!  The MS rasterizer does that with */
    /* twilight points (confirmed by Greg Hitchcock)   */
    if exc.gs.gep2 == 0 {
        let z = exc.zone_mut(exc.zp2);
        z.org[l as usize] = z.cur[l as usize];
    }
}

/// `MD[a]`: Measure Distance
///
/// XXX: UNDOCUMENTED: Measure taken in the original glyph must be along
///                    the dual projection vector.
///
/// XXX: UNDOCUMENTED: Flag attributes are inverted!
///                      0 => measure distance in original outline
///                      1 => measure distance in grid-fitted outline
///
/// XXX: UNDOCUMENTED: `zp0 - zp1', and not `zp2 - zp1!
fn ins_md(exc: &mut TtExecContextRec) {
    let k = exc.arg(1) as FtUShort as usize;
    let l = exc.arg(0) as FtUShort as usize;
    let d: FtF26Dot6;

    let zp0 = exc.zone(exc.zp0);
    let zp1 = exc.zone(exc.zp1);

    if bounds(l as FtLong, zp0.n_points as FtLong) || bounds(k as FtLong, zp1.n_points as FtLong) {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
        d = 0;
    } else if exc.opcode & 1 != 0 {
        d = project(exc, zp0.cur[l], zp1.cur[k]);
    } else {
        /* XXX: UNDOCUMENTED: twilight zone special case */

        if exc.gs.gep0 == 0 || exc.gs.gep1 == 0 {
            let vec1 = zp0.org[l];
            let vec2 = zp1.org[k];

            d = dualproj(exc, vec1, vec2);
        } else {
            let vec1 = zp0.orus[l];
            let vec2 = zp1.orus[k];

            if exc.metrics.x_scale == exc.metrics.y_scale {
                /* this should be faster */
                d = ft_mul_fix(dualproj(exc, vec1, vec2), exc.metrics.x_scale);
            } else {
                let vec = FtVector {
                    x: ft_mul_fix(vec1.x.wrapping_sub(vec2.x), exc.metrics.x_scale),
                    y: ft_mul_fix(vec1.y.wrapping_sub(vec2.y), exc.metrics.y_scale),
                };

                d = fast_dualproj(exc, vec);
            }
        }
    }

    exc.set_arg(0, d);
}

/// `SDPvTL[a]`: Set Dual PVector to Line
fn ins_sdpvtl(exc: &mut TtExecContextRec) {
    let mut opcode = exc.opcode;

    let p1 = exc.arg(1) as FtUShort as usize;
    let p2 = exc.arg(0) as FtUShort as usize;

    if bounds(p2 as FtLong, exc.zone(exc.zp1).n_points as FtLong)
        || bounds(p1 as FtLong, exc.zone(exc.zp2).n_points as FtLong)
    {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
        return;
    }

    let (mut a, mut b);
    {
        let v1 = exc.zone(exc.zp1).org[p2];
        let v2 = exc.zone(exc.zp2).org[p1];

        a = sub_long(v1.x, v2.x);
        b = sub_long(v1.y, v2.y);

        /* If v1 == v2, SDPvTL behaves the same as */
        /* SVTCA[X], respectively.                 */
        /*                                         */
        /* Confirmed by Greg Hitchcock.            */

        if a == 0 && b == 0 {
            a = 0x4000;
            opcode = 0;
        }
    }

    if (opcode & 1) != 0 {
        let c = b; /* counter-clockwise rotation */
        b = a;
        a = neg_long(c);
    }

    normalize(a, b, &mut exc.gs.dual_vector);

    {
        let v1 = exc.zone(exc.zp1).cur[p2];
        let v2 = exc.zone(exc.zp2).cur[p1];

        a = sub_long(v1.x, v2.x);
        b = sub_long(v1.y, v2.y);

        if a == 0 && b == 0 {
            a = 0x4000;
            opcode = 0;
        }
    }

    if (opcode & 1) != 0 {
        let c = b; /* counter-clockwise rotation */
        b = a;
        a = neg_long(c);
    }

    normalize(a, b, &mut exc.gs.proj_vector);
    compute_funcs(exc);
}

/// The zone for a `SZPx` argument; `None` is the `default:` case.
fn zone_for(v: FtLong) -> Option<ZoneRef> {
    match v as FtInt {
        0 => Some(ZoneRef::Twilight),
        1 => Some(ZoneRef::Pts),
        _ => None,
    }
}

/// `SZP0[]`: Set Zone Pointer 0
fn ins_szp0(exc: &mut TtExecContextRec) {
    match zone_for(exc.arg(0)) {
        Some(z) => exc.zp0 = z,
        None => {
            if exc.pedantic_hinting {
                exc.error = FT_ERR_INVALID_REFERENCE;
            }
            return;
        }
    }

    exc.gs.gep0 = exc.arg(0) as FtUShort;
}

/// `SZP1[]`: Set Zone Pointer 1
fn ins_szp1(exc: &mut TtExecContextRec) {
    match zone_for(exc.arg(0)) {
        Some(z) => exc.zp1 = z,
        None => {
            if exc.pedantic_hinting {
                exc.error = FT_ERR_INVALID_REFERENCE;
            }
            return;
        }
    }

    exc.gs.gep1 = exc.arg(0) as FtUShort;
}

/// `SZP2[]`: Set Zone Pointer 2
fn ins_szp2(exc: &mut TtExecContextRec) {
    match zone_for(exc.arg(0)) {
        Some(z) => exc.zp2 = z,
        None => {
            if exc.pedantic_hinting {
                exc.error = FT_ERR_INVALID_REFERENCE;
            }
            return;
        }
    }

    exc.gs.gep2 = exc.arg(0) as FtUShort;
}

/// `SZPS[]`: Set Zone PointerS
fn ins_szps(exc: &mut TtExecContextRec) {
    match zone_for(exc.arg(0)) {
        Some(z) => exc.zp0 = z,
        None => {
            if exc.pedantic_hinting {
                exc.error = FT_ERR_INVALID_REFERENCE;
            }
            return;
        }
    }

    exc.zp1 = exc.zp0;
    exc.zp2 = exc.zp0;

    let a = exc.arg(0) as FtUShort;
    exc.gs.gep0 = a;
    exc.gs.gep1 = a;
    exc.gs.gep2 = a;
}

/// `INSTCTRL[]`: INSTruction ConTRoL
fn ins_instctrl(exc: &mut TtExecContextRec) {
    let k = exc.arg(1) as FtULong;
    let l = exc.arg(0) as FtULong;

    /* selector values cannot be `OR'ed;                 */
    /* they are indices starting with index 1, not flags */
    if !(1..=3).contains(&k) {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
        return;
    }

    /* convert index to flag value */
    let kf: FtULong = 1 << (k - 1);

    if l != 0 {
        /* arguments to selectors look like flag values */
        if l != kf {
            if exc.pedantic_hinting {
                exc.error = FT_ERR_INVALID_REFERENCE;
            }
            return;
        }
    }

    /* INSTCTRL should only be used in the CVT program */
    if exc.ini_range == TT_CODERANGE_CVT {
        exc.gs.instruct_control &= !(kf as FtByte);
        exc.gs.instruct_control |= l as FtByte;
    }
    /* except to change the subpixel flags temporarily */
    else if exc.ini_range == TT_CODERANGE_GLYPH && k == 3 {
        /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
        /* Native ClearType fonts sign a waiver that turns off all backward  */
        /* compatibility hacks and lets them program points to the grid like */
        /* it's 1996.  They might sign a waiver for just one glyph, though.  */
        if subpixel_hinting_minimal(exc) {
            exc.backward_compatibility = l != 4;
        }
    } else if exc.pedantic_hinting {
        exc.error = FT_ERR_INVALID_REFERENCE;
    }
}

/// `SCANCTRL[]`: SCAN ConTRoL
fn ins_scanctrl(exc: &mut TtExecContextRec) {
    let a0 = exc.arg(0);

    /* Get Threshold */
    let a = (a0 & 0xFF) as FtInt;

    if a == 0xFF {
        exc.gs.scan_control = true;
        return;
    } else if a == 0 {
        exc.gs.scan_control = false;
        return;
    }

    let ppem = exc.tt_metrics.ppem as FtInt;

    if (a0 & 0x100) != 0 && ppem <= a {
        exc.gs.scan_control = true;
    }

    if (a0 & 0x200) != 0 && exc.tt_metrics.rotated {
        exc.gs.scan_control = true;
    }

    if (a0 & 0x400) != 0 && exc.tt_metrics.stretched {
        exc.gs.scan_control = true;
    }

    if (a0 & 0x800) != 0 && ppem > a {
        exc.gs.scan_control = false;
    }

    if (a0 & 0x1000) != 0 && exc.tt_metrics.rotated {
        exc.gs.scan_control = false;
    }

    if (a0 & 0x2000) != 0 && exc.tt_metrics.stretched {
        exc.gs.scan_control = false;
    }
}

/// `SCANTYPE[]`: SCAN TYPE
fn ins_scantype(exc: &mut TtExecContextRec) {
    if exc.arg(0) >= 0 {
        exc.gs.scan_type = (exc.arg(0) as FtInt) & 0xFFFF;
    }
}

/*
 *
 * MANAGING OUTLINES
 *
 */

/// `FLIPPT[]`: FLIP PoinT
fn ins_flippt(exc: &mut TtExecContextRec) {
    'fail: {
        /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
        /* See `ttinterp.h' for details on backward compatibility mode. */
        if subpixel_hinting_minimal(exc)
            && exc.backward_compatibility
            && exc.iupx_called
            && exc.iupy_called
        {
            break 'fail;
        }

        if exc.top < exc.gs.loop_ {
            if exc.pedantic_hinting {
                exc.error = FT_ERR_TOO_FEW_ARGUMENTS;
            }
            break 'fail;
        }

        while exc.gs.loop_ > 0 {
            exc.args -= 1;

            let point = exc.stack[exc.args as usize] as FtUShort;

            if bounds(point as FtLong, exc.pts.n_points as FtLong) {
                if exc.pedantic_hinting {
                    exc.error = FT_ERR_INVALID_REFERENCE;
                    return;
                }
            } else {
                exc.pts.tags[point as usize] ^= FT_CURVE_TAG_ON;
            }

            exc.gs.loop_ -= 1;
        }
    }

    /* Fail: */
    exc.gs.loop_ = 1;
    exc.new_top = exc.args;
}

/// `FLIPRGON[]`: FLIP RanGe ON
fn ins_fliprgon(exc: &mut TtExecContextRec) {
    /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
    /* See `ttinterp.h' for details on backward compatibility mode. */
    if subpixel_hinting_minimal(exc)
        && exc.backward_compatibility
        && exc.iupx_called
        && exc.iupy_called
    {
        return;
    }

    let k = exc.arg(1) as FtUShort;
    let l = exc.arg(0) as FtUShort;

    if bounds(k as FtLong, exc.pts.n_points as FtLong)
        || bounds(l as FtLong, exc.pts.n_points as FtLong)
    {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
        return;
    }

    for i in l..=k {
        exc.pts.tags[i as usize] |= FT_CURVE_TAG_ON;
    }
}

/// `FLIPRGOFF`: FLIP RanGe OFF
fn ins_fliprgoff(exc: &mut TtExecContextRec) {
    /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
    /* See `ttinterp.h' for details on backward compatibility mode. */
    if subpixel_hinting_minimal(exc)
        && exc.backward_compatibility
        && exc.iupx_called
        && exc.iupy_called
    {
        return;
    }

    let k = exc.arg(1) as FtUShort;
    let l = exc.arg(0) as FtUShort;

    if bounds(k as FtLong, exc.pts.n_points as FtLong)
        || bounds(l as FtLong, exc.pts.n_points as FtLong)
    {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
        return;
    }

    for i in l..=k {
        exc.pts.tags[i as usize] &= !FT_CURVE_TAG_ON;
    }
}

/// `Compute_Point_Displacement`; returns `(x, y, zone, refp)` on SUCCESS.
fn compute_point_displacement(
    exc: &mut TtExecContextRec,
) -> Option<(FtF26Dot6, FtF26Dot6, ZoneRef, FtUShort)> {
    let (zp, p) = if exc.opcode & 1 != 0 {
        (exc.zp0, exc.gs.rp1)
    } else {
        (exc.zp1, exc.gs.rp2)
    };

    if bounds(p as FtLong, exc.zone(zp).n_points as FtLong) {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
        return None;
    }

    let z = exc.zone(zp);
    let d = project(exc, z.cur[p as usize], z.org[p as usize]);

    let x = ft_mul_div(d, exc.gs.free_vector.x as FtLong, exc.f_dot_p);
    let y = ft_mul_div(d, exc.gs.free_vector.y as FtLong, exc.f_dot_p);

    Some((x, y, zp, p))
}

/// `Move_Zp2_Point`: See `ttinterp.h' for details on backward
/// compatibility mode.
fn move_zp2_point(
    exc: &mut TtExecContextRec,
    point: FtUShort,
    dx: FtF26Dot6,
    dy: FtF26Dot6,
    touch: bool,
) {
    let p = point as usize;

    if exc.gs.free_vector.x != 0 {
        /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
        if !(subpixel_hinting_minimal(exc) && exc.backward_compatibility) {
            let z = exc.zone_mut(exc.zp2);
            z.cur[p].x = add_long(z.cur[p].x, dx);
        }

        if touch {
            exc.zone_mut(exc.zp2).tags[p] |= FT_CURVE_TAG_TOUCH_X;
        }
    }

    if exc.gs.free_vector.y != 0 {
        /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
        if !(subpixel_hinting_minimal(exc)
            && exc.backward_compatibility
            && exc.iupx_called
            && exc.iupy_called)
        {
            let z = exc.zone_mut(exc.zp2);
            z.cur[p].y = add_long(z.cur[p].y, dy);
        }

        if touch {
            exc.zone_mut(exc.zp2).tags[p] |= FT_CURVE_TAG_TOUCH_Y;
        }
    }
}

/// `SHP[a]`: SHift Point by the last point
fn ins_shp(exc: &mut TtExecContextRec) {
    'fail: {
        if exc.top < exc.gs.loop_ {
            if exc.pedantic_hinting {
                exc.error = FT_ERR_INVALID_REFERENCE;
            }
            break 'fail;
        }

        let (dx, dy) = match compute_point_displacement(exc) {
            Some((dx, dy, _, _)) => (dx, dy),
            None => return,
        };

        while exc.gs.loop_ > 0 {
            exc.args -= 1;
            let point = exc.stack[exc.args as usize] as FtUShort;

            if bounds(point as FtLong, exc.zone(exc.zp2).n_points as FtLong) {
                if exc.pedantic_hinting {
                    exc.error = FT_ERR_INVALID_REFERENCE;
                    return;
                }
            } else {
                move_zp2_point(exc, point, dx, dy, true);
            }

            exc.gs.loop_ -= 1;
        }
    }

    /* Fail: */
    exc.gs.loop_ = 1;
    exc.new_top = exc.args;
}

/// `SHC[a]`: SHift Contour
///
/// UNDOCUMENTED: According to Greg Hitchcock, there is one (virtual)
///               contour in the twilight zone, namely contour number
///               zero which includes all points of it.
fn ins_shc(exc: &mut TtExecContextRec) {
    let contour = exc.arg(0) as FtShort;
    let bounds_ = if exc.gs.gep2 == 0 {
        1
    } else {
        exc.zone(exc.zp2).n_contours
    };

    if bounds(contour as FtLong, bounds_ as FtLong) {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
        return;
    }

    let (dx, dy, zp, refp) = match compute_point_displacement(exc) {
        Some(r) => r,
        None => return,
    };

    let zp2 = exc.zone(exc.zp2);
    let start: FtUShort = if contour == 0 {
        0
    } else {
        (zp2.contours[(contour - 1) as usize] as i32 + 1 - zp2.first_point as i32) as FtUShort
    };

    /* we use the number of points if in the twilight zone */
    let limit: FtUShort = if exc.gs.gep2 == 0 {
        zp2.n_points
    } else {
        (zp2.contours[contour as usize] as i32 - zp2.first_point as i32 + 1) as FtUShort
    };

    let same_zone = zp == exc.zp2;
    for i in start..limit {
        if !same_zone || refp != i {
            move_zp2_point(exc, i, dx, dy, true);
        }
    }
}

/// `SHZ[a]`: SHift Zone
fn ins_shz(exc: &mut TtExecContextRec) {
    if bounds(exc.arg(0), 2) {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
        return;
    }

    let (dx, dy, zp, refp) = match compute_point_displacement(exc) {
        Some(r) => r,
        None => return,
    };

    /* XXX: UNDOCUMENTED! SHZ doesn't move the phantom points.     */
    /*      Twilight zone has no real contours, so use `n_points'. */
    /*      Normal zone's `n_points' includes phantoms, so must    */
    /*      use end of last contour.                               */
    let zp2 = exc.zone(exc.zp2);
    let limit: FtUShort = if exc.gs.gep2 == 0 {
        zp2.n_points
    } else if exc.gs.gep2 == 1 && zp2.n_contours > 0 {
        (zp2.contours[(zp2.n_contours - 1) as usize] as i32 + 1) as FtUShort
    } else {
        0
    };

    /* XXX: UNDOCUMENTED! SHZ doesn't touch the points */
    let same_zone = zp == exc.zp2;
    for i in 0..limit {
        if !same_zone || refp != i {
            move_zp2_point(exc, i, dx, dy, false);
        }
    }
}

/// `SHPIX[]`: SHift points by a PIXel amount
fn ins_shpix(exc: &mut TtExecContextRec) {
    /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
    let in_twilight = exc.gs.gep0 == 0 || exc.gs.gep1 == 0 || exc.gs.gep2 == 0;

    'fail: {
        if exc.top < exc.gs.loop_ + 1 {
            if exc.pedantic_hinting {
                exc.error = FT_ERR_INVALID_REFERENCE;
            }
            break 'fail;
        }

        let dx = tt_mul_fix14(exc.arg(0) as FtInt32, exc.gs.free_vector.x as FtInt) as FtF26Dot6;
        let dy = tt_mul_fix14(exc.arg(0) as FtInt32, exc.gs.free_vector.y as FtInt) as FtF26Dot6;

        while exc.gs.loop_ > 0 {
            exc.args -= 1;

            let point = exc.stack[exc.args as usize] as FtUShort;

            if bounds(point as FtLong, exc.zone(exc.zp2).n_points as FtLong) {
                if exc.pedantic_hinting {
                    exc.error = FT_ERR_INVALID_REFERENCE;
                    return;
                }
            } else if subpixel_hinting_minimal(exc) && exc.backward_compatibility {
                /* Special case: allow SHPIX to move points in the twilight zone.  */
                /* Otherwise, treat SHPIX the same as DELTAP.  Unbreaks various    */
                /* fonts such as older versions of Rokkitt and DTL Argo T Light    */
                /* that would glitch severely after calling ALIGNRP after a        */
                /* blocked SHPIX.                                                  */
                if in_twilight
                    || (!(exc.iupx_called && exc.iupy_called)
                        && ((exc.is_composite && exc.gs.free_vector.y != 0)
                            || (exc.zone(exc.zp2).tags[point as usize] & FT_CURVE_TAG_TOUCH_Y)
                                != 0))
                {
                    move_zp2_point(exc, point, 0, dy, true);
                }
            } else {
                move_zp2_point(exc, point, dx, dy, true);
            }

            exc.gs.loop_ -= 1;
        }
    }

    /* Fail: */
    exc.gs.loop_ = 1;
    exc.new_top = exc.args;
}

/// `MSIRP[a]`: Move Stack Indirect Relative Position
fn ins_msirp(exc: &mut TtExecContextRec) {
    let point = exc.arg(0) as FtUShort;
    let p = point as usize;
    let rp0 = exc.gs.rp0 as usize;

    if bounds(point as FtLong, exc.zone(exc.zp1).n_points as FtLong)
        || bounds(exc.gs.rp0 as FtLong, exc.zone(exc.zp0).n_points as FtLong)
    {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
        return;
    }

    /* UNDOCUMENTED!  The MS rasterizer does that with */
    /* twilight points (confirmed by Greg Hitchcock)   */
    if exc.gs.gep1 == 0 {
        let o = exc.zone(exc.zp0).org[rp0];
        exc.zone_mut(exc.zp1).org[p] = o;
        let a1 = exc.arg(1);
        (exc.func_move_orig)(exc, exc.zp1, point, a1);
        let z = exc.zone_mut(exc.zp1);
        z.cur[p] = z.org[p];
    }

    let distance = project(exc, exc.zone(exc.zp1).cur[p], exc.zone(exc.zp0).cur[rp0]);

    let d = sub_long(exc.arg(1), distance);
    (exc.func_move)(exc, exc.zp1, point, d);

    exc.gs.rp1 = exc.gs.rp0;
    exc.gs.rp2 = point;

    if (exc.opcode & 1) != 0 {
        exc.gs.rp0 = point;
    }
}

/// `MDAP[a]`: Move Direct Absolute Point
fn ins_mdap(exc: &mut TtExecContextRec) {
    let point = exc.arg(0) as FtUShort;

    if bounds(point as FtLong, exc.zone(exc.zp0).n_points as FtLong) {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
        return;
    }

    let distance = if (exc.opcode & 1) != 0 {
        let cur_dist = fast_project(exc, exc.zone(exc.zp0).cur[point as usize]);
        sub_long((exc.func_round)(exc, cur_dist, 3), cur_dist)
    } else {
        0
    };

    (exc.func_move)(exc, exc.zp0, point, distance);

    exc.gs.rp0 = point;
    exc.gs.rp1 = point;
}

/// `MIAP[a]`: Move Indirect Absolute Point
fn ins_miap(exc: &mut TtExecContextRec) {
    let cvt_entry = exc.arg(1) as FtULong;
    let point = exc.arg(0) as FtUShort;

    'fail: {
        if bounds(point as FtLong, exc.zone(exc.zp0).n_points as FtLong)
            || boundsl(cvt_entry as FtLong, exc.cvt_size as FtLong)
        {
            if exc.pedantic_hinting {
                exc.error = FT_ERR_INVALID_REFERENCE;
            }
            break 'fail;
        }

        /* UNDOCUMENTED!                                                      */
        /*                                                                    */
        /* The behaviour of an MIAP instruction is quite different when used  */
        /* in the twilight zone.                                              */
        /*                                                                    */
        /* First, no control value cut-in test is performed as it would fail  */
        /* anyway.  Second, the original point, i.e. (org_x,org_y) of         */
        /* zp0.point, is set to the absolute, unrounded distance found in the */
        /* CVT.                                                               */
        /*                                                                    */
        /* This is used in the CVT programs of the Microsoft fonts Arial,     */
        /* Times, etc., in order to re-adjust some key font heights.  It      */
        /* allows the use of the IP instruction in the twilight zone, which   */
        /* otherwise would be invalid according to the specification.         */
        /*                                                                    */
        /* We implement it with a special sequence for the twilight zone.     */
        /* This is a bad hack, but it seems to work.                          */
        /*                                                                    */
        /* Confirmed by Greg Hitchcock.                                       */

        let mut distance = (exc.func_read_cvt)(exc, cvt_entry);

        if exc.gs.gep0 == 0 {
            /* If in twilight zone */
            let fx = exc.gs.free_vector.x as FtInt;
            let fy = exc.gs.free_vector.y as FtInt;
            let z = exc.zone_mut(exc.zp0);
            z.org[point as usize].x = tt_mul_fix14(distance as FtInt32, fx) as FtPos;
            z.org[point as usize].y = tt_mul_fix14(distance as FtInt32, fy) as FtPos;
            z.cur[point as usize] = z.org[point as usize];
        }

        let org_dist = fast_project(exc, exc.zone(exc.zp0).cur[point as usize]);

        if (exc.opcode & 1) != 0 {
            /* rounding and control cut-in flag */
            let control_value_cutin = exc.gs.control_value_cutin;

            let mut delta = sub_long(distance, org_dist);
            if delta < 0 {
                delta = neg_long(delta);
            }

            if delta > control_value_cutin {
                distance = org_dist;
            }

            distance = (exc.func_round)(exc, distance, 3);
        }

        (exc.func_move)(exc, exc.zp0, point, sub_long(distance, org_dist));
    }

    /* Fail: */
    exc.gs.rp0 = point;
    exc.gs.rp1 = point;
}

/// `MDRP[abcde]`: Move Direct Relative Point
fn ins_mdrp(exc: &mut TtExecContextRec) {
    let point = exc.arg(0) as FtUShort;
    let p = point as usize;
    let rp0 = exc.gs.rp0 as usize;

    'fail: {
        if bounds(point as FtLong, exc.zone(exc.zp1).n_points as FtLong)
            || bounds(exc.gs.rp0 as FtLong, exc.zone(exc.zp0).n_points as FtLong)
        {
            if exc.pedantic_hinting {
                exc.error = FT_ERR_INVALID_REFERENCE;
            }
            break 'fail;
        }

        /* XXX: Is there some undocumented feature while in the */
        /*      twilight zone?                                  */

        /* XXX: UNDOCUMENTED: twilight zone special case */

        let mut org_dist: FtF26Dot6;
        if exc.gs.gep0 == 0 || exc.gs.gep1 == 0 {
            let vec1 = exc.zone(exc.zp1).org[p];
            let vec2 = exc.zone(exc.zp0).org[rp0];

            org_dist = dualproj(exc, vec1, vec2);
        } else {
            let vec1 = exc.zone(exc.zp1).orus[p];
            let vec2 = exc.zone(exc.zp0).orus[rp0];

            if exc.metrics.x_scale == exc.metrics.y_scale {
                /* this should be faster */
                org_dist = dualproj(exc, vec1, vec2);
                org_dist = ft_mul_fix(org_dist, exc.metrics.x_scale);
            } else {
                let vec = FtVector {
                    x: ft_mul_fix(sub_long(vec1.x, vec2.x), exc.metrics.x_scale),
                    y: ft_mul_fix(sub_long(vec1.y, vec2.y), exc.metrics.y_scale),
                };

                org_dist = fast_dualproj(exc, vec);
            }
        }

        /* single width cut-in test */

        /* |org_dist - single_width_value| < single_width_cutin */
        if exc.gs.single_width_cutin > 0
            && org_dist
                < exc
                    .gs
                    .single_width_value
                    .wrapping_add(exc.gs.single_width_cutin)
            && org_dist
                > exc
                    .gs
                    .single_width_value
                    .wrapping_sub(exc.gs.single_width_cutin)
        {
            if org_dist >= 0 {
                org_dist = exc.gs.single_width_value;
            } else {
                org_dist = exc.gs.single_width_value.wrapping_neg();
            }
        }

        /* round flag */

        let mut distance = if (exc.opcode & 4) != 0 {
            (exc.func_round)(exc, org_dist, (exc.opcode & 3) as FtInt)
        } else {
            round_none(exc, org_dist, (exc.opcode & 3) as FtInt)
        };

        /* minimum distance flag */

        if (exc.opcode & 8) != 0 {
            let minimum_distance = exc.gs.minimum_distance;

            if org_dist >= 0 {
                if distance < minimum_distance {
                    distance = minimum_distance;
                }
            } else if distance > neg_long(minimum_distance) {
                distance = neg_long(minimum_distance);
            }
        }

        /* now move the point */

        let org_dist = project(exc, exc.zone(exc.zp1).cur[p], exc.zone(exc.zp0).cur[rp0]);

        (exc.func_move)(exc, exc.zp1, point, sub_long(distance, org_dist));
    }

    /* Fail: */
    exc.gs.rp1 = exc.gs.rp0;
    exc.gs.rp2 = point;

    if (exc.opcode & 16) != 0 {
        exc.gs.rp0 = point;
    }
}

/// `MIRP[abcde]`: Move Indirect Relative Point
fn ins_mirp(exc: &mut TtExecContextRec) {
    let point = exc.arg(0) as FtUShort;
    let p = point as usize;
    let cvt_entry = add_long(exc.arg(1), 1) as FtULong;
    let rp0 = exc.gs.rp0 as usize;

    'fail: {
        /* XXX: UNDOCUMENTED! cvt[-1] = 0 always */

        if bounds(point as FtLong, exc.zone(exc.zp1).n_points as FtLong)
            || boundsl(cvt_entry as FtLong, exc.cvt_size as FtLong + 1)
            || bounds(exc.gs.rp0 as FtLong, exc.zone(exc.zp0).n_points as FtLong)
        {
            if exc.pedantic_hinting {
                exc.error = FT_ERR_INVALID_REFERENCE;
            }
            break 'fail;
        }

        let mut cvt_dist = if cvt_entry == 0 {
            0
        } else {
            (exc.func_read_cvt)(exc, cvt_entry - 1)
        };

        /* single width test */

        let mut delta = sub_long(cvt_dist, exc.gs.single_width_value);
        if delta < 0 {
            delta = neg_long(delta);
        }

        if delta < exc.gs.single_width_cutin {
            if cvt_dist >= 0 {
                cvt_dist = exc.gs.single_width_value;
            } else {
                cvt_dist = exc.gs.single_width_value.wrapping_neg();
            }
        }

        /* UNDOCUMENTED!  The MS rasterizer does that with */
        /* twilight points (confirmed by Greg Hitchcock)   */
        if exc.gs.gep1 == 0 {
            let o = exc.zone(exc.zp0).org[rp0];
            let fx = exc.gs.free_vector.x as FtInt;
            let fy = exc.gs.free_vector.y as FtInt;
            let z = exc.zone_mut(exc.zp1);
            z.org[p].x = add_long(o.x, tt_mul_fix14(cvt_dist as FtInt32, fx) as FtLong);
            z.org[p].y = add_long(o.y, tt_mul_fix14(cvt_dist as FtInt32, fy) as FtLong);
            z.cur[p] = z.org[p];
        }

        let org_dist = dualproj(exc, exc.zone(exc.zp1).org[p], exc.zone(exc.zp0).org[rp0]);
        let cur_dist = project(exc, exc.zone(exc.zp1).cur[p], exc.zone(exc.zp0).cur[rp0]);

        /* auto-flip test */

        if exc.gs.auto_flip && (org_dist ^ cvt_dist) < 0 {
            cvt_dist = neg_long(cvt_dist);
        }

        /* control value cut-in and round */

        let mut distance = if (exc.opcode & 4) != 0 {
            /* XXX: UNDOCUMENTED!  Only perform cut-in test when both points */
            /*      refer to the same zone.                                  */

            if exc.gs.gep0 == exc.gs.gep1 {
                let control_value_cutin = exc.gs.control_value_cutin;

                /* XXX: According to Greg Hitchcock, the following wording is */
                /*      the right one:                                        */
                /*                                                            */
                /*        When the absolute difference between the value in   */
                /*        the table [CVT] and the measurement directly from   */
                /*        the outline is _greater_ than the cut_in value, the */
                /*        outline measurement is used.                        */
                /*                                                            */
                /*      This is from `instgly.doc'.  The description in       */
                /*      `ttinst2.doc', version 1.66, is thus incorrect since  */
                /*      it implies `>=' instead of `>'.                       */

                let mut delta = sub_long(cvt_dist, org_dist);
                if delta < 0 {
                    delta = neg_long(delta);
                }

                if delta > control_value_cutin {
                    cvt_dist = org_dist;
                }
            }

            (exc.func_round)(exc, cvt_dist, (exc.opcode & 3) as FtInt)
        } else {
            round_none(exc, cvt_dist, (exc.opcode & 3) as FtInt)
        };

        /* minimum distance test */

        if (exc.opcode & 8) != 0 {
            let minimum_distance = exc.gs.minimum_distance;

            if org_dist >= 0 {
                if distance < minimum_distance {
                    distance = minimum_distance;
                }
            } else if distance > neg_long(minimum_distance) {
                distance = neg_long(minimum_distance);
            }
        }

        (exc.func_move)(exc, exc.zp1, point, sub_long(distance, cur_dist));
    }

    /* Fail: */
    exc.gs.rp1 = exc.gs.rp0;

    if (exc.opcode & 16) != 0 {
        exc.gs.rp0 = point;
    }

    exc.gs.rp2 = point;
}

/// `ALIGNRP[]`: ALIGN Relative Point
fn ins_alignrp(exc: &mut TtExecContextRec) {
    'fail: {
        if exc.top < exc.gs.loop_
            || bounds(exc.gs.rp0 as FtLong, exc.zone(exc.zp0).n_points as FtLong)
        {
            if exc.pedantic_hinting {
                exc.error = FT_ERR_INVALID_REFERENCE;
            }
            break 'fail;
        }

        while exc.gs.loop_ > 0 {
            exc.args -= 1;

            let point = exc.stack[exc.args as usize] as FtUShort;

            if bounds(point as FtLong, exc.zone(exc.zp1).n_points as FtLong) {
                if exc.pedantic_hinting {
                    exc.error = FT_ERR_INVALID_REFERENCE;
                    return;
                }
            } else {
                let distance = project(
                    exc,
                    exc.zone(exc.zp1).cur[point as usize],
                    exc.zone(exc.zp0).cur[exc.gs.rp0 as usize],
                );

                (exc.func_move)(exc, exc.zp1, point, neg_long(distance));
            }

            exc.gs.loop_ -= 1;
        }
    }

    /* Fail: */
    exc.gs.loop_ = 1;
    exc.new_top = exc.args;
}

/// `ISECT[]`: moves point to InterSECTion
fn ins_isect(exc: &mut TtExecContextRec) {
    let point = exc.arg(0) as FtUShort as usize;

    let a0 = exc.arg(1) as FtUShort as usize;
    let a1 = exc.arg(2) as FtUShort as usize;
    let b0 = exc.arg(3) as FtUShort as usize;
    let b1 = exc.arg(4) as FtUShort as usize;

    let zp0 = exc.zone(exc.zp0);
    let zp1 = exc.zone(exc.zp1);
    if bounds(b0 as FtLong, zp0.n_points as FtLong)
        || bounds(b1 as FtLong, zp0.n_points as FtLong)
        || bounds(a0 as FtLong, zp1.n_points as FtLong)
        || bounds(a1 as FtLong, zp1.n_points as FtLong)
        || bounds(point as FtLong, exc.zone(exc.zp2).n_points as FtLong)
    {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
        return;
    }

    let (zb0, zb1, za0, za1) = (zp0.cur[b0], zp0.cur[b1], zp1.cur[a0], zp1.cur[a1]);

    /* Cramer's rule */

    let dbx = sub_long(zb1.x, zb0.x);
    let dby = sub_long(zb1.y, zb0.y);

    let dax = sub_long(za1.x, za0.x);
    let day = sub_long(za1.y, za0.y);

    let dx = sub_long(zb0.x, za0.x);
    let dy = sub_long(zb0.y, za0.y);

    let discriminant = add_long(
        ft_mul_div(dax, neg_long(dby), 0x40),
        ft_mul_div(day, dbx, 0x40),
    );
    let dotproduct = add_long(ft_mul_div(dax, dbx, 0x40), ft_mul_div(day, dby, 0x40));

    /* The discriminant above is actually a cross product of vectors     */
    /* da and db. Together with the dot product, they can be used as     */
    /* surrogates for sine and cosine of the angle between the vectors.  */
    /* Indeed,                                                           */
    /*       dotproduct   = |da||db|cos(angle)                           */
    /*       discriminant = |da||db|sin(angle)     .                     */
    /* We use these equations to reject grazing intersections by         */
    /* thresholding abs(tan(angle)) at 1/19, corresponding to 3 degrees. */
    let z2 = exc.zp2;
    if mul_long(19, ft_abs(discriminant)) > ft_abs(dotproduct) {
        let val = add_long(
            ft_mul_div(dx, neg_long(dby), 0x40),
            ft_mul_div(dy, dbx, 0x40),
        );

        let r = FtVector {
            x: ft_mul_div(val, dax, discriminant),
            y: ft_mul_div(val, day, discriminant),
        };

        /* XXX: Block in backward_compatibility and/or post-IUP? */
        let z = exc.zone_mut(z2);
        z.cur[point].x = add_long(za0.x, r.x);
        z.cur[point].y = add_long(za0.y, r.y);
    } else {
        /* else, take the middle of the middles of A and B */

        /* XXX: Block in backward_compatibility and/or post-IUP? */
        let z = exc.zone_mut(z2);
        z.cur[point].x = add_long(add_long(za0.x, za1.x), add_long(zb0.x, zb1.x)) / 4;
        z.cur[point].y = add_long(add_long(za0.y, za1.y), add_long(zb0.y, zb1.y)) / 4;
    }

    exc.zone_mut(z2).tags[point] |= FT_CURVE_TAG_TOUCH_BOTH;
}

/// `ALIGNPTS[]`: ALIGN PoinTS
fn ins_alignpts(exc: &mut TtExecContextRec) {
    let p1 = exc.arg(0) as FtUShort;
    let p2 = exc.arg(1) as FtUShort;

    if bounds(p1 as FtLong, exc.zone(exc.zp1).n_points as FtLong)
        || bounds(p2 as FtLong, exc.zone(exc.zp0).n_points as FtLong)
    {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
        return;
    }

    let distance = project(
        exc,
        exc.zone(exc.zp0).cur[p2 as usize],
        exc.zone(exc.zp1).cur[p1 as usize],
    ) / 2;

    (exc.func_move)(exc, exc.zp1, p1, distance);
    (exc.func_move)(exc, exc.zp0, p2, neg_long(distance));
}

/// `IP[]`: Interpolate Point
///
/// SOMETIMES, DUMBER CODE IS BETTER CODE
fn ins_ip(exc: &mut TtExecContextRec) {
    'fail: {
        if exc.top < exc.gs.loop_ {
            if exc.pedantic_hinting {
                exc.error = FT_ERR_INVALID_REFERENCE;
            }
            break 'fail;
        }

        /*
         * We need to deal in a special way with the twilight zone.
         * Otherwise, by definition, the value of exc->twilight.orus[n] is (0,0),
         * for every n.
         */
        let twilight = exc.gs.gep0 == 0 || exc.gs.gep1 == 0 || exc.gs.gep2 == 0;

        if bounds(exc.gs.rp1 as FtLong, exc.zone(exc.zp0).n_points as FtLong) {
            if exc.pedantic_hinting {
                exc.error = FT_ERR_INVALID_REFERENCE;
            }
            break 'fail;
        }

        let rp1 = exc.gs.rp1 as usize;
        let rp2 = exc.gs.rp2 as usize;
        let zp0 = exc.zp0;

        /* (`orus_base' and `cur_base' point into zp0; they are re-read  */
        /* below since moving a point of zp2 can change them, as in C)    */
        let orus_base = |exc: &TtExecContextRec| -> FtVector {
            if twilight {
                exc.zone(zp0).org[rp1]
            } else {
                exc.zone(zp0).orus[rp1]
            }
        };
        let cur_base = |exc: &TtExecContextRec| -> FtVector { exc.zone(zp0).cur[rp1] };

        /* XXX: There are some glyphs in some braindead but popular */
        /*      fonts out there (e.g. [aeu]grave in monotype.ttf)   */
        /*      calling IP[] with bad values of rp[12].             */
        /*      Do something sane when this odd thing happens.      */
        let (old_range, cur_range): (FtF26Dot6, FtF26Dot6);
        if bounds(exc.gs.rp1 as FtLong, exc.zone(exc.zp0).n_points as FtLong)
            || bounds(exc.gs.rp2 as FtLong, exc.zone(exc.zp1).n_points as FtLong)
        {
            old_range = 0;
            cur_range = 0;
        } else {
            let ob = orus_base(exc);
            if twilight {
                old_range = dualproj(exc, exc.zone(exc.zp1).org[rp2], ob);
            } else if exc.metrics.x_scale == exc.metrics.y_scale {
                old_range = dualproj(exc, exc.zone(exc.zp1).orus[rp2], ob);
            } else {
                let o = exc.zone(exc.zp1).orus[rp2];
                let vec = FtVector {
                    x: ft_mul_fix(sub_long(o.x, ob.x), exc.metrics.x_scale),
                    y: ft_mul_fix(sub_long(o.y, ob.y), exc.metrics.y_scale),
                };

                old_range = fast_dualproj(exc, vec);
            }

            cur_range = project(exc, exc.zone(exc.zp1).cur[rp2], cur_base(exc));
        }

        while exc.gs.loop_ > 0 {
            exc.args -= 1;
            let point = exc.stack[exc.args as usize] as FtUInt;

            /* check point bounds */
            if bounds(point as FtLong, exc.zone(exc.zp2).n_points as FtLong) {
                if exc.pedantic_hinting {
                    exc.error = FT_ERR_INVALID_REFERENCE;
                    return;
                }
                exc.gs.loop_ -= 1;
                continue;
            }

            let pt = point as usize;
            let ob = orus_base(exc);
            let org_dist = if twilight {
                dualproj(exc, exc.zone(exc.zp2).org[pt], ob)
            } else if exc.metrics.x_scale == exc.metrics.y_scale {
                dualproj(exc, exc.zone(exc.zp2).orus[pt], ob)
            } else {
                let o = exc.zone(exc.zp2).orus[pt];
                let vec = FtVector {
                    x: ft_mul_fix(sub_long(o.x, ob.x), exc.metrics.x_scale),
                    y: ft_mul_fix(sub_long(o.y, ob.y), exc.metrics.y_scale),
                };

                fast_dualproj(exc, vec)
            };

            let cur_dist = project(exc, exc.zone(exc.zp2).cur[pt], cur_base(exc));

            let new_dist = if org_dist != 0 {
                if old_range != 0 {
                    ft_mul_div(org_dist, cur_range, old_range)
                } else {
                    /* This is the same as what MS does for the invalid case:  */
                    /*                                                         */
                    /*   delta = (Original_Pt - Original_RP1) -                */
                    /*           (Current_Pt - Current_RP1)         ;          */
                    /*                                                         */
                    /* In FreeType speak:                                      */
                    /*                                                         */
                    /*   delta = org_dist - cur_dist          .                */
                    /*                                                         */
                    /* We move `point' by `new_dist - cur_dist' after leaving  */
                    /* this block, thus we have                                */
                    /*                                                         */
                    /*   new_dist - cur_dist = delta                   ,       */
                    /*   new_dist - cur_dist = org_dist - cur_dist     ,       */
                    /*              new_dist = org_dist                .       */

                    org_dist
                }
            } else {
                0
            };

            (exc.func_move)(
                exc,
                exc.zp2,
                point as FtUShort,
                sub_long(new_dist, cur_dist),
            );

            exc.gs.loop_ -= 1;
        }
    }

    /* Fail: */
    exc.gs.loop_ = 1;
    exc.new_top = exc.args;
}

/// `UTP[a]`: UnTouch Point
fn ins_utp(exc: &mut TtExecContextRec) {
    let point = exc.arg(0) as FtUShort;

    if bounds(point as FtLong, exc.zone(exc.zp0).n_points as FtLong) {
        if exc.pedantic_hinting {
            exc.error = FT_ERR_INVALID_REFERENCE;
        }
        return;
    }

    let mut mask: FtByte = 0xFF;

    if exc.gs.free_vector.x != 0 {
        mask &= !FT_CURVE_TAG_TOUCH_X;
    }

    if exc.gs.free_vector.y != 0 {
        mask &= !FT_CURVE_TAG_TOUCH_Y;
    }

    exc.zone_mut(exc.zp0).tags[point as usize] &= mask;
}

/// `IUP_WorkerRec`: Local variables for Ins_IUP (the coordinate arrays of
/// the glyph zone, with `y` selecting the coordinate C reaches by pointer
/// arithmetic).
struct IupWorkerRec<'a> {
    orgs: &'a [FtVector],     /* original and current coordinate */
    curs: &'a mut [FtVector], /* arrays                          */
    orus: &'a [FtVector],
    max_points: FtUInt,
    y: bool,
}

impl IupWorkerRec<'_> {
    #[inline]
    fn get(v: &FtVector, y: bool) -> FtPos {
        if y {
            v.y
        } else {
            v.x
        }
    }

    #[inline]
    fn org(&self, i: FtUInt) -> FtPos {
        Self::get(&self.orgs[i as usize], self.y)
    }

    #[inline]
    fn orus(&self, i: FtUInt) -> FtPos {
        Self::get(&self.orus[i as usize], self.y)
    }

    #[inline]
    fn cur(&self, i: FtUInt) -> FtPos {
        Self::get(&self.curs[i as usize], self.y)
    }

    #[inline]
    fn set_cur(&mut self, i: FtUInt, v: FtPos) {
        if self.y {
            self.curs[i as usize].y = v;
        } else {
            self.curs[i as usize].x = v;
        }
    }
}

/// `iup_worker_shift_`
fn iup_worker_shift(worker: &mut IupWorkerRec, p1: FtUInt, p2: FtUInt, p: FtUInt) {
    let dx = sub_long(worker.cur(p), worker.org(p));
    if dx != 0 {
        for i in p1..p {
            let v = add_long(worker.cur(i), dx);
            worker.set_cur(i, v);
        }

        for i in p + 1..=p2 {
            let v = add_long(worker.cur(i), dx);
            worker.set_cur(i, v);
        }
    }
}

/// `iup_worker_interpolate_`
fn iup_worker_interpolate(
    worker: &mut IupWorkerRec,
    p1: FtUInt,
    p2: FtUInt,
    mut ref1: FtUInt,
    mut ref2: FtUInt,
) {
    if p1 > p2 {
        return;
    }

    if bounds(ref1 as FtLong, worker.max_points as FtLong)
        || bounds(ref2 as FtLong, worker.max_points as FtLong)
    {
        return;
    }

    let mut orus1 = worker.orus(ref1);
    let mut orus2 = worker.orus(ref2);

    if orus1 > orus2 {
        std::mem::swap(&mut orus1, &mut orus2);
        std::mem::swap(&mut ref1, &mut ref2);
    }

    let org1 = worker.org(ref1);
    let org2 = worker.org(ref2);
    let cur1 = worker.cur(ref1);
    let cur2 = worker.cur(ref2);
    let delta1 = sub_long(cur1, org1);
    let delta2 = sub_long(cur2, org2);

    if cur1 == cur2 || orus1 == orus2 {
        /* trivial snap or shift of untouched points */
        for i in p1..=p2 {
            let mut x = worker.org(i);

            if x <= org1 {
                x = add_long(x, delta1);
            } else if x >= org2 {
                x = add_long(x, delta2);
            } else {
                x = cur1;
            }

            worker.set_cur(i, x);
        }
    } else {
        let mut scale: FtFixed = 0;
        let mut scale_valid = false;

        /* interpolation */
        for i in p1..=p2 {
            let mut x = worker.org(i);

            if x <= org1 {
                x = add_long(x, delta1);
            } else if x >= org2 {
                x = add_long(x, delta2);
            } else {
                if !scale_valid {
                    scale_valid = true;
                    scale = ft_div_fix(sub_long(cur2, cur1), sub_long(orus2, orus1));
                }

                x = add_long(cur1, ft_mul_fix(sub_long(worker.orus(i), orus1), scale));
            }
            worker.set_cur(i, x);
        }
    }
}

/// `IUP[a]`: Interpolate Untouched Points
fn ins_iup(exc: &mut TtExecContextRec) {
    /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
    /* See `ttinterp.h' for details on backward compatibility mode.  */
    /* Allow IUP until it has been called on both axes.  Immediately */
    /* return on subsequent ones.                                    */
    if subpixel_hinting_minimal(exc) && exc.backward_compatibility {
        if exc.iupx_called && exc.iupy_called {
            return;
        }

        if exc.opcode & 1 != 0 {
            exc.iupx_called = true;
        } else {
            exc.iupy_called = true;
        }
    }

    /* ignore empty outlines */
    if exc.pts.n_contours == 0 {
        return;
    }

    let (mask, y) = if exc.opcode & 1 != 0 {
        (FT_CURVE_TAG_TOUCH_X, false)
    } else {
        (FT_CURVE_TAG_TOUCH_Y, true)
    };

    let pts = &mut exc.pts;
    let n_points = pts.n_points as FtUInt;
    let n_contours = pts.n_contours;
    let first_point_ = pts.first_point as FtUInt;
    let mut v = IupWorkerRec {
        orgs: &pts.org,
        curs: &mut pts.cur,
        orus: &pts.orus,
        max_points: n_points,
        y,
    };
    let tags = &pts.tags;
    let contours = &pts.contours;

    let mut contour: FtShort = 0;
    let mut point: FtUInt = 0;

    loop {
        let mut end_point = (contours[contour as usize] as FtUInt).wrapping_sub(first_point_);
        let first_point = point;

        if bounds(end_point as FtLong, n_points as FtLong) {
            end_point = n_points.wrapping_sub(1);
        }

        while point <= end_point && (tags[point as usize] & mask) == 0 {
            point += 1;
        }

        if point <= end_point {
            let first_touched = point;
            let mut cur_touched = point;

            point += 1;

            while point <= end_point {
                if (tags[point as usize] & mask) != 0 {
                    iup_worker_interpolate(&mut v, cur_touched + 1, point - 1, cur_touched, point);
                    cur_touched = point;
                }

                point += 1;
            }

            if cur_touched == first_touched {
                iup_worker_shift(&mut v, first_point, end_point, cur_touched);
            } else {
                iup_worker_interpolate(
                    &mut v,
                    (cur_touched + 1) as FtUShort as FtUInt,
                    end_point,
                    cur_touched,
                    first_touched,
                );

                if first_touched > 0 {
                    iup_worker_interpolate(
                        &mut v,
                        first_point,
                        first_touched - 1,
                        cur_touched,
                        first_touched,
                    );
                }
            }
        }
        contour += 1;

        if contour >= n_contours {
            break;
        }
    }
}

/// `DELTAPn[]`: DELTA exceptions P1, P2, P3
fn ins_deltap(exc: &mut TtExecContextRec) {
    let p = (exc.func_cur_ppem)(exc) as FtULong;
    let nump = exc.arg(0) as FtULong; /* some points theoretically may occur more
                                      than once, thus UShort isn't enough */

    'fail: {
        let mut k: FtULong = 1;
        while k <= nump {
            if exc.args < 2 {
                if exc.pedantic_hinting {
                    exc.error = FT_ERR_TOO_FEW_ARGUMENTS;
                }
                exc.args = 0;
                break 'fail;
            }

            exc.args -= 2;

            let a = exc.stack[exc.args as usize + 1] as FtUShort;
            let mut b = exc.stack[exc.args as usize];

            /* XXX: Because some popular fonts contain some invalid DeltaP */
            /*      instructions, we simply ignore them when the stacked   */
            /*      point reference is off limit, rather than returning an */
            /*      error.  As a delta instruction doesn't change a glyph  */
            /*      in great ways, this shouldn't be a problem.            */

            if !bounds(a as FtLong, exc.zone(exc.zp0).n_points as FtLong) {
                let mut c = ((b as FtULong) & 0xF0) >> 4;

                match exc.opcode {
                    0x5D => {}
                    0x71 => c += 16,
                    0x72 => c += 32,
                    _ => {}
                }

                c += exc.gs.delta_base as FtULong;

                if p == c {
                    b = (((b as FtULong) & 0xF) as FtLong) - 8;
                    if b >= 0 {
                        b += 1;
                    }
                    b = b.wrapping_mul(1 << (6 - exc.gs.delta_shift as FtLong));

                    /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
                    /* See `ttinterp.h' for details on backward compatibility */
                    /* mode.                                                  */
                    if subpixel_hinting_minimal(exc) && exc.backward_compatibility {
                        if !(exc.iupx_called && exc.iupy_called)
                            && ((exc.is_composite && exc.gs.free_vector.y != 0)
                                || (exc.zone(exc.zp0).tags[a as usize] & FT_CURVE_TAG_TOUCH_Y) != 0)
                        {
                            (exc.func_move)(exc, exc.zp0, a, b);
                        }
                    } else {
                        (exc.func_move)(exc, exc.zp0, a, b);
                    }
                }
            } else if exc.pedantic_hinting {
                exc.error = FT_ERR_INVALID_REFERENCE;
            }

            k += 1;
        }
    }

    /* Fail: */
    exc.new_top = exc.args;
}

/// `DELTACn[]`: DELTA exceptions C1, C2, C3
fn ins_deltac(exc: &mut TtExecContextRec) {
    let p = (exc.func_cur_ppem)(exc) as FtULong;
    let nump = exc.arg(0) as FtULong;

    'fail: {
        let mut k: FtULong = 1;
        while k <= nump {
            if exc.args < 2 {
                if exc.pedantic_hinting {
                    exc.error = FT_ERR_TOO_FEW_ARGUMENTS;
                }
                exc.args = 0;
                break 'fail;
            }

            exc.args -= 2;

            let a = exc.stack[exc.args as usize + 1] as FtULong;
            let mut b = exc.stack[exc.args as usize];

            if boundsl(a as FtLong, exc.cvt_size as FtLong) {
                if exc.pedantic_hinting {
                    exc.error = FT_ERR_INVALID_REFERENCE;
                    return;
                }
            } else {
                let mut c = ((b as FtULong) & 0xF0) >> 4;

                match exc.opcode {
                    0x73 => {}
                    0x74 => c += 16,
                    0x75 => c += 32,
                    _ => {}
                }

                c += exc.gs.delta_base as FtULong;

                if p == c {
                    b = (((b as FtULong) & 0xF) as FtLong) - 8;
                    if b >= 0 {
                        b += 1;
                    }
                    b = b.wrapping_mul(1 << (6 - exc.gs.delta_shift as FtLong));

                    (exc.func_move_cvt)(exc, a, b);
                }
            }

            k += 1;
        }
    }

    /* Fail: */
    exc.new_top = exc.args;
}

/*
 *
 * MISC. INSTRUCTIONS
 *
 */

/// `GETINFO[]`: GET INFOrmation
fn ins_getinfo(exc: &mut TtExecContextRec) {
    let a0 = exc.arg(0);
    let mut k: FtLong = 0;

    if (a0 & 1) != 0 {
        k = exc.interpreter_version as FtLong;
    }

    /*
     * GLYPH ROTATED
     * Selector Bit:  1
     * Return Bit(s): 8
     */
    if (a0 & 2) != 0 && exc.tt_metrics.rotated {
        k |= 1 << 8;
    }

    /*
     * GLYPH STRETCHED
     * Selector Bit:  2
     * Return Bit(s): 9
     */
    if (a0 & 4) != 0 && exc.tt_metrics.stretched {
        k |= 1 << 9;
    }

    /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
    /*
     * VARIATION GLYPH
     * Selector Bit:  3
     * Return Bit(s): 10
     */
    if (a0 & 8) != 0 && exc.blend_num_axis.is_some() {
        k |= 1 << 10;
    }

    /*
     * BI-LEVEL HINTING AND
     * GRAYSCALE RENDERING
     * Selector Bit:  5
     * Return Bit(s): 12
     */
    if (a0 & 32) != 0 && exc.grayscale {
        k |= 1 << 12;
    }

    /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
    /* Toggle the following flags only outside of monochrome mode.      */
    /* Otherwise, instructions may behave weirdly and rendering results */
    /* may differ between v35 and v40 mode, e.g., in `Times New Roman   */
    /* Bold Italic'. */
    if subpixel_hinting_minimal(exc) && exc.subpixel_hinting_lean {
        /*
         * HINTING FOR SUBPIXEL
         * Selector Bit:  6
         * Return Bit(s): 13
         *
         * v40 does subpixel hinting by default.
         */
        if (a0 & 64) != 0 {
            k |= 1 << 13;
        }

        /*
         * VERTICAL LCD SUBPIXELS?
         * Selector Bit:  8
         * Return Bit(s): 15
         */
        if (a0 & 256) != 0 && exc.vertical_lcd_lean {
            k |= 1 << 15;
        }

        /*
         * SUBPIXEL POSITIONED?
         * Selector Bit:  10
         * Return Bit(s): 17
         *
         * XXX: FreeType supports it, dependent on what client does?
         */
        if (a0 & 1024) != 0 {
            k |= 1 << 17;
        }

        /*
         * SYMMETRICAL SMOOTHING
         * Selector Bit:  11
         * Return Bit(s): 18
         *
         * The only smoothing method FreeType supports unless someone sets
         * FT_LOAD_TARGET_MONO.
         */
        if (a0 & 2048) != 0 && exc.subpixel_hinting_lean {
            k |= 1 << 18;
        }

        /*
         * CLEARTYPE HINTING AND
         * GRAYSCALE RENDERING
         * Selector Bit:  12
         * Return Bit(s): 19
         *
         * Grayscale rendering is what FreeType does anyway unless someone
         * sets FT_LOAD_TARGET_MONO or FT_LOAD_TARGET_LCD(_V)
         */
        if (a0 & 4096) != 0 && exc.grayscale_cleartype {
            k |= 1 << 19;
        }
    }

    exc.set_arg(0, k);
}

/// `GETVARIATION[]`: get normalized variation (blend) coordinates
///
/// XXX: UNDOCUMENTED!  There is no official documentation from Apple for
///      this bytecode instruction.  Active only if a font has GX
///      variation axes.
fn ins_getvariation(exc: &mut TtExecContextRec) {
    let num_axes = exc.blend_num_axis.unwrap_or(0);

    if bounds(num_axes as FtLong, exc.stack_size + 1 - exc.top) {
        exc.error = FT_ERR_STACK_OVERFLOW;
        return;
    }

    let coords = exc.blend_coords.take();
    match &coords {
        Some(coords) => {
            for i in 0..num_axes as usize {
                exc.set_arg(i, coords[i] >> 2); /* convert 16.16 to 2.14 format */
            }
        }
        None => {
            for i in 0..num_axes as usize {
                exc.set_arg(i, 0);
            }
        }
    }
    exc.blend_coords = coords;
}

/// `GETDATA[]`: no idea what this is good for
///
/// XXX: UNDOCUMENTED!  There is no documentation from Apple for this
///      very weird bytecode instruction.
fn ins_getdata(exc: &mut TtExecContextRec) {
    exc.set_arg(0, 17);
}

/// `Ins_UNKNOWN`
fn ins_unknown(exc: &mut TtExecContextRec) {
    let limit = exc.num_idefs as usize;

    for def in 0..limit {
        let d = exc.idefs[def];
        if d.opc as FtByte == exc.opcode && d.active {
            if exc.call_top >= exc.call_size {
                exc.error = FT_ERR_STACK_OVERFLOW;
                return;
            }

            let call = exc.call_top as usize;
            exc.call_top += 1;

            exc.call_stack[call] = TtCallRec {
                caller_range: exc.cur_range,
                caller_ip: exc.ip + 1,
                cur_count: 1,
                def: DefRef::I(def),
            };

            let _ = ins_goto_code_range(exc, d.range, d.start);

            exc.step_ins = false;
            return;
        }
    }

    exc.error = FT_ERR_INVALID_OPCODE;
}

/*
 *
 * RUN
 *
 * This function executes a run of opcodes.  It will exit in the
 * following cases:
 *
 * - Errors (in which case it returns FALSE).
 *
 * - Reaching the end of the main code range (returns TRUE).
 *   Reaching the end of a code range within a function call is an
 *   error.
 *
 * - After executing one single opcode, if the flag `Instruction_Trap'
 *   is set to TRUE (returns TRUE).
 *
 * On exit with TRUE, test IP < CodeSize to know whether it comes from
 * an instruction trap or a normal termination.
 *
 *
 * Note: The documented DEBUG opcode pops a value from the stack.  This
 *       behaviour is unsupported; here a DEBUG opcode is always an
 *       error.
 *
 *
 * THIS IS THE INTERPRETER'S MAIN LOOP.
 *
 */

/// `TT_RunIns` (documentation is in ttinterp.h)
pub fn tt_run_ins(exc: &mut TtExecContextRec) -> FtResult<()> {
    let mut ins_counter: FtULong = 0; /* executed instructions counter */

    /* We restrict the number of twilight points to a reasonable,     */
    /* heuristic value to avoid slow execution of malformed bytecode. */
    let mut num_twilight_points: FtULong =
        std::cmp::max(30, 2 * (exc.pts.n_points as FtULong + exc.cvt_size));
    if exc.twilight.n_points as FtULong > num_twilight_points {
        if num_twilight_points > 0xFFFF {
            num_twilight_points = 0xFFFF;
        }

        exc.twilight.n_points = num_twilight_points as FtUShort;
    }

    /* Set up loop detectors.  We restrict the number of LOOPCALL loops */
    /* and the number of JMPR, JROT, and JROF calls with a negative     */
    /* argument to values that depend on various parameters like the    */
    /* size of the CVT table or the number of points in the current     */
    /* glyph (if applicable).                                           */
    /*                                                                  */
    /* The idea is that in real-world bytecode you either iterate over  */
    /* all CVT entries (in the `prep' table), or over all points (or    */
    /* contours, in the `glyf' table) of a glyph, and such iterations   */
    /* don't happen very often.                                         */
    exc.loopcall_counter = 0;
    exc.neg_jump_counter = 0;

    /* The maximum values are heuristic. */
    if exc.pts.n_points != 0 {
        exc.loopcall_counter_max = std::cmp::max(50, 10 * exc.pts.n_points as FtULong)
            + std::cmp::max(50, exc.cvt_size / 10);
    } else {
        exc.loopcall_counter_max = 300 + 22 * exc.cvt_size;
    }

    /* as a protection against an unreasonable number of CVT entries  */
    /* we assume at most 100 control values per glyph for the counter */
    if exc.loopcall_counter_max > 100u64.wrapping_mul(exc.num_glyphs as FtULong) {
        exc.loopcall_counter_max = 100u64.wrapping_mul(exc.num_glyphs as FtULong);
    }

    exc.neg_jump_counter_max = exc.loopcall_counter_max;

    /* set PPEM and CVT functions */
    exc.tt_metrics.ratio = 0;
    if exc.metrics.x_ppem != exc.metrics.y_ppem {
        /* non-square pixels, use the stretched routines */
        exc.func_cur_ppem = current_ppem_stretched;
        exc.func_read_cvt = read_cvt_stretched;
        exc.func_write_cvt = write_cvt_stretched;
        exc.func_move_cvt = move_cvt_stretched;
    } else {
        /* square pixels, use normal routines */
        exc.func_cur_ppem = current_ppem;
        exc.func_read_cvt = read_cvt;
        exc.func_write_cvt = write_cvt;
        exc.func_move_cvt = move_cvt;
    }

    exc.ini_range = exc.cur_range;

    compute_funcs(exc);
    compute_round(exc, exc.gs.round_state as FtByte);

    /* These flags cancel execution of some opcodes after IUP is called */
    /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
    exc.iupx_called = false;
    exc.iupy_called = false;

    loop {
        exc.opcode = exc.code_at(exc.ip);

        exc.length = OPCODE_LENGTH[exc.opcode as usize] as FtInt;
        if exc.length < 0 {
            if exc.ip + 1 >= exc.code_size {
                /* LErrorCodeOverflow_: */
                exc.error = FT_ERR_CODE_OVERFLOW;
                return Err(exc.error);
            }
            exc.length = 2 - exc.length * exc.code_at(exc.ip + 1) as FtInt;
        }

        if exc.ip + exc.length as FtLong > exc.code_size {
            /* LErrorCodeOverflow_: */
            exc.error = FT_ERR_CODE_OVERFLOW;
            return Err(exc.error);
        }

        /* First, let's check for empty stack and overflow */
        let pops = (POP_PUSH_COUNT[exc.opcode as usize] >> 4) as FtLong;
        exc.args = exc.top - pops;

        /* `args' is the top of the stack once arguments have been popped. */
        /* One can also interpret it as the index of the last argument.    */
        if exc.args < 0 {
            if exc.pedantic_hinting {
                exc.error = FT_ERR_TOO_FEW_ARGUMENTS;
                return Err(exc.error);
            }

            /* push zeroes onto the stack */
            for i in 0..pops as usize {
                exc.stack[i] = 0;
            }
            exc.args = 0;
        }

        /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
        if exc.opcode == 0x91 {
            /* this is very special: GETVARIATION returns */
            /* a variable number of arguments             */

            /* it is the job of the application to `activate' GX handling, */
            /* that is, calling any of the GX API functions on the current */
            /* font to select a variation instance                         */
            if let Some(n) = exc.blend_num_axis {
                exc.new_top = exc.args + n as FtLong;
            }
        } else {
            exc.new_top = exc.args + (POP_PUSH_COUNT[exc.opcode as usize] & 15) as FtLong;
        }

        /* `new_top' is the new top of the stack, after the instruction's */
        /* execution.  `top' will be set to `new_top' after the `switch'  */
        /* statement.                                                     */
        if exc.new_top > exc.stack_size {
            exc.error = FT_ERR_STACK_OVERFLOW;
            return Err(exc.error);
        }

        exc.step_ins = true;
        exc.error = 0;

        {
            let opcode = exc.opcode;

            match opcode {
                0x00 | 0x01 | 0x02 | 0x03 | 0x04 | 0x05 => {
                    /* SVTCA y  */
                    /* SVTCA x  */
                    /* SPvTCA y */
                    /* SPvTCA x */
                    /* SFvTCA y */
                    /* SFvTCA x */
                    ins_sxy_tca(exc);
                }

                0x06 | 0x07 => {
                    /* SPvTL // */
                    /* SPvTL +  */
                    ins_spvtl(exc);
                }

                0x08 | 0x09 => {
                    /* SFvTL // */
                    /* SFvTL +  */
                    ins_sfvtl(exc);
                }

                0x0A => ins_spvfs(exc), /* SPvFS */

                0x0B => ins_sfvfs(exc), /* SFvFS */

                0x0C => ins_gpv(exc), /* GPv */

                0x0D => ins_gfv(exc), /* GFv */

                0x0E => ins_sfvtpv(exc), /* SFvTPv */

                0x0F => ins_isect(exc), /* ISECT  */

                0x10 => ins_srp0(exc), /* SRP0 */

                0x11 => ins_srp1(exc), /* SRP1 */

                0x12 => ins_srp2(exc), /* SRP2 */

                0x13 => ins_szp0(exc), /* SZP0 */

                0x14 => ins_szp1(exc), /* SZP1 */

                0x15 => ins_szp2(exc), /* SZP2 */

                0x16 => ins_szps(exc), /* SZPS */

                0x17 => ins_sloop(exc), /* SLOOP */

                0x18 => ins_rtg(exc), /* RTG */

                0x19 => ins_rthg(exc), /* RTHG */

                0x1A => ins_smd(exc), /* SMD */

                0x1B => ins_else(exc), /* ELSE */

                0x1C => ins_jmpr(exc), /* JMPR */

                0x1D => ins_scvtci(exc), /* SCVTCI */

                0x1E => ins_sswci(exc), /* SSWCI */

                0x1F => ins_ssw(exc), /* SSW */

                0x20 => ins_dup(exc), /* DUP */

                0x21 => ins_pop(), /* POP */

                0x22 => ins_clear(exc), /* CLEAR */

                0x23 => ins_swap(exc), /* SWAP */

                0x24 => ins_depth(exc), /* DEPTH */

                0x25 => ins_cindex(exc), /* CINDEX */

                0x26 => ins_mindex(exc), /* MINDEX */

                0x27 => ins_alignpts(exc), /* ALIGNPTS */

                0x28 => ins_unknown(exc), /* RAW */

                0x29 => ins_utp(exc), /* UTP */

                0x2A => ins_loopcall(exc), /* LOOPCALL */

                0x2B => ins_call(exc), /* CALL */

                0x2C => ins_fdef(exc), /* FDEF */

                0x2D => ins_endf(exc), /* ENDF */

                0x2E | 0x2F => ins_mdap(exc), /* MDAP */

                0x30 | 0x31 => ins_iup(exc), /* IUP */

                0x32 | 0x33 => ins_shp(exc), /* SHP */

                0x34 | 0x35 => ins_shc(exc), /* SHC */

                0x36 | 0x37 => ins_shz(exc), /* SHZ */

                0x38 => ins_shpix(exc), /* SHPIX */

                0x39 => ins_ip(exc), /* IP    */

                0x3A | 0x3B => ins_msirp(exc), /* MSIRP */

                0x3C => ins_alignrp(exc), /* AlignRP */

                0x3D => ins_rtdg(exc), /* RTDG */

                0x3E | 0x3F => ins_miap(exc), /* MIAP */

                0x40 => ins_npushb(exc), /* NPUSHB */

                0x41 => ins_npushw(exc), /* NPUSHW */

                0x42 => ins_ws(exc), /* WS */

                0x43 => ins_rs(exc), /* RS */

                0x44 => ins_wcvtp(exc), /* WCVTP */

                0x45 => ins_rcvt(exc), /* RCVT */

                0x46 | 0x47 => ins_gc(exc), /* GC */

                0x48 => ins_scfs(exc), /* SCFS */

                0x49 | 0x4A => ins_md(exc), /* MD */

                0x4B => ins_mppem(exc), /* MPPEM */

                0x4C => ins_mps(exc), /* MPS */

                0x4D => ins_flipon(exc), /* FLIPON */

                0x4E => ins_flipoff(exc), /* FLIPOFF */

                0x4F => ins_debug(exc), /* DEBUG */

                0x50 => ins_lt(exc), /* LT */

                0x51 => ins_lteq(exc), /* LTEQ */

                0x52 => ins_gt(exc), /* GT */

                0x53 => ins_gteq(exc), /* GTEQ */

                0x54 => ins_eq(exc), /* EQ */

                0x55 => ins_neq(exc), /* NEQ */

                0x56 => ins_odd(exc), /* ODD */

                0x57 => ins_even(exc), /* EVEN */

                0x58 => ins_if(exc), /* IF */

                0x59 => ins_eif(), /* EIF */

                0x5A => ins_and(exc), /* AND */

                0x5B => ins_or(exc), /* OR */

                0x5C => ins_not(exc), /* NOT */

                0x5D => ins_deltap(exc), /* DELTAP1 */

                0x5E => ins_sdb(exc), /* SDB */

                0x5F => ins_sds(exc), /* SDS */

                0x60 => ins_add(exc), /* ADD */

                0x61 => ins_sub(exc), /* SUB */

                0x62 => ins_div(exc), /* DIV */

                0x63 => ins_mul(exc), /* MUL */

                0x64 => ins_abs(exc), /* ABS */

                0x65 => ins_neg(exc), /* NEG */

                0x66 => ins_floor(exc), /* FLOOR */

                0x67 => ins_ceiling(exc), /* CEILING */

                0x68..=0x6B => ins_round(exc), /* ROUND */

                0x6C..=0x6F => ins_nround(exc), /* NROUND */

                0x70 => ins_wcvtf(exc), /* WCVTF */

                0x71 | 0x72 => ins_deltap(exc), /* DELTAP2 */ /* DELTAP3 */

                0x73..=0x75 => ins_deltac(exc), /* DELTAC0 */ /* DELTAC1 */ /* DELTAC2 */

                0x76 => ins_sround(exc), /* SROUND */

                0x77 => ins_s45round(exc), /* S45Round */

                0x78 => ins_jrot(exc), /* JROT */

                0x79 => ins_jrof(exc), /* JROF */

                0x7A => ins_roff(exc), /* ROFF */

                0x7B => ins_unknown(exc), /* ???? */

                0x7C => ins_rutg(exc), /* RUTG */

                0x7D => ins_rdtg(exc), /* RDTG */

                0x7E => ins_sangw(), /* SANGW */

                0x7F => ins_aa(), /* AA */

                0x80 => ins_flippt(exc), /* FLIPPT */

                0x81 => ins_fliprgon(exc), /* FLIPRGON */

                0x82 => ins_fliprgoff(exc), /* FLIPRGOFF */

                0x83 | 0x84 => ins_unknown(exc), /* UNKNOWN */

                0x85 => ins_scanctrl(exc), /* SCANCTRL */

                0x86 | 0x87 => ins_sdpvtl(exc), /* SDPvTL */

                0x88 => ins_getinfo(exc), /* GETINFO */

                0x89 => ins_idef(exc), /* IDEF */

                0x8A => ins_roll(exc), /* ROLL */

                0x8B => ins_max(exc), /* MAX */

                0x8C => ins_min(exc), /* MIN */

                0x8D => ins_scantype(exc), /* SCANTYPE */

                0x8E => ins_instctrl(exc), /* INSTCTRL */

                0x8F | 0x90 => ins_unknown(exc), /* ADJUST */

                /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
                0x91 => {
                    /* it is the job of the application to `activate' GX handling, */
                    /* that is, calling any of the GX API functions on the current */
                    /* font to select a variation instance                         */
                    if exc.blend_num_axis.is_some() {
                        ins_getvariation(exc);
                    } else {
                        ins_unknown(exc);
                    }
                }

                0x92 => {
                    /* there is at least one MS font (LaoUI.ttf version 5.01) that */
                    /* uses IDEFs for 0x91 and 0x92; for this reason we activate   */
                    /* GETDATA for GX fonts only, similar to GETVARIATION          */
                    if exc.blend_num_axis.is_some() {
                        ins_getdata(exc);
                    } else {
                        ins_unknown(exc);
                    }
                }

                _ => {
                    if opcode >= 0xE0 {
                        ins_mirp(exc);
                    } else if opcode >= 0xC0 {
                        ins_mdrp(exc);
                    } else if opcode >= 0xB8 {
                        ins_pushw(exc);
                    } else if opcode >= 0xB0 {
                        ins_pushb(exc);
                    } else {
                        ins_unknown(exc);
                    }
                }
            }
        }

        let mut suite = false;
        if exc.error != 0 {
            match exc.error {
                /* looking for redefined instructions */
                FT_ERR_INVALID_OPCODE => {
                    let limit = exc.num_idefs as usize;

                    for def in 0..limit {
                        let d = exc.idefs[def];
                        if d.active && exc.opcode == d.opc as FtByte {
                            if exc.call_top >= exc.call_size {
                                exc.error = FT_ERR_INVALID_REFERENCE;
                                return Err(exc.error);
                            }

                            let callrec = exc.call_top as usize;

                            exc.call_stack[callrec] = TtCallRec {
                                caller_range: exc.cur_range,
                                caller_ip: exc.ip + 1,
                                cur_count: 1,
                                def: DefRef::I(def),
                            };

                            if ins_goto_code_range(exc, d.range, d.start) == FAILURE {
                                return Err(exc.error);
                            }

                            suite = true;
                            break;
                        }
                    }

                    if !suite {
                        exc.error = FT_ERR_INVALID_OPCODE;
                        return Err(exc.error);
                    }
                }

                _ => return Err(exc.error),
            }
        }

        if !suite {
            exc.top = exc.new_top;

            if exc.step_ins {
                exc.ip += exc.length as FtLong;
            }

            /* increment instruction counter and check if we didn't */
            /* run this program for too long (e.g. infinite loops). */
            ins_counter += 1;
            if ins_counter > TT_CONFIG_OPTION_MAX_RUNNABLE_OPCODES {
                exc.error = FT_ERR_EXECUTION_TOO_LONG;
                return Err(exc.error);
            }
        }

        /* LSuiteLabel_: */
        if exc.ip >= exc.code_size {
            if exc.call_top > 0 {
                exc.error = FT_ERR_CODE_OVERFLOW;
                return Err(exc.error);
            } else {
                /* LNo_Error_: */
                return Ok(());
            }
        }

        if exc.instruction_trap {
            break;
        }
    }

    /* LNo_Error_: */
    Ok(())
}

/// `TT_CONFIG_OPTION_MAX_RUNNABLE_OPCODES`
const TT_CONFIG_OPTION_MAX_RUNNABLE_OPCODES: FtULong = 1000000;
