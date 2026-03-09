//! Shadow memory instrumentation for wasm modules.
//!
//! Adds a shadow memory (memory 1) mirroring memory 0.
//! - Every store to memory 0 is duplicated to memory 1.
//! - Every load from memory 0 is compared with memory 1.
//!   If the values diverge, `record_memory_diff(addr, size)` is called.
//! - Bulk memory operations (memory.copy, memory.init, memory.fill) are mirrored.
//! - memory.grow on memory 0 also grows the shadow.
//! - SIMD (v128) loads and stores are handled, including lane operations.
//! - `record_import_call(func_idx)` is called before every import call.
//! - Trampoline wrappers redirect call_indirect through imports via record_import_call.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Trace events emitted by the r3 host functions during recording.
#[derive(Debug, Serialize, Deserialize)]
pub enum R3Event {
    ImportCall { func_idx: u32 },
    /// Memory divergence: only the bytes that differ between real and shadow.
    /// Each entry is (byte_offset_from_addr, real_value).
    MemoryWrite {
        func_idx: u32,
        addr: u32,
        data: Vec<(u8, u8)>,
    },
}

/// Export name for the shadow memory added by instrumentation.
pub const SHADOW_MEMORY_EXPORT: &str = "__r3_shadow";
use wirm::ir::function::FunctionBuilder;
use wirm::ir::id::{FunctionID, LocalID, MemoryID, TypeID};
use wirm::ir::module::module_functions::FuncKind;
use wirm::ir::module::module_types::Types;
use wirm::ir::module::Module;
use wirm::ir::types::{DataType, ElementItems, InitInstr};
use wirm::iterator::iterator_trait::{IteratingInstrumenter, Iterator as WirmIterator};
use wirm::iterator::module_iterator::ModuleIterator;
use wirm::module_builder::AddLocal;
use wirm::opcode::{Inject, Instrumenter, Opcode};
use wirm::wasmparser::{BlockType, MemArg, Operator, TypeRef};

/// Get the value type on the wasm stack for a store operation.
fn store_val_type(op: &Operator) -> Option<DataType> {
    match op {
        Operator::I32Store { memarg }
        | Operator::I32Store8 { memarg }
        | Operator::I32Store16 { memarg }
            if memarg.memory == 0 =>
        {
            Some(DataType::I32)
        }
        Operator::I64Store { memarg }
        | Operator::I64Store8 { memarg }
        | Operator::I64Store16 { memarg }
        | Operator::I64Store32 { memarg }
            if memarg.memory == 0 =>
        {
            Some(DataType::I64)
        }
        Operator::F32Store { memarg } if memarg.memory == 0 => Some(DataType::F32),
        Operator::F64Store { memarg } if memarg.memory == 0 => Some(DataType::F64),
        Operator::V128Store { memarg }
        | Operator::V128Store8Lane { memarg, .. }
        | Operator::V128Store16Lane { memarg, .. }
        | Operator::V128Store32Lane { memarg, .. }
        | Operator::V128Store64Lane { memarg, .. }
            if memarg.memory == 0 =>
        {
            Some(DataType::V128)
        }
        _ => None,
    }
}

/// Get the value type produced by a simple load (stack: [addr] -> [val]).
/// Does NOT include lane loads (stack: [addr, v128] -> [v128]).
fn load_val_type(op: &Operator) -> Option<DataType> {
    match op {
        Operator::I32Load { memarg }
        | Operator::I32Load8S { memarg }
        | Operator::I32Load8U { memarg }
        | Operator::I32Load16S { memarg }
        | Operator::I32Load16U { memarg }
            if memarg.memory == 0 =>
        {
            Some(DataType::I32)
        }
        Operator::I64Load { memarg }
        | Operator::I64Load8S { memarg }
        | Operator::I64Load8U { memarg }
        | Operator::I64Load16S { memarg }
        | Operator::I64Load16U { memarg }
        | Operator::I64Load32S { memarg }
        | Operator::I64Load32U { memarg }
            if memarg.memory == 0 =>
        {
            Some(DataType::I64)
        }
        Operator::F32Load { memarg } if memarg.memory == 0 => Some(DataType::F32),
        Operator::F64Load { memarg } if memarg.memory == 0 => Some(DataType::F64),
        Operator::V128Load { memarg }
        | Operator::V128Load8x8S { memarg }
        | Operator::V128Load8x8U { memarg }
        | Operator::V128Load16x4S { memarg }
        | Operator::V128Load16x4U { memarg }
        | Operator::V128Load32x2S { memarg }
        | Operator::V128Load32x2U { memarg }
        | Operator::V128Load8Splat { memarg }
        | Operator::V128Load16Splat { memarg }
        | Operator::V128Load32Splat { memarg }
        | Operator::V128Load64Splat { memarg }
        | Operator::V128Load32Zero { memarg }
        | Operator::V128Load64Zero { memarg }
            if memarg.memory == 0 =>
        {
            Some(DataType::V128)
        }
        _ => None,
    }
}

