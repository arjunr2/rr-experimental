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
    /// Contiguous run of differing bytes from real memory at the given address.
    MemoryWrite { addr: u32, data: Vec<u8> },
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
use wirm::ir::types::BlockType as WirmBlockType;
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

/// Instrument a core module with shadow memory and r3 recording.
///
/// When `component_mode` is false, imports `r3.record_memory_diff(addr, size)`
/// and the host reads memory to find diff bytes.
///
/// When `component_mode` is true, imports `r3.record_memory_write(addr, size, lo, hi)`
/// and a wasm-side helper function scans byte-by-byte for exact diff runs,
/// packs them into (lo, hi) i64s, and syncs the shadow memory itself.
///
/// Returns `Ok(true)` if instrumentation was applied, `Ok(false)` if skipped.
pub fn instrument_shadow(module: &mut Module, component_mode: bool) -> Result<bool> {
    if module.memories.is_empty() {
        return Ok(false);
    }
    // Skip modules where memory 0 is imported (e.g. WASI adapter modules).
    let has_memory_import = module
        .imports
        .iter()
        .any(|imp| matches!(imp.ty, TypeRef::Memory(_)));
    if has_memory_import {
        return Ok(false);
    }
    let num_local = module
        .functions
        .iter()
        .filter(|f| matches!(f.kind(), FuncKind::Local(_)))
        .count();
    if num_local == 0 {
        return Ok(false);
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
    let call_type = module.types.add_func_type(&[DataType::I32], &[]);

    // In component mode, import record_memory_write(i32, i32, i64, i64)
    // In core mode, import record_memory_diff(i32, i32) — host reads memory
    let record_memory_fn_id = if component_mode {
        let write_type = module.types.add_func_type(
            &[DataType::I32, DataType::I32, DataType::I64, DataType::I64],
            &[],
        );
        let (id, imp_id) = module.add_import_func(
            "r3".to_string(),
            "record_memory_write".to_string(),
            write_type,
        );
        module
            .imports
            .set_name("record_memory_write".to_string(), imp_id);
        id
    } else {
        let diff_type = module.types.add_func_type(&[DataType::I32, DataType::I32], &[]);
        let (id, imp_id) = module.add_import_func(
            "r3".to_string(),
            "record_memory_diff".to_string(),
            diff_type,
        );
        module
            .imports
            .set_name("record_memory_diff".to_string(), imp_id);
        id
    };

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
    // Phase 5b: Build __r3_scan_diff helper (component mode only)
    // ---------------------------------------------------------------
    // In component mode, this helper does byte-by-byte comparison between
    // mem0 and mem1, finds contiguous runs of differing bytes, packs them
    // into (lo: i64, hi: i64) scalars, calls record_memory_write, and
    // syncs the shadow memory.
    //
    // In core mode, record_memory_diff_id points to the host import and
    // scan_diff_id is not used (we use record_memory_fn_id directly).
    let scan_diff_id = if component_mode {
        Some(build_scan_diff_helper(module, record_memory_fn_id))
    } else {
        None
    };

    // The ID to call when a load mismatch is detected:
    // - component mode: call the wasm helper (2 params: addr, size)
    // - core mode: call the host import (2 params: addr, size)
    let on_mismatch_id = scan_diff_id.unwrap_or(record_memory_fn_id);

    // ---------------------------------------------------------------
    // Phase 6: Iterate all instructions (skip trampolines)
    // ---------------------------------------------------------------
    let mut skip: Vec<FunctionID> = trampoline_map.values().copied().collect();
    if let Some(id) = scan_diff_id {
        skip.push(id);
    }
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
                    it.call(on_mismatch_id);
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
                        it.call(on_mismatch_id);
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

    Ok(true)
}

/// Build the `__r3_scan_diff(addr: i32, size: i32)` helper function.
///
/// Scans byte-by-byte between mem0 and mem1 at `[addr..addr+size]`, finds
/// contiguous runs of differing bytes, packs each run into `(lo: i64, hi: i64)`,
/// calls `record_memory_write(run_addr, run_size, lo, hi)`, and syncs the
/// shadow memory for each differing byte.
fn build_scan_diff_helper(module: &mut Module, record_memory_write_id: FunctionID) -> FunctionID {
    let mem0 = MemArg {
        align: 0,
        offset: 0,
        memory: 0,
        max_align: 0,
    };
    let mem1 = MemArg {
        align: 0,
        offset: 0,
        memory: 1,
        max_align: 0,
    };

    // params: addr (i32), size (i32)
    let mut fb = FunctionBuilder::new(&[DataType::I32, DataType::I32], &[]);
    let p_addr = LocalID(0);
    let p_size = LocalID(1);

    let l_end = fb.add_local(DataType::I32); // addr + size
    let l_i = fb.add_local(DataType::I32); // current byte offset (absolute addr)
    let l_run_start = fb.add_local(DataType::I32); // start of current run (absolute addr)
    let l_byte_real = fb.add_local(DataType::I32);
    let l_byte_shadow = fb.add_local(DataType::I32);
    let l_lo = fb.add_local(DataType::I64);
    let l_hi = fb.add_local(DataType::I64);
    let l_offset = fb.add_local(DataType::I32); // byte offset within run
    let l_shift = fb.add_local(DataType::I64); // shift amount

    // end = addr + size
    fb.local_get(p_addr);
    fb.local_get(p_size);
    fb.i32_add();
    fb.local_set(l_end);

    // i = addr
    fb.local_get(p_addr);
    fb.local_set(l_i);

    // block $break { loop $scan {
    fb.block(WirmBlockType::Empty);
    fb.loop_stmt(WirmBlockType::Empty);

    // if i >= end: br $break
    fb.local_get(l_i);
    fb.local_get(l_end);
    fb.i32_gte_unsigned();
    fb.br_if(1); // break out of block

    // byte_real = mem0[i]
    fb.local_get(l_i);
    fb.i32_load8_u(mem0);
    fb.local_set(l_byte_real);

    // byte_shadow = mem1[i]
    fb.local_get(l_i);
    fb.i32_load8_u(mem1);
    fb.local_set(l_byte_shadow);

    // if byte_real == byte_shadow: i++, continue
    fb.local_get(l_byte_real);
    fb.local_get(l_byte_shadow);
    fb.i32_eq();
    fb.if_stmt(WirmBlockType::Empty);
    {
        fb.local_get(l_i);
        fb.i32_const(1);
        fb.i32_add();
        fb.local_set(l_i);
        fb.br(1); // continue loop $scan
    }
    fb.end(); // end if

    // Found a diff — start a run
    fb.local_get(l_i);
    fb.local_set(l_run_start);
    fb.i64_const(0);
    fb.local_set(l_lo);
    fb.i64_const(0);
    fb.local_set(l_hi);

    // block $run_break { loop $run_loop {
    fb.block(WirmBlockType::Empty);
    fb.loop_stmt(WirmBlockType::Empty);

    // if i >= end: br $run_break
    fb.local_get(l_i);
    fb.local_get(l_end);
    fb.i32_gte_unsigned();
    fb.br_if(1);

    // byte_real = mem0[i]
    fb.local_get(l_i);
    fb.i32_load8_u(mem0);
    fb.local_set(l_byte_real);

    // byte_shadow = mem1[i]
    fb.local_get(l_i);
    fb.i32_load8_u(mem1);
    fb.local_set(l_byte_shadow);

    // if byte_real == byte_shadow: br $run_break
    fb.local_get(l_byte_real);
    fb.local_get(l_byte_shadow);
    fb.i32_eq();
    fb.br_if(1);

    // offset = i - run_start
    fb.local_get(l_i);
    fb.local_get(l_run_start);
    fb.i32_sub();
    fb.local_set(l_offset);

    // shift = (offset % 8) * 8 as i64
    fb.local_get(l_offset);
    fb.i32_const(7);
    fb.i32_and();
    fb.i32_const(3);
    fb.i32_shl();
    fb.i64_extend_i32u();
    fb.local_set(l_shift);

    // if offset < 8: lo |= (byte_real as i64) << shift
    // else:          hi |= (byte_real as i64) << shift
    fb.local_get(l_offset);
    fb.i32_const(8);
    fb.i32_lt_unsigned();
    fb.if_stmt(WirmBlockType::Empty);
    {
        fb.local_get(l_lo);
        fb.local_get(l_byte_real);
        fb.i64_extend_i32u();
        fb.local_get(l_shift);
        fb.i64_shl();
        fb.i64_or();
        fb.local_set(l_lo);
    }
    fb.else_stmt();
    {
        fb.local_get(l_hi);
        fb.local_get(l_byte_real);
        fb.i64_extend_i32u();
        fb.local_get(l_shift);
        fb.i64_shl();
        fb.i64_or();
        fb.local_set(l_hi);
    }
    fb.end(); // end if

    // Sync this byte: mem1[i] = byte_real
    fb.local_get(l_i);
    fb.local_get(l_byte_real);
    fb.i32_store8(mem1);

    // i++
    fb.local_get(l_i);
    fb.i32_const(1);
    fb.i32_add();
    fb.local_set(l_i);

    fb.br(0); // continue $run_loop
    fb.end(); // end loop $run_loop
    fb.end(); // end block $run_break

    // Emit record_memory_write(run_start, i - run_start, lo, hi)
    fb.local_get(l_run_start);
    fb.local_get(l_i);
    fb.local_get(l_run_start);
    fb.i32_sub();
    fb.local_get(l_lo);
    fb.local_get(l_hi);
    fb.call(record_memory_write_id);

    fb.br(0); // continue $scan
    fb.end(); // end loop $scan
    fb.end(); // end block $break

    fb.finish_module(module)
}
