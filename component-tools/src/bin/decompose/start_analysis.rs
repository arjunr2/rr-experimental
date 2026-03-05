//! Static analysis to validate start functions in core modules.
//!
//! Relaxes the blanket "no start functions" restriction to allow a single
//! start function provided it does not touch any *true* imported state.
//! Imports that are renames (linked from another module's export) are allowed.
//! This is verified by walking the call graph from the start function.
//! `call_indirect` is conservatively rejected.

use anyhow::{Result, anyhow};
use std::collections::{HashMap, HashSet, VecDeque};
use wirm::Module;
use wirm::ir::id::FunctionID;
use wirm::wasmparser::{Operator, TypeRef};

use super::linking::{
    ImportKind, LinkingMetadata, ModuleID, ModuleImportIndex, assumed_instance_id,
};

/// Maps kind-specific indices (function index, global index, etc.) to their
/// `ModuleImportIndex` in the import section, but only for imports that are
/// NOT renames (i.e., true imports or builtins).
struct TrueImportIndices {
    funcs: HashSet<u32>,
    globals: HashSet<u32>,
    memories: HashSet<u32>,
    tables: HashSet<u32>,
    /// Number of imported functions (to distinguish import calls from local calls)
    num_import_funcs: u32,
}

impl TrueImportIndices {
    fn build(
        module: &Module,
        import_kinds: &HashMap<ModuleImportIndex, ImportKind>,
    ) -> Self {
        let mut funcs = HashSet::new();
        let mut globals = HashSet::new();
        let mut memories = HashSet::new();
        let mut tables = HashSet::new();

        let mut func_idx = 0u32;
        let mut global_idx = 0u32;
        let mut memory_idx = 0u32;
        let mut table_idx = 0u32;

        for (i, import) in module.imports.iter().enumerate() {
            let mii = ModuleImportIndex(i as u32);
            let is_rename = matches!(
                import_kinds.get(&mii),
                Some(ImportKind::Rename { .. })
            );

            match import.ty {
                TypeRef::Func(..) | TypeRef::FuncExact(..) => {
                    if !is_rename {
                        funcs.insert(func_idx);
                    }
                    func_idx += 1;
                }
                TypeRef::Global(..) => {
                    if !is_rename {
                        globals.insert(global_idx);
                    }
                    global_idx += 1;
                }
                TypeRef::Memory(..) => {
                    if !is_rename {
                        memories.insert(memory_idx);
                    }
                    memory_idx += 1;
                }
                TypeRef::Table(..) => {
                    if !is_rename {
                        tables.insert(table_idx);
                    }
                    table_idx += 1;
                }
                _ => {}
            }
        }

        Self {
            funcs,
            globals,
            memories,
            tables,
            num_import_funcs: func_idx,
        }
    }
}