/// Create the same memory operator but targeting memory 1 (shadow).
fn to_shadow(op: &Operator) -> Option<Operator<'static>> {
    let s = |m: &MemArg| MemArg { memory: 1, ..*m };
    match op {
        // Scalar stores
        Operator::I32Store { memarg } if memarg.memory == 0 => {
            Some(Operator::I32Store { memarg: s(memarg) })
        }
        Operator::I32Store8 { memarg } if memarg.memory == 0 => {
            Some(Operator::I32Store8 { memarg: s(memarg) })
        }
        Operator::I32Store16 { memarg } if memarg.memory == 0 => {
            Some(Operator::I32Store16 { memarg: s(memarg) })
        }
        Operator::I64Store { memarg } if memarg.memory == 0 => {
            Some(Operator::I64Store { memarg: s(memarg) })
        }
        Operator::I64Store8 { memarg } if memarg.memory == 0 => {
            Some(Operator::I64Store8 { memarg: s(memarg) })
        }
        Operator::I64Store16 { memarg } if memarg.memory == 0 => {
            Some(Operator::I64Store16 { memarg: s(memarg) })
        }
        Operator::I64Store32 { memarg } if memarg.memory == 0 => {
            Some(Operator::I64Store32 { memarg: s(memarg) })
        }
        Operator::F32Store { memarg } if memarg.memory == 0 => {
            Some(Operator::F32Store { memarg: s(memarg) })
        }
        Operator::F64Store { memarg } if memarg.memory == 0 => {
            Some(Operator::F64Store { memarg: s(memarg) })
        }
        // V128 stores
        Operator::V128Store { memarg } if memarg.memory == 0 => {
            Some(Operator::V128Store { memarg: s(memarg) })
        }
        Operator::V128Store8Lane { memarg, lane } if memarg.memory == 0 => {
            Some(Operator::V128Store8Lane {
                memarg: s(memarg),
                lane: *lane,
            })
        }
        Operator::V128Store16Lane { memarg, lane } if memarg.memory == 0 => {
            Some(Operator::V128Store16Lane {
                memarg: s(memarg),
                lane: *lane,
            })
        }
        Operator::V128Store32Lane { memarg, lane } if memarg.memory == 0 => {
            Some(Operator::V128Store32Lane {
                memarg: s(memarg),
                lane: *lane,
            })
        }
        Operator::V128Store64Lane { memarg, lane } if memarg.memory == 0 => {
            Some(Operator::V128Store64Lane {
                memarg: s(memarg),
                lane: *lane,
            })
        }
        // Scalar loads
        Operator::I32Load { memarg } if memarg.memory == 0 => {
            Some(Operator::I32Load { memarg: s(memarg) })
        }
        Operator::I32Load8S { memarg } if memarg.memory == 0 => {
            Some(Operator::I32Load8S { memarg: s(memarg) })
        }
        Operator::I32Load8U { memarg } if memarg.memory == 0 => {
            Some(Operator::I32Load8U { memarg: s(memarg) })
        }
        Operator::I32Load16S { memarg } if memarg.memory == 0 => {
            Some(Operator::I32Load16S { memarg: s(memarg) })
        }
        Operator::I32Load16U { memarg } if memarg.memory == 0 => {
            Some(Operator::I32Load16U { memarg: s(memarg) })
        }
        Operator::I64Load { memarg } if memarg.memory == 0 => {
            Some(Operator::I64Load { memarg: s(memarg) })
        }
        Operator::I64Load8S { memarg } if memarg.memory == 0 => {
            Some(Operator::I64Load8S { memarg: s(memarg) })
        }
        Operator::I64Load8U { memarg } if memarg.memory == 0 => {
            Some(Operator::I64Load8U { memarg: s(memarg) })
        }
        Operator::I64Load16S { memarg } if memarg.memory == 0 => {
            Some(Operator::I64Load16S { memarg: s(memarg) })
        }
        Operator::I64Load16U { memarg } if memarg.memory == 0 => {
            Some(Operator::I64Load16U { memarg: s(memarg) })
        }
        Operator::I64Load32S { memarg } if memarg.memory == 0 => {
            Some(Operator::I64Load32S { memarg: s(memarg) })
        }
        Operator::I64Load32U { memarg } if memarg.memory == 0 => {
            Some(Operator::I64Load32U { memarg: s(memarg) })
        }
        Operator::F32Load { memarg } if memarg.memory == 0 => {
            Some(Operator::F32Load { memarg: s(memarg) })
        }
        Operator::F64Load { memarg } if memarg.memory == 0 => {
            Some(Operator::F64Load { memarg: s(memarg) })
        }
        // V128 simple loads
        Operator::V128Load { memarg } if memarg.memory == 0 => {
            Some(Operator::V128Load { memarg: s(memarg) })
        }
        Operator::V128Load8x8S { memarg } if memarg.memory == 0 => {
            Some(Operator::V128Load8x8S { memarg: s(memarg) })
        }
        Operator::V128Load8x8U { memarg } if memarg.memory == 0 => {
            Some(Operator::V128Load8x8U { memarg: s(memarg) })
        }
        Operator::V128Load16x4S { memarg } if memarg.memory == 0 => {
            Some(Operator::V128Load16x4S { memarg: s(memarg) })
        }
        Operator::V128Load16x4U { memarg } if memarg.memory == 0 => {
            Some(Operator::V128Load16x4U { memarg: s(memarg) })
        }
        Operator::V128Load32x2S { memarg } if memarg.memory == 0 => {
            Some(Operator::V128Load32x2S { memarg: s(memarg) })
        }
        Operator::V128Load32x2U { memarg } if memarg.memory == 0 => {
            Some(Operator::V128Load32x2U { memarg: s(memarg) })
        }
        Operator::V128Load8Splat { memarg } if memarg.memory == 0 => {
            Some(Operator::V128Load8Splat { memarg: s(memarg) })
        }
        Operator::V128Load16Splat { memarg } if memarg.memory == 0 => {
            Some(Operator::V128Load16Splat { memarg: s(memarg) })
        }
        Operator::V128Load32Splat { memarg } if memarg.memory == 0 => {
            Some(Operator::V128Load32Splat { memarg: s(memarg) })
        }
        Operator::V128Load64Splat { memarg } if memarg.memory == 0 => {
            Some(Operator::V128Load64Splat { memarg: s(memarg) })
        }
        Operator::V128Load32Zero { memarg } if memarg.memory == 0 => {
            Some(Operator::V128Load32Zero { memarg: s(memarg) })
        }
        Operator::V128Load64Zero { memarg } if memarg.memory == 0 => {
            Some(Operator::V128Load64Zero { memarg: s(memarg) })
        }
        // V128 lane loads
        Operator::V128Load8Lane { memarg, lane } if memarg.memory == 0 => {
            Some(Operator::V128Load8Lane {
                memarg: s(memarg),
                lane: *lane,
            })
        }
        Operator::V128Load16Lane { memarg, lane } if memarg.memory == 0 => {
            Some(Operator::V128Load16Lane {
                memarg: s(memarg),
                lane: *lane,
            })
        }
        Operator::V128Load32Lane { memarg, lane } if memarg.memory == 0 => {
            Some(Operator::V128Load32Lane {
                memarg: s(memarg),
                lane: *lane,
            })
        }
        Operator::V128Load64Lane { memarg, lane } if memarg.memory == 0 => {
            Some(Operator::V128Load64Lane {
                memarg: s(memarg),
                lane: *lane,
            })
        }
        _ => None,
    }
}

