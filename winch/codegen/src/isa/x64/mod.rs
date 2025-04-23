use crate::{abi::{wasm_sig, ABI}, codegen::{BuiltinFunctions, CodeGen, CodeGenContext, FuncEnv, TypeConverter}, CallingConvention};

use crate::frame::{DefinedLocals, Frame};
use crate::isa::x64::masm::MacroAssembler as X64Masm;
use crate::masm::MacroAssembler;
use crate::regalloc::RegAlloc;
use crate::stack::Stack;
use crate::{
    isa::{Builder, TargetIsa},
    regset::RegBitSet,
};
use anyhow::Result;
use cranelift_codegen::settings::{self, Configurable, Flags};
use cranelift_codegen::{isa::x64::settings as x64_settings, Final, MachBufferFinalized};
use cranelift_codegen::{MachTextSectionBuilder, TextSectionBuilder};
use target_lexicon::Triple;
use wasmparser::{FuncValidator, FunctionBody, Validator, ValidatorResources};
use cranelift_codegen::isa::x64;
use wasmtime_cranelift::CompiledFunction;
use wasmtime_environ::{ModuleTranslation, ModuleTypesBuilder, Tunables, VMOffsets, VMOffsetsFields, WasmFuncType};

use self::regs::{ALL_FPR, ALL_GPR, MAX_FPR, MAX_GPR, NON_ALLOCATABLE_FPR, NON_ALLOCATABLE_GPR};

mod abi;
mod address;
mod asm;
mod masm;
// Not all the fpr and gpr constructors are used at the moment;
// in that sense, this directive is a temporary measure to avoid
// dead code warnings.
#[allow(dead_code)]
mod regs;

/// Create an ISA builder.
pub(crate) fn isa_builder(triple: Triple) -> Builder {
    Builder::new(
        triple,
        x64_settings::builder(),
        |triple, shared_flags, settings| {
            // TODO: Once enabling/disabling flags is allowed, and once features like SIMD are supported
            // ensure compatibility between shared flags and ISA flags.
            let isa_flags = x64_settings::Flags::new(&shared_flags, settings);
            let isa = X64::new(triple, shared_flags, isa_flags);
            Ok(Box::new(isa))
        },
    )
}



/// x64 ISA.
pub struct X64 {
    /// The target triple.
    triple: Triple,
    /// ISA specific flags.
    isa_flags: x64_settings::Flags,
    /// Shared flags.
    shared_flags: Flags,
}

impl X64 {
    /// Create a x64 ISA.
    pub fn new(triple: Triple, shared_flags: Flags, isa_flags: x64_settings::Flags) -> Self {
        Self {
            isa_flags,
            shared_flags,
            triple,
        }
    }

    pub fn new2() -> Self {
        let mut flag_builder = settings::builder();
        flag_builder.enable("is_pic").unwrap();
        let flags = Flags::new(flag_builder);
        let isa_flag_builder = x64::settings::builder();
        let isa_flags = x64::settings::Flags::new(&flags, &isa_flag_builder);
        Self {
            triple: Triple::host(),
            isa_flags,
            shared_flags: flags,
        }
    }

    pub fn compile_rwasm_function(
        &self,
        rwasm_module: rwasm_executor::RwasmModule2,
    ) -> Result<CompiledFunction> {
        let pointer_bytes = self.pointer_bytes();
        let vmoffsets = VMOffsets::from(VMOffsetsFields {
            ptr: pointer_bytes,
            num_imported_functions: 0,
            num_imported_tables: 0,
            num_imported_memories: 0,
            num_imported_globals: 0,
            num_imported_tags: 0,
            num_defined_tables: 0,
            num_defined_memories: 0,
            num_owned_memories: 0,
            num_defined_globals: 0,
            num_defined_tags: 0,
            num_escaped_funcs: 0,
        });

        let mut masm = X64Masm::new(
            pointer_bytes,
            self.shared_flags.clone(),
            self.isa_flags.clone(),
        )?;
        let stack = Stack::new();

        let sig = WasmFuncType::new([].into(), [].into());
        let abi_sig = wasm_sig::<abi::X64ABI>(&sig)?;

        let mut builtins   = BuiltinFunctions::new(&vmoffsets, CallingConvention::SystemV, CallingConvention::SystemV);

        let translation = ModuleTranslation::default();
        let types_builder = ModuleTypesBuilder::new(&Validator::default());

        let env = FuncEnv::new(
            &vmoffsets,
            &translation,
            &types_builder,
            &mut builtins,
            self,
            abi::X64ABI::ptr_type(),
        );
        let defined_locals = DefinedLocals::default();
        let frame = Frame::new::<abi::X64ABI>(&abi_sig, &defined_locals)?;
        let gpr = RegBitSet::int(
            ALL_GPR.into(),
            NON_ALLOCATABLE_GPR.into(),
            usize::try_from(MAX_GPR).unwrap(),
        );
        let fpr = RegBitSet::float(
            ALL_FPR.into(),
            NON_ALLOCATABLE_FPR.into(),
            usize::try_from(MAX_FPR).unwrap(),
        );

        let regalloc = RegAlloc::from(gpr, fpr);
        let codegen_context = CodeGenContext::new(regalloc, stack, frame, &vmoffsets);
        let tunables = Tunables::default_host();
        let codegen = CodeGen::new(&tunables, &mut masm, codegen_context, env, abi_sig);

        let mut body_codegen = codegen.emit_prologue()?;

        //
        // let mut ip = InstructionPtr::new(rwasm_module.code_section.as_ptr(), rwasm_module.instr_data.as_ptr());


        // body_codegen.emit(&mut body, validator)?;
        let base = body_codegen.source_location.base;

        let names = body_codegen.env.take_name_map();
        Ok(CompiledFunction::new(
            masm.finalize(base)?,
            names,
            self.function_alignment(),
        ))
    }
}

