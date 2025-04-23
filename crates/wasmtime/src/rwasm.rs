use target_lexicon::{DefaultToHost, Triple};
use wasmtime_environ::{CompiledModuleInfo, FinishedObject, ModuleTypes, WasmFuncType};
use crate::Engine;
use wasmtime_winch::codegen::isa::x64::X64;
use wasmtime_environ::WasmValType::{I32, I64};

pub(crate) fn build_rwasm_artifacts<T: FinishedObject>(
    engine: &Engine,
    wasm: &[u8],
    dwarf_package: Option<&[u8]>,
    obj_state: &T::State,
) -> anyhow::Result<(T, Option<(CompiledModuleInfo, ModuleTypes)>)> {
    let rwasm_module = rwasm_executor::RwasmModule2::new(wasm);


    let compiler = X64::new2();

    let _ = compiler.compile_rwasm_function(rwasm_module);

    panic!("rwasm not implemented yet");
}