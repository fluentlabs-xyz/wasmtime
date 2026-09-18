//! The stack limits of the rwasm VM, which compiled code emulates so that Wasmtime traps
//! `StackOverflow` exactly where the rwasm VM does.

use crate::wasmparser::Operator;

/// The custom section that carries the frame height of every function of a module.
///
/// The rwasm compiler records, for each function in the module's index space (imports first),
/// the `StackCheck` it emits for that function's prologue: the value-stack slots of its locals
/// plus its operand peak, as a little-endian `u32`. Cranelift reads this height for the entry
/// check of [`RwasmStackLimits`], because only the rwasm translator knows the temporaries its
/// lowering pushes.
pub const RWASM_FRAMES_SECTION: &str = "rwasm.frames";

/// The stack limits of the rwasm VM.
///
/// The rwasm VM runs every function on one shared value stack of `max_stack_slots` 32-bit slots
/// (`i64` and `f64` take two) and refuses a call once `max_call_depth` frames are on its call
/// stack. Compiled code keeps both counters in the store (`VMStoreContext::rwasm_call_depth`,
/// `VMStoreContext::rwasm_stack_slots`, read and reset through `Store::rwasm_stack_counters`):
///
/// * a call site traps `StackOverflow` when the call stack is full, then publishes the callee's
///   depth and frame base (the caller's base plus its parameters, locals and the operands below
///   the arguments) and restores its own after the call; tail calls keep both counters, like
///   `ReturnCallInternal` on rwasm;
/// * a function prologue traps `StackOverflow` when its frame base plus its parameters and its
///   recorded height ([`RWASM_FRAMES_SECTION`]) exceed the window, like rwasm's `StackCheck`;
/// * with `code_snippets`, the `i64` operators that rwasm runs in a hidden snippet frame
///   ([`rwasm_snippet_frames`]) check the depth and the slots of that frame;
/// * host functions are checked by the embedder, whose import trampoline has its own frame on
///   rwasm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RwasmStackLimits {
    /// `N_MAX_RECURSION_DEPTH` of the rwasm runtime: a call traps once this many frames are on
    /// the call stack.
    pub max_call_depth: u32,
    /// The value-stack window of the rwasm runtime, `N_MAX_STACK_SIZE` plus the headroom for a
    /// compiler-injected frame.
    pub max_stack_slots: u32,
    /// `CompilationConfig::code_snippets` of the rwasm compiler: whether `i64` arithmetic runs
    /// in hidden snippet frames.
    pub code_snippets: bool,
}

/// The rwasm stack counters of a store; see [`RwasmStackLimits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RwasmStackCounters {
    /// Frames on the rwasm call stack while the running function executes: zero for the
    /// function the host called.
    pub call_depth: u32,
    /// Value-stack slots below the running function's parameters: zero for the function the
    /// host called.
    pub stack_slots: u32,
}

/// The frame the rwasm compiler hides behind a single `i64` operator when `code_snippets` is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RwasmSnippetFrames {
    /// Frames the operator pushes on the call stack: the snippet itself, plus the shared
    /// `UDivMod64` core the div/rem wrappers call.
    pub frames: u32,
    /// The snippet's `StackCheck`: the slots it needs above the operator's operands. A
    /// wrapper's height includes the core it calls, so it is the whole hidden frame.
    pub max_stack_height: u32,
}

/// The hidden frame of an `i64` operator, or `None` for an operator the rwasm compiler inlines.
///
/// The heights are the `MSH_*` constants of the rwasm instruction set (`src/isa/*.rs`); rwasm
/// pins this table to its snippet definitions in `src/wasmtime/tests.rs`.
pub fn rwasm_snippet_frames(op: &Operator<'_>) -> Option<RwasmSnippetFrames> {
    let (frames, max_stack_height) = match op {
        Operator::I64Eq | Operator::I64Ne => (1, 1),
        Operator::I64LtS
        | Operator::I64LtU
        | Operator::I64GtS
        | Operator::I64GtU
        | Operator::I64LeS
        | Operator::I64LeU
        | Operator::I64GeS
        | Operator::I64GeU => (1, 2),
        Operator::I64Add | Operator::I64Sub => (1, 4),
        Operator::I64Mul => (1, 5),
        Operator::I64DivU | Operator::I64RemU => (2, 8),
        Operator::I64DivS | Operator::I64RemS => (2, 13),
        Operator::I64Shl | Operator::I64ShrS | Operator::I64ShrU => (1, 3),
        Operator::I64Rotl | Operator::I64Rotr => (1, 4),
        _ => return None,
    };
    Some(RwasmSnippetFrames {
        frames,
        max_stack_height,
    })
}