/// Check a single operator for true-imported state access.
/// Returns `Ok(Some(fid))` if the op is a `Call` to a local function (to enqueue),
/// `Ok(None)` for safe ops (including calls to renamed imports), and `Err` for violations.
fn check_op(op: &Operator, idx: &TrueImportIndices) -> Result<Option<FunctionID>> {
    match op {
        Operator::Call { function_index } => {
            if *function_index < idx.num_import_funcs {
                // It's a call to an imported function
                if idx.funcs.contains(function_index) {
                    return Err(anyhow!(
                        "start function transitively calls true-imported function (index {})",
                        function_index
                    ));
                }
                // Rename — allowed, but don't follow into it (different module)
                return Ok(None);
            }
            Ok(Some(FunctionID(*function_index)))
        }

        Operator::CallIndirect { .. } => {
            Err(anyhow!("start function uses call_indirect"))
        }

        Operator::GlobalGet { global_index } | Operator::GlobalSet { global_index } => {
            if idx.globals.contains(global_index) {
                return Err(anyhow!(
                    "start function accesses true-imported global (index {})",
                    global_index
                ));
            }
            Ok(None)
        }

        // Memory operations with MemArg
        Operator::I32Load { memarg }
        | Operator::I64Load { memarg }
        | Operator::F32Load { memarg }
        | Operator::F64Load { memarg }
        | Operator::I32Load8S { memarg }
        | Operator::I32Load8U { memarg }
        | Operator::I32Load16S { memarg }
        | Operator::I32Load16U { memarg }
        | Operator::I64Load8S { memarg }
        | Operator::I64Load8U { memarg }
        | Operator::I64Load16S { memarg }
        | Operator::I64Load16U { memarg }
        | Operator::I64Load32S { memarg }
        | Operator::I64Load32U { memarg }
        | Operator::I32Store { memarg }
        | Operator::I64Store { memarg }
        | Operator::F32Store { memarg }
        | Operator::F64Store { memarg }
        | Operator::I32Store8 { memarg }
        | Operator::I32Store16 { memarg }
        | Operator::I64Store8 { memarg }
        | Operator::I64Store16 { memarg }
        | Operator::I64Store32 { memarg } => {
            if idx.memories.contains(&memarg.memory) {
                return Err(anyhow!(
                    "start function accesses true-imported memory (index {})",
                    memarg.memory
                ));
            }
            Ok(None)
        }

        Operator::MemorySize { mem } | Operator::MemoryGrow { mem } => {
            if idx.memories.contains(mem) {
                return Err(anyhow!(
                    "start function accesses true-imported memory (index {})",
                    mem
                ));
            }
            Ok(None)
        }

        Operator::MemoryFill { mem } => {
            if idx.memories.contains(mem) {
                return Err(anyhow!(
                    "start function accesses true-imported memory (index {})",
                    mem
                ));
            }
            Ok(None)
        }

        Operator::MemoryCopy { dst_mem, src_mem } => {
            if idx.memories.contains(dst_mem) || idx.memories.contains(src_mem) {
                return Err(anyhow!(
                    "start function accesses true-imported memory (copy between {} and {})",
                    src_mem,
                    dst_mem
                ));
            }
            Ok(None)
        }

        Operator::MemoryInit { mem, .. } => {
            if idx.memories.contains(mem) {
                return Err(anyhow!(
                    "start function accesses true-imported memory (index {})",
                    mem
                ));
            }
            Ok(None)
        }

        // Table operations
        Operator::TableGet { table }
        | Operator::TableSet { table }
        | Operator::TableGrow { table }
        | Operator::TableSize { table }
        | Operator::TableFill { table } => {
            if idx.tables.contains(table) {
                return Err(anyhow!(
                    "start function accesses true-imported table (index {})",
                    table
                ));
            }
            Ok(None)
        }

        Operator::TableCopy {
            dst_table,
            src_table,
        } => {
            if idx.tables.contains(dst_table) || idx.tables.contains(src_table) {
                return Err(anyhow!(
                    "start function accesses true-imported table (copy between {} and {})",
                    src_table,
                    dst_table
                ));
            }
            Ok(None)
        }

        Operator::TableInit { table, .. } => {
            if idx.tables.contains(table) {
                return Err(anyhow!(
                    "start function accesses true-imported table (index {})",
                    table
                ));
            }
            Ok(None)
        }

        // Everything else (arithmetic, control flow, locals, etc.) is safe.
        _ => Ok(None),
    }
}

/// Validates that start functions across all modules in the linking metadata are safe.
///
/// Rules:
/// - At most one start function across all modules
/// - The start function (and anything it calls transitively) must not
///   touch true-imported state (imports that are not renames from sister modules)
/// - `call_indirect` is conservatively rejected
pub fn validate_start_functions(linking: &LinkingMetadata) -> Result<()> {
    // Collect (ModuleID, Module ref) pairs that have start functions
    let with_start: Vec<(ModuleID, &Module)> = linking
        .mm
        .iter()
        .filter(|(_, meta)| meta.module.start.is_some())
        .map(|(mid, meta)| (*mid, &meta.module))
        .collect();

    if with_start.is_empty() {
        return Ok(());
    }
    if with_start.len() > 1 {
        return Err(anyhow!(
            "multiple core modules have start functions ({} found, at most 1 allowed)",
            with_start.len()
        ));
    }

    let (module_id, module) = with_start[0];
    let start_fid = module.start.unwrap();

    // Look up the instantiation metadata for this module's instance
    let instance_id = assumed_instance_id(&linking.instance_map, module_id);
    let inst_meta = linking
        .instantiations
        .get(&instance_id)
        .ok_or_else(|| anyhow!("no instantiation metadata for module with start function"))?;

    let true_imports = TrueImportIndices::build(module, &inst_meta.imports);

    let mut visited = HashSet::new();
    let mut worklist = VecDeque::new();
    worklist.push_back(start_fid);
    visited.insert(start_fid);

    while let Some(fid) = worklist.pop_front() {
        if module.functions.is_import(fid) {
            // Shouldn't happen — check_op should catch Call to imports before they
            // enter the worklist. But guard against it.
            return Err(anyhow!(
                "start function transitively reaches imported function (index {})",
                *fid
            ));
        }

        let local_fn = module.functions.get(fid).unwrap_local().map_err(|e| {
            anyhow!("expected local function at index {}: {}", *fid, e)
        })?;

        for op in local_fn.body.instructions.get_ops() {
            if let Some(callee) = check_op(op, &true_imports)? {
                if visited.insert(callee) {
                    worklist.push_back(callee);
                }
            }
        }
    }

    Ok(())
}