/// Check if an operator is a V128 lane load (stack: [addr, v128] -> [v128]).
fn is_lane_load(op: &Operator) -> bool {
    matches!(
        op,
        Operator::V128Load8Lane { memarg, .. }
        | Operator::V128Load16Lane { memarg, .. }
        | Operator::V128Load32Lane { memarg, .. }
        | Operator::V128Load64Lane { memarg, .. }
        if memarg.memory == 0
    )
}

/// Get the byte size of a load operation. Only call when the op is known to be a load.
fn load_byte_size(op: &Operator) -> i32 {
    match op {
        Operator::I32Load { .. } => 4,
        Operator::I32Load8S { .. } | Operator::I32Load8U { .. } => 1,
        Operator::I32Load16S { .. } | Operator::I32Load16U { .. } => 2,
        Operator::I64Load { .. } => 8,
        Operator::I64Load8S { .. } | Operator::I64Load8U { .. } => 1,
        Operator::I64Load16S { .. } | Operator::I64Load16U { .. } => 2,
        Operator::I64Load32S { .. } | Operator::I64Load32U { .. } => 4,
        Operator::F32Load { .. } => 4,
        Operator::F64Load { .. } => 8,
        Operator::V128Load { .. } => 16,
        Operator::V128Load8x8S { .. } | Operator::V128Load8x8U { .. } => 8,
        Operator::V128Load16x4S { .. } | Operator::V128Load16x4U { .. } => 8,
        Operator::V128Load32x2S { .. } | Operator::V128Load32x2U { .. } => 8,
        Operator::V128Load8Splat { .. } => 1,
        Operator::V128Load16Splat { .. } => 2,
        Operator::V128Load32Splat { .. } => 4,
        Operator::V128Load64Splat { .. } => 8,
        Operator::V128Load32Zero { .. } => 4,
        Operator::V128Load64Zero { .. } => 8,
        Operator::V128Load8Lane { .. } => 1,
        Operator::V128Load16Lane { .. } => 2,
        Operator::V128Load32Lane { .. } => 4,
        Operator::V128Load64Lane { .. } => 8,
        _ => unreachable!("not a load operator"),
    }
}

