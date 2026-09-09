//! The rwasm fuel schedule as seen by Cranelift.
//!
//! The per-operator table below must stay identical to `rwasm_fuel_policy::rwasm_fuel_for_operator`
//! and `rwasm_fuel_policy::is_rwasm_operator_disabled`. It cannot simply delegate to them because
//! the policy crate pins a different `wasmparser` than this crate, so the `Operator` types differ.
//! The cost constants are shared, so at least the numbers cannot drift.

pub use rwasm_fuel_policy::{
    BASE_FUEL_COST, CALL_FUEL_COST, ENTITY_FUEL_COST, LOAD_FUEL_COST, STORE_FUEL_COST,
};
use wasmparser::Operator;

pub fn rwasm_fuel_for_operator(op: &Operator<'_>) -> u32 {
    use wasmparser::Operator::*;

    match op {
        Nop | Drop => 0,

        Block { .. } | Loop { .. } | Unreachable | Return | Else | End => 0,

        Call { .. } | CallIndirect { .. } | ReturnCall { .. } | ReturnCallIndirect { .. } => {
            CALL_FUEL_COST
        }

        I32Load { .. }
        | I64Load { .. }
        | F32Load { .. }
        | F64Load { .. }
        | I32Load8S { .. }
        | I32Load8U { .. }
        | I32Load16S { .. }
        | I32Load16U { .. }
        | I64Load8S { .. }
        | I64Load8U { .. }
        | I64Load16S { .. }
        | I64Load16U { .. }
        | I64Load32S { .. }
        | I64Load32U { .. } => LOAD_FUEL_COST,

        I32Store { .. }
        | I64Store { .. }
        | F32Store { .. }
        | F64Store { .. }
        | I32Store8 { .. }
        | I32Store16 { .. }
        | I64Store8 { .. }
        | I64Store16 { .. }
        | I64Store32 { .. } => STORE_FUEL_COST,

        GlobalGet { .. }
        | GlobalSet { .. }
        | MemorySize { .. }
        | MemoryGrow { .. }
        | MemoryInit { .. }
        | DataDrop { .. }
        | MemoryCopy { .. }
        | MemoryFill { .. }
        | TableInit { .. }
        | ElemDrop { .. }
        | TableCopy { .. }
        | TableFill { .. }
        | TableGet { .. }
        | TableSet { .. }
        | TableGrow { .. }
        | TableSize { .. } => ENTITY_FUEL_COST,

        _ => BASE_FUEL_COST,
    }
}

/// Operators the rwasm subset rejects at runtime; only consulted when float support is
/// compiled out, which is the default build.
#[cfg(not(feature = "full-wasm-mode"))]
pub fn is_rwasm_operator_disabled(op: &Operator<'_>) -> bool {
    use wasmparser::Operator::*;

    match op {
        F32Load { .. }
        | F64Load { .. }
        | F32Store { .. }
        | F64Store { .. }
        | F32Eq
        | F32Ne
        | F32Lt
        | F32Gt
        | F32Le
        | F32Ge
        | F64Eq
        | F64Ne
        | F64Lt
        | F64Gt
        | F64Le
        | F64Ge
        | F32Abs
        | F32Neg
        | F32Ceil
        | F32Floor
        | F32Trunc
        | F32Nearest
        | F32Sqrt
        | F32Add
        | F32Sub
        | F32Mul
        | F32Div
        | F32Min
        | F32Max
        | F32Copysign
        | F64Abs
        | F64Neg
        | F64Ceil
        | F64Floor
        | F64Trunc
        | F64Nearest
        | F64Sqrt
        | F64Add
        | F64Sub
        | F64Mul
        | F64Div
        | F64Min
        | F64Max
        | F64Copysign
        | I32TruncF32S
        | I32TruncF32U
        | I32TruncF64S
        | I32TruncF64U
        | I64TruncF32S
        | I64TruncF32U
        | I64TruncF64S
        | I64TruncF64U
        | F32ConvertI32S
        | F32ConvertI32U
        | F32ConvertI64S
        | F32ConvertI64U
        | F32DemoteF64
        | F64ConvertI32S
        | F64ConvertI32U
        | F64ConvertI64S
        | F64ConvertI64U
        | F64PromoteF32
        | I32TruncSatF32S
        | I32TruncSatF32U
        | I32TruncSatF64S
        | I32TruncSatF64U
        | I64TruncSatF32S
        | I64TruncSatF32U
        | I64TruncSatF64S
        | I64TruncSatF64U => true,
        _ => false,
    }
}