impl TargetIsa for X64 {
    fn name(&self) -> &'static str {
        "x64"
    }

    fn triple(&self) -> &Triple {
        &self.triple
    }

    fn flags(&self) -> &settings::Flags {
        &self.shared_flags
    }

    fn isa_flags(&self) -> Vec<settings::Value> {
        self.isa_flags.iter().collect()
    }

    fn compile_function(
        &self,
        sig: &WasmFuncType,
        body: &FunctionBody,
        translation: &ModuleTranslation,
        types: &ModuleTypesBuilder,
        builtins: &mut BuiltinFunctions,
        validator: &mut FuncValidator<ValidatorResources>,
        tunables: &Tunables,
    ) -> Result<CompiledFunction> {
        let pointer_bytes = self.pointer_bytes();
        let vmoffsets = VMOffsets::new(pointer_bytes, &translation.module);

        let mut body = body.get_binary_reader();
        let mut masm = X64Masm::new(
            pointer_bytes,
            self.shared_flags.clone(),
            self.isa_flags.clone(),
        )?;
        let stack = Stack::new();

        let abi_sig = wasm_sig::<abi::X64ABI>(sig)?;

        let env = FuncEnv::new(
            &vmoffsets,
            translation,
            types,
            builtins,
            self,
            abi::X64ABI::ptr_type(),
        );
        let type_converter = TypeConverter::new(env.translation, env.types);
        let defined_locals =
            DefinedLocals::new::<abi::X64ABI>(&type_converter, &mut body, validator)?;
        let frame = Frame::new::<abi::X64ABI>(&abi_sig, &defined_locals)?;
        let gpr = RegBitSet::int(
            ALL_GPR.into(),
            NON_ALLOCATABLE_GPR.into(),
            usize::try_from(MAX_GPR).unwrap(),
        );
        let fpr = RegBitSet::float(
            ALL_FPR.into(),
            NON_ALLOCATABLE_FPR.into(),
            usize::try_from(MAX_FPR).unwrap(),
        );

        let regalloc = RegAlloc::from(gpr, fpr);
        let codegen_context = CodeGenContext::new(regalloc, stack, frame, &vmoffsets);
        let codegen = CodeGen::new(tunables, &mut masm, codegen_context, env, abi_sig);

        let mut body_codegen = codegen.emit_prologue()?;

        body_codegen.emit(&mut body, validator)?;
        let base = body_codegen.source_location.base;

        let names = body_codegen.env.take_name_map();
        Ok(CompiledFunction::new(
            masm.finalize(base)?,
            names,
            self.function_alignment(),
        ))
    }

    fn text_section_builder(&self, num_funcs: usize) -> Box<dyn TextSectionBuilder> {
        Box::new(MachTextSectionBuilder::<cranelift_codegen::isa::x64::Inst>::new(num_funcs))
    }

    fn function_alignment(&self) -> u32 {
        // See `cranelift_codegen`'s value of this for more information.
        16
    }

    fn emit_unwind_info(
        &self,
        buffer: &MachBufferFinalized<Final>,
        kind: cranelift_codegen::isa::unwind::UnwindInfoKind,
    ) -> Result<Option<cranelift_codegen::isa::unwind::UnwindInfo>> {
        Ok(cranelift_codegen::isa::x64::emit_unwind_info(buffer, kind)?)
    }

    fn create_systemv_cie(&self) -> Option<gimli::write::CommonInformationEntry> {
        Some(cranelift_codegen::isa::x64::create_cie())
    }

    fn page_size_align_log2(&self) -> u8 {
        12
    }
}