/// Emit a not-equal comparison for the given value type.
fn emit_ne(it: &mut ModuleIterator, vt: DataType) {
    match vt {
        DataType::I32 => it.inject(Operator::I32Ne),
        DataType::I64 => it.inject(Operator::I64Ne),
        DataType::F32 => it.inject(Operator::F32Ne),
        DataType::F64 => it.inject(Operator::F64Ne),
        DataType::V128 => {
            it.inject(Operator::V128Xor);
            it.inject(Operator::V128AnyTrue);
        }
        _ => {}
    }
}

fn get_or_add_local(
    it: &mut ModuleIterator,
    slot: &mut Option<LocalID>,
    ty: DataType,
) -> LocalID {
    *slot.get_or_insert_with(|| it.add_local(ty))
}

fn get_val_local(
    it: &mut ModuleIterator,
    vt: DataType,
    val_i32: &mut Option<LocalID>,
    val_i64: &mut Option<LocalID>,
    val_f32: &mut Option<LocalID>,
    val_f64: &mut Option<LocalID>,
    val_v128: &mut Option<LocalID>,
) -> LocalID {
    match vt {
        DataType::I32 => get_or_add_local(it, val_i32, DataType::I32),
        DataType::I64 => get_or_add_local(it, val_i64, DataType::I64),
        DataType::F32 => get_or_add_local(it, val_f32, DataType::F32),
        DataType::F64 => get_or_add_local(it, val_f64, DataType::F64),
        DataType::V128 => get_or_add_local(it, val_v128, DataType::V128),
        _ => unreachable!(),
    }
}

/// Instrument a core module with shadow memory, record_memory_diff, and
/// record_import_call.
pub fn instrument_shadow(module: &mut Module) -> Result<()> {
    if module.memories.is_empty() {
        return Ok(());
    }
    // Skip modules where memory 0 is imported (e.g. WASI adapter modules).
    let has_memory_import = module
        .imports
        .iter()
        .any(|imp| matches!(imp.ty, TypeRef::Memory(_)));
    if has_memory_import {
        return Ok(());
    }
    let num_local = module
        .functions
        .iter()
        .filter(|f| matches!(f.kind(), FuncKind::Local(_)))
        .count();
    if num_local == 0 {
        return Ok(());
    }

    // ---------------------------------------------------------------
    // Phase 1: Collect original import info (before mutating module)
    // ---------------------------------------------------------------
    let import_infos: Vec<(FunctionID, Vec<DataType>, Vec<DataType>)> = {
        let mut func_idx = 0u32;
        let mut infos = Vec::new();
        for imp in module.imports.iter() {
            if let TypeRef::Func(type_idx) = imp.ty {
                let type_id = TypeID(type_idx);
                if let Some(Types::FuncType { params, results, .. }) = module.types.get(type_id) {
                    infos.push((
                        FunctionID(func_idx),
                        params.to_vec(),
                        results.to_vec(),
                    ));
                }
                func_idx += 1;
            }
        }
        infos
    };
    let num_orig_fn_imports = import_infos.len() as u32;

    // ---------------------------------------------------------------
    // Phase 2: Add r3 imports
    // ---------------------------------------------------------------
    let diff_type = module.types.add_func_type(&[DataType::I32, DataType::I32], &[]);
    let call_type = module.types.add_func_type(&[DataType::I32], &[]);

    let (record_memory_diff_id, diff_imp_id) = module.add_import_func(
        "r3".to_string(),
        "record_memory_diff".to_string(),
        diff_type,
    );
    module.imports.set_name("record_memory_diff".to_string(), diff_imp_id);

    let (record_import_call_id, call_imp_id) = module.add_import_func(
        "r3".to_string(),
        "record_import_call".to_string(),
        call_type,
    );
    module.imports.set_name("record_import_call".to_string(), call_imp_id);

    // ---------------------------------------------------------------
    // Phase 3: Add shadow memory
    // ---------------------------------------------------------------
    let mem0_ty = module
        .memories
        .get_mem_by_id(MemoryID(0))
        .ok_or_else(|| anyhow::anyhow!("no memory 0"))?
        .ty;
    let shadow_id = module.add_local_memory(mem0_ty);
    module
        .exports
        .add_export_mem(SHADOW_MEMORY_EXPORT.to_string(), *shadow_id);

    // ---------------------------------------------------------------
    // Phase 4: Create trampoline wrappers for each original import
    // ---------------------------------------------------------------
    let mut trampoline_map: HashMap<FunctionID, FunctionID> = HashMap::new();
    for (orig_func_id, params, results) in &import_infos {
        let mut fb = FunctionBuilder::new(params, results);
        fb.i32_const(**orig_func_id as i32);
        fb.call(record_import_call_id);
        for i in 0..params.len() {
            fb.local_get(LocalID(i as u32));
        }
        fb.call(*orig_func_id);
        let trampoline_id = fb.finish_module(module);
        trampoline_map.insert(*orig_func_id, trampoline_id);
    }

    // ---------------------------------------------------------------
    // Phase 5: Update element segments to use trampolines
    // ---------------------------------------------------------------
    for elem in module.elements.iter_mut() {
        match &mut elem.items {
            ElementItems::Functions(funcs) => {
                for func_id in funcs.iter_mut() {
                    if let Some(&tramp_id) = trampoline_map.get(func_id) {
                        *func_id = tramp_id;
                    }
                }
            }
            ElementItems::ConstExprs { exprs, .. } => {
                for expr in exprs.iter_mut() {
                    for instr in expr.exprs.iter_mut() {
                        if let InitInstr::RefFunc(func_id) = instr {
                            if let Some(&tramp_id) = trampoline_map.get(func_id) {
                                *func_id = tramp_id;
                            }
                        }
                    }
                }
            }
        }
    }

    // ---------------------------------------------------------------
    // Phase 6: Iterate all instructions (skip trampolines)
    // ---------------------------------------------------------------
    let skip: Vec<FunctionID> = trampoline_map.values().copied().collect();
    let mut it = ModuleIterator::new(module, &skip);

    let mut current_func: Option<u32> = None;
    let mut addr_local: Option<LocalID> = None;
    let mut val_i32: Option<LocalID> = None;
    let mut val_i64: Option<LocalID> = None;
    let mut val_f32: Option<LocalID> = None;
    let mut val_f64: Option<LocalID> = None;
    let mut val_v128: Option<LocalID> = None;
    let mut scratch_v128: Option<LocalID> = None;
    let mut bulk_arg1: Option<LocalID> = None;
    let mut bulk_arg2: Option<LocalID> = None;

    loop {
        let (loc, at_end) = it.curr_loc();
        let func_idx = match loc {
            wirm::ir::types::Location::Module { func_idx, .. } => *func_idx,
            _ => panic!("expected module location"),
        };

        if current_func != Some(func_idx) {
            current_func = Some(func_idx);
            addr_local = None;
            val_i32 = None;
            val_i64 = None;
            val_f32 = None;
            val_f64 = None;
            val_v128 = None;
            scratch_v128 = None;
            bulk_arg1 = None;
            bulk_arg2 = None;
        }

        if !at_end {
            if let Some(op) = it.curr_op_owned() {
                // --- Stores: duplicate to shadow memory ---
                if let (Some(vt), Some(shadow_store)) = (store_val_type(&op), to_shadow(&op)) {
                    let addr = get_or_add_local(&mut it, &mut addr_local, DataType::I32);
                    let val = get_val_local(
                        &mut it,
                        vt,
                        &mut val_i32,
                        &mut val_i64,
                        &mut val_f32,
                        &mut val_f64,
                        &mut val_v128,
                    );

                    it.before();
                    it.local_set(val);
                    it.local_tee(addr);
                    it.local_get(val);
                    it.finish_instr();

                    it.after();
                    it.local_get(addr);
                    it.local_get(val);
                    it.inject(shadow_store);
                    it.finish_instr();
                }
                // --- Simple Loads: compare real vs shadow, call record_memory_diff on divergence ---
                else if let (Some(vt), Some(shadow_load)) = (load_val_type(&op), to_shadow(&op))
                {
                    let byte_size = load_byte_size(&op);
                    let addr = get_or_add_local(&mut it, &mut addr_local, DataType::I32);
                    let val = get_val_local(
                        &mut it,
                        vt,
                        &mut val_i32,
                        &mut val_i64,
                        &mut val_f32,
                        &mut val_f64,
                        &mut val_v128,
                    );

                    it.before();
                    it.local_tee(addr);
                    it.finish_instr();

                    it.after();
                    it.local_set(val);
                    it.local_get(addr);
                    it.inject(shadow_load);
                    it.local_get(val);
                    emit_ne(&mut it, vt);
                    it.inject(Operator::If {
                        blockty: BlockType::Empty,
                    });
                    it.local_get(addr);
                    it.i32_const(byte_size);
                    it.call(record_memory_diff_id);
                    it.inject(Operator::End);
                    it.local_get(val);
                    it.finish_instr();
                }
                // --- V128 Lane Loads: [addr, v128] -> [v128] ---
                else if is_lane_load(&op) {
                    if let Some(shadow_lane_load) = to_shadow(&op) {
                        let byte_size = load_byte_size(&op);
                        let addr = get_or_add_local(&mut it, &mut addr_local, DataType::I32);
                        let input =
                            get_or_add_local(&mut it, &mut scratch_v128, DataType::V128);
                        let result = get_or_add_local(&mut it, &mut val_v128, DataType::V128);

                        it.before();
                        it.local_set(input);
                        it.local_tee(addr);
                        it.local_get(input);
                        it.finish_instr();

                        it.after();
                        it.local_set(result);
                        it.local_get(addr);
                        it.local_get(input);
                        it.inject(shadow_lane_load);
                        it.local_get(result);
                        it.inject(Operator::V128Xor);
                        it.inject(Operator::V128AnyTrue);
                        it.inject(Operator::If {
                            blockty: BlockType::Empty,
                        });
                        it.local_get(addr);
                        it.i32_const(byte_size);
                        it.call(record_memory_diff_id);
                        it.inject(Operator::End);
                        it.local_get(result);
                        it.finish_instr();
                    }
                }
                // --- memory.grow: grow shadow too ---
                else if matches!(&op, Operator::MemoryGrow { mem } if *mem == 0) {
                    let delta = get_or_add_local(&mut it, &mut addr_local, DataType::I32);
                    let saved_result =
                        get_or_add_local(&mut it, &mut bulk_arg1, DataType::I32);

                    it.before();
                    it.local_tee(delta);
                    it.finish_instr();

                    it.after();
                    it.local_set(saved_result);
                    it.local_get(delta);
                    it.inject(Operator::MemoryGrow { mem: 1 });
                    it.inject(Operator::Drop);
                    it.local_get(saved_result);
                    it.finish_instr();
                }
                // --- memory.copy: mirror to shadow ---
                else if matches!(&op, Operator::MemoryCopy { dst_mem, src_mem } if *dst_mem == 0 && *src_mem == 0)
                {
                    let dst = get_or_add_local(&mut it, &mut addr_local, DataType::I32);
                    let src = get_or_add_local(&mut it, &mut bulk_arg1, DataType::I32);
                    let len = get_or_add_local(&mut it, &mut bulk_arg2, DataType::I32);

                    it.before();
                    it.local_set(len);
                    it.local_set(src);
                    it.local_tee(dst);
                    it.local_get(src);
                    it.local_get(len);
                    it.finish_instr();

                    it.after();
                    it.local_get(dst);
                    it.local_get(src);
                    it.local_get(len);
                    it.inject(Operator::MemoryCopy {
                        dst_mem: 1,
                        src_mem: 1,
                    });
                    it.finish_instr();
                }
                // --- memory.init: mirror to shadow ---
                else if matches!(&op, Operator::MemoryInit { mem, .. } if *mem == 0) {
                    let data_index = match &op {
                        Operator::MemoryInit { data_index, .. } => *data_index,
                        _ => unreachable!(),
                    };
                    let dst = get_or_add_local(&mut it, &mut addr_local, DataType::I32);
                    let src_off = get_or_add_local(&mut it, &mut bulk_arg1, DataType::I32);
                    let len = get_or_add_local(&mut it, &mut bulk_arg2, DataType::I32);

                    it.before();
                    it.local_set(len);
                    it.local_set(src_off);
                    it.local_tee(dst);
                    it.local_get(src_off);
                    it.local_get(len);
                    it.finish_instr();

                    it.after();
                    it.local_get(dst);
                    it.local_get(src_off);
                    it.local_get(len);
                    it.inject(Operator::MemoryInit {
                        data_index,
                        mem: 1,
                    });
                    it.finish_instr();
                }
                // --- memory.fill: mirror to shadow ---
                else if matches!(&op, Operator::MemoryFill { mem } if *mem == 0) {
                    let dst = get_or_add_local(&mut it, &mut addr_local, DataType::I32);
                    let val = get_or_add_local(&mut it, &mut bulk_arg1, DataType::I32);
                    let len = get_or_add_local(&mut it, &mut bulk_arg2, DataType::I32);

                    it.before();
                    it.local_set(len);
                    it.local_set(val);
                    it.local_tee(dst);
                    it.local_get(val);
                    it.local_get(len);
                    it.finish_instr();

                    it.after();
                    it.local_get(dst);
                    it.local_get(val);
                    it.local_get(len);
                    it.inject(Operator::MemoryFill { mem: 1 });
                    it.finish_instr();
                }
                // --- Direct calls to original imports: record_import_call ---
                else if let Operator::Call { function_index } = &op {
                    if *function_index < num_orig_fn_imports {
                        it.before();
                        it.i32_const(*function_index as i32);
                        it.call(record_import_call_id);
                        it.finish_instr();
                    }
                }
            }
        }

        if it.next().is_none() {
            break;
        }
    }

    Ok(())
}
