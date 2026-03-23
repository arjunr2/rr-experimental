use escargot::format::Message;
use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use clap::Args;
use component_tools::wasmparser::{MemArg, MemoryType, Operator, RefType, TableType};
use component_tools::wirm::Module;
use component_tools::wirm::ir::function::FunctionBuilder;
use component_tools::wirm::ir::id::{FunctionID, LocalID, MemoryID, TypeID};
use component_tools::wirm::ir::module::module_tables::{Element, ModuleTables, Table};
use component_tools::wirm::ir::module::module_types::Types;
use component_tools::wirm::ir::types::{
    BlockType, ElementItems, ElementKind, InitExpr, InitInstr, Value,
};
use component_tools::wirm::module_builder::AddLocal;
use component_tools::wirm::opcode::{Inject, Opcode};

use crate::linking::{
    BuiltinOptions, Checksum, ExportFuncMetadata, ImportAdapterCrimpData, LinkingMetadata,
    ModuleInstanceExport, ModuleInstanceID, module_name_from_ids,
};

pub const GLUE_MODULE_NAME: &str = "crimp_glue";
pub const DRIVER_MODULE_NAME: &str = "crimp_driver_mono";
pub const DECOMPOSED_COMPONENT_NAME: &str = "decomposed_component";

use component_tools::wirm::DataType;

#[derive(Debug, Default)]
pub struct DriverGlueModules<'a> {
    pub driver: Module<'a>,
    pub glue: Module<'a>,
}

impl<'a> DriverGlueModules<'a> {
    /// Build the crimp-driver targeting wasm32-wasip1 with the given trace path,
    /// parse the resulting .wasm into a Module, and finalize the glue module from the builder.
    pub fn from_path_and_builder(trace_path: PathBuf, builder: GlueBuilder<'a>) -> Result<Self> {
        let driver_manifest = PathBuf::from(env!("CRIMP_DRIVER_MONO_MANIFEST"));
        let trace_path = trace_path
            .canonicalize()
            .map_err(|e| anyhow!("Failed to canonicalize trace path: {}", e))?;

        log::info!(
            "Building crimp-glue-driver with TRACE_FILEPATH={:?}, manifest={:?}",
            trace_path,
            driver_manifest
        );

        let messages = escargot::CargoBuild::new()
            .manifest_path(&driver_manifest)
            .target("wasm32-wasip1")
            .env("TRACE_FILEPATH", &trace_path)
            .release()
            .exec()
            .map_err(|e| anyhow!("Failed to run cargo build: {}", e))?;

        let mut wasm_path: Option<PathBuf> = None;
        for msg_result in messages {
            let msg =
                msg_result.map_err(|e| anyhow!("Error reading cargo build message: {}", e))?;
            let decoded = msg
                .decode()
                .map_err(|e| anyhow!("Error decoding cargo build message: {}", e))?;

            if let Message::CompilerMessage(msg) = &decoded {
                if let Some(rendered) = &msg.message.rendered {
                    log::warn!("{}", rendered);
                }
            }
            if let Message::CompilerArtifact(artifact) = decoded {
                if artifact
                    .target
                    .crate_types
                    .iter()
                    .any(|ct| ct.as_ref() == "cdylib")
                {
                    for filename in &artifact.filenames {
                        if filename.extension().is_some_and(|ext| ext == "wasm") {
                            wasm_path = Some(filename.to_path_buf());
                            break;
                        }
                    }
                }
            }
        }

        let wasm_path = wasm_path
            .ok_or_else(|| anyhow!("Failed to find .wasm artifact from crimp-glue-driver build"))?;

        log::info!("Driver built at: {:?}", wasm_path);
        let bytes = std::fs::read(&wasm_path)?;
        // Leak the bytes so the parsed Module can borrow with 'static lifetime.
        // This is acceptable since the `decompose` is a short-lived CLI tool.
        let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        let mut driver = Module::parse(bytes, true, false)
            .map_err(|e| anyhow!("Failed to parse driver wasm: {:?}", e))?;
        driver.module_name = Some(DRIVER_MODULE_NAME.to_string());

        Ok(Self {
            driver,
            glue: builder.finish(),
        })
    }
}

#[derive(Debug, Args)]
pub struct GlueArgs {
    /// Only valid with `glue` is true - the path to the trace file to be embedded in the replay driver
    /// module for use during replay
    #[arg(short = 'p', long = "trace-path")]
    pub trace_path: Option<PathBuf>,
}

/// Builder for the synthetic glue Wasm module.
///
/// The glue module bridges decomposed component modules with the replay driver:
/// - Exports stub functions for component "crimp-replay" imports (calling driver's replay functions)
/// - Exports the "crimp_glue" dispatch interface for the driver (checksum, realloc, memory_write, core_func)
pub struct GlueBuilder<'a> {
    module: Module<'a>,
    checksum: Checksum,

    // Driver imports
    driver_memory: MemoryID,
    replay_host_call: FunctionID,
    replay_builtin_call: FunctionID,
    replay_instruction: FunctionID,
    init_replayer: FunctionID,
    allocate_args_results_buffer: FunctionID,

    // Dedup caches for component imports
    imported_memories: HashMap<(ModuleInstanceID, String), MemoryID>,
    imported_funcs: HashMap<(ModuleInstanceID, String), (FunctionID, TypeID)>,

    // Global import counter (unique across all modules)
    next_import_id: u32,

    // Dispatch tables split by direction (populated per-import/per-export, consumed in finish)
    // Import: indexed by unified import_id; Export: indexed by record_id
    realloc_import_dispatch: Vec<(u32, FunctionID)>,
    realloc_export_dispatch: Vec<(u32, FunctionID)>,
    memwrite_import_dispatch: Vec<(u32, MemoryID)>,
    memwrite_export_dispatch: Vec<(u32, MemoryID)>,
    core_func_dispatch: Vec<(u32, FunctionID, TypeID)>, // export only
    post_return_dispatch: Vec<(u32, FunctionID, TypeID)>, // export only

    // Accumulated tables and elements for the module (set in finish)
    pending_tables: Vec<Table<'a>>,
    pending_elements: Vec<Element>,
}

impl<'a> GlueBuilder<'a> {
    pub fn new(checksum: Checksum) -> Self {
        let mut module = Module::default();
        module.module_name = Some(GLUE_MODULE_NAME.to_string());

        // Import driver memory
        let (driver_memory, _) = module.add_import_memory(
            DRIVER_MODULE_NAME.to_string(),
            "memory".to_string(),
            MemoryType {
                initial: 0,
                maximum: None,
                shared: false,
                memory64: false,
                page_size_log2: None,
            },
        );

        // Import replay functions from driver
        // replay_host_call: (import_index, params_bytes_len, params_sizes_len) -> i32
        let replay_host_call_type = module.types.add_func_type(
            &[DataType::I32, DataType::I32, DataType::I32],
            &[DataType::I32],
        );
        let (replay_host_call, _) = module.add_import_func(
            DRIVER_MODULE_NAME.to_string(),
            "replay_host_call".to_string(),
            replay_host_call_type,
        );
        // replay_builtin_call: (import_index, params_bytes_len, params_sizes_len) -> i32
        let replay_builtin_type = module.types.add_func_type(
            &[DataType::I32, DataType::I32, DataType::I32],
            &[DataType::I32],
        );
        let (replay_builtin_call, _) = module.add_import_func(
            DRIVER_MODULE_NAME.to_string(),
            "replay_builtin_call".to_string(),
            replay_builtin_type,
        );
        // replay_instruction: (result: i32) -> i32 (returns recorded value)
        let replay_instruction_type =
            module.types.add_func_type(&[DataType::I32], &[DataType::I32]);
        let (replay_instruction, _) = module.add_import_func(
            DRIVER_MODULE_NAME.to_string(),
            "replay_instruction".to_string(),
            replay_instruction_type,
        );
        // init_replayer: () -> ()
        let init_replayer_type = module.types.add_func_type(&[], &[]);
        let (init_replayer, _) = module.add_import_func(
            DRIVER_MODULE_NAME.to_string(),
            "init_replayer".to_string(),
            init_replayer_type,
        );

        // Import allocate_args_results_buffer from driver: (i32, i32) -> i32 (returns pointer to RRFuncArgValsFFI)
        let allocate_args_results_buffer_type = module
            .types
            .add_func_type(&[DataType::I32, DataType::I32], &[DataType::I32]);
        let (allocate_args_results_buffer, _) = module.add_import_func(
            DRIVER_MODULE_NAME.to_string(),
            "allocate_args_results_buffer".to_string(),
            allocate_args_results_buffer_type,
        );

        Self {
            module,
            checksum,
            driver_memory,
            replay_host_call,
            replay_builtin_call,
            replay_instruction,
            init_replayer,
            allocate_args_results_buffer,
            next_import_id: 0,
            imported_memories: HashMap::new(),
            imported_funcs: HashMap::new(),
            realloc_import_dispatch: Vec::new(),
            realloc_export_dispatch: Vec::new(),
            memwrite_import_dispatch: Vec::new(),
            memwrite_export_dispatch: Vec::new(),
            core_func_dispatch: Vec::new(),
            post_return_dispatch: Vec::new(),
            pending_tables: Vec::new(),
            pending_elements: Vec::new(),
        }
    }

    // ========================
    // ==== Import Helpers ====
    // ========================

    /// Import a component memory if not already imported. Returns the MemoryID in the glue module.
    fn ensure_memory_imported(
        &mut self,
        export: &ModuleInstanceExport,
        instance_name: &str,
    ) -> MemoryID {
        let key = (export.mid, export.name.clone());
        if let Some(&mem_id) = self.imported_memories.get(&key) {
            return mem_id;
        }
        let (mem_id, _) = self.module.add_import_memory(
            instance_name.to_string(),
            export.name.clone(),
            MemoryType {
                initial: 0,
                maximum: None,
                shared: false,
                memory64: false,
                page_size_log2: None,
            },
        );
        self.imported_memories.insert(key, mem_id);
        mem_id
    }

    /// Import a component function if not already imported. Returns (FunctionID, TypeID) in the glue module.
    fn ensure_func_imported(
        &mut self,
        instance_id: ModuleInstanceID,
        func_name: &str,
        instance_name: &str,
        params: &[DataType],
        results: &[DataType],
    ) -> (FunctionID, TypeID) {
        let key = (instance_id, func_name.to_string());
        if let Some(&ids) = self.imported_funcs.get(&key) {
            return ids;
        }
        let type_id = self.module.types.add_func_type(params, results);
        let (func_id, _) =
            self.module
                .add_import_func(instance_name.to_string(), func_name.to_string(), type_id);
        self.imported_funcs.insert(key, (func_id, type_id));
        (func_id, type_id)
    }

    // ==============================
    // ==== Replay Stub Builders ====
    // ==============================

    /// Add a replay stub function that the component module will call for a replaced import.
    ///
    /// The stub:
    /// 1. Allocates an FFI buffer via `allocate_args_results_buffer` and stores the params into it
    /// 2. Calls `replay_host_call` or `replay_builtin_call` on the driver
    /// 3. Reads return values from the returned pointer in driver memory
    pub fn add_replay_stub(
        &mut self,
        export_name: &str,
        params: &[DataType],
        results: &[DataType],
        adapter: &ImportAdapterCrimpData,
        linking: &LinkingMetadata,
    ) {
        let import_id = self.next_import_id;
        self.next_import_id += 1;

        // Register realloc/memory dispatch entries for this import (direction=0)
        if let Some(realloc) = &adapter.realloc {
            let realloc_instance_name =
                module_name_from_ids(linking.module_id(realloc.mid), realloc.mid);
            let (realloc_func_id, _) = self.ensure_func_imported(
                realloc.mid,
                &realloc.name,
                &realloc_instance_name,
                &[DataType::I32, DataType::I32, DataType::I32, DataType::I32],
                &[DataType::I32],
            );
            self.realloc_import_dispatch
                .push((import_id, realloc_func_id));
        }
        if let Some(memory) = &adapter.memory {
            let memory_instance_name =
                module_name_from_ids(linking.module_id(memory.mid), memory.mid);
            let mem_id = self.ensure_memory_imported(memory, &memory_instance_name);
            self.memwrite_import_dispatch.push((import_id, mem_id));
        }
        let mut fb = FunctionBuilder::new(params, results);
        fb.set_name(format!("stub_{}", export_name));
        let driver_mem = *self.driver_memory;

        let replay_func = if adapter.builtin.is_some() {
            self.replay_builtin_call
        } else {
            self.replay_host_call
        };

        // Save params to locals so we can store them to memory
        let param_locals: Vec<(LocalID, DataType)> = params
            .iter()
            .enumerate()
            .map(|(i, ty)| (LocalID(i as u32), *ty))
            .collect();

        let total_param_bytes: u32 = params.iter().map(|ty| data_type_byte_size(ty)).sum();
        let num_params: u32 = params.len() as u32;
        let (_ffi_ptr, bytes_ptr, sizes_ptr) = emit_alloc_ffi_buffer(
            &mut fb,
            total_param_bytes,
            num_params,
            self.allocate_args_results_buffer,
            driver_mem,
        );

        // Store each param into bytes_ptr (driver memory)
        let mut byte_offset: u64 = 0;
        for (local_id, param_ty) in &param_locals {
            fb.local_get(bytes_ptr);
            fb.local_get(*local_id);
            byte_offset = emit_typed_store(&mut fb, param_ty, driver_mem, byte_offset);
        }

        // Store size descriptors into sizes_ptr (1 byte each)
        emit_store_size_descriptors(&mut fb, params, sizes_ptr, driver_mem);

        // Call the replay function with (import_index, params_bytes_len, params_sizes_len)
        fb.i32_const(import_id as i32);
        fb.i32_const(total_param_bytes as i32);
        fb.i32_const(num_params as i32);
        fb.call(replay_func);

        // Save the result pointer (always returned by replay_host_call/replay_builtin_call)
        let result_ptr = fb.add_local(DataType::I32);
        fb.local_set(result_ptr);

        // Handle builtin side-effects (e.g. resource destructors)
        if let Some(builtin) = &adapter.builtin {
            match builtin {
                BuiltinOptions::NoSideEffects => {
                    // No side-effects, do nothing
                }
                BuiltinOptions::ResourceDrop {
                    host_dtor,
                    guest_dtor,
                } => {
                    if *host_dtor || guest_dtor.is_some() {
                        // The replay_builtin_call result buffer layout:
                        //   offset 0: i32 — resource rep to pass to the destructor
                        //   offset 4: u8  — some/none flag (non-zero = call dtor)
                        let dtor_rep = fb.add_local(DataType::I32);

                        // Load rep (i32) from offset 0
                        fb.local_get(result_ptr);
                        fb.i32_load(MemArg {
                            align: 2,
                            max_align: 2,
                            offset: 0,
                            memory: driver_mem,
                        });
                        fb.local_set(dtor_rep);

                        // Load some/none flag (u8) from offset 4
                        fb.local_get(result_ptr);
                        fb.inject(Operator::I32Load8U {
                            memarg: MemArg {
                                align: 0,
                                max_align: 0,
                                offset: 4,
                                memory: driver_mem,
                            },
                        });

                        // Conditionally execute destructors if flag is set
                        fb.if_stmt(BlockType::Empty);
                        {
                            if *host_dtor {
                                // Host destructor: replay the host dtor call with [dtor_rep] -> []
                                let dtor_import_id = self.next_import_id;
                                self.next_import_id += 1;

                                let (_dtor_ffi_ptr, dtor_bytes_ptr, dtor_sizes_ptr) =
                                    emit_alloc_ffi_buffer(
                                        &mut fb,
                                        4, // one i32
                                        1,
                                        self.allocate_args_results_buffer,
                                        driver_mem,
                                    );

                                // Store the resource rep into the dtor buffer
                                fb.local_get(dtor_bytes_ptr);
                                fb.local_get(dtor_rep);
                                fb.i32_store(MemArg {
                                    align: 2,
                                    max_align: 2,
                                    offset: 0,
                                    memory: driver_mem,
                                });

                                // Store size descriptor (4 bytes for i32)
                                emit_store_size_descriptors(
                                    &mut fb,
                                    &[DataType::I32],
                                    dtor_sizes_ptr,
                                    driver_mem,
                                );

                                // Call replay_host_call for the host destructor
                                fb.i32_const(dtor_import_id as i32);
                                fb.i32_const(4); // params_bytes_len
                                fb.i32_const(1); // params_sizes_len
                                fb.call(self.replay_host_call);
                                fb.drop(); // dtor returns [], drop the result pointer
                            }

                            if let Some(guest_dtor_export) = guest_dtor {
                                // Guest destructor: import and call the guest dtor function
                                let dtor_instance_name = module_name_from_ids(
                                    linking.module_id(guest_dtor_export.mid),
                                    guest_dtor_export.mid,
                                );
                                let (dtor_func_id, _) = self.ensure_func_imported(
                                    guest_dtor_export.mid,
                                    &guest_dtor_export.name,
                                    &dtor_instance_name,
                                    &[DataType::I32],
                                    &[],
                                );
                                fb.local_get(dtor_rep);
                                fb.call(dtor_func_id);
                            }
                        }
                        fb.end();
                    }
                }
            }
        }

        if !results.is_empty() {
            // Load each result from driver memory at result_ptr + offset
            let mut offset: u64 = 0;
            for result_ty in results {
                fb.local_get(result_ptr);
                offset = emit_typed_load(&mut fb, result_ty, driver_mem, offset);
            }
        }

        let func_id = fb.finish_module(&mut self.module);
        self.module
            .exports
            .add_export_func(export_name.to_string(), *func_id);
    }

    // ====================================
    // ==== Export Registration (dispatch) ====
    // ====================================

    /// Register a component export function for dispatch from the driver.
    ///
    /// This imports the core function (and its realloc/memory if present) into the glue module
    /// and populates the dispatch tables used by the `dispatch_*` functions.
    pub fn register_export(
        &mut self,
        export_func: &ExportFuncMetadata,
        instance_id: ModuleInstanceID,
        linking: &LinkingMetadata,
    ) -> Result<()> {
        let module_id = linking.module_id(instance_id);
        let instance_name = module_name_from_ids(module_id, instance_id);
        let src_module = linking.module(instance_id);

        // Look up the core function's type from the source module
        let core_export = src_module
            .exports
            .get_by_name(export_func.name.clone())
            .ok_or_else(|| {
                anyhow!(
                    "Export '{}' not found in module for instance {:?}",
                    export_func.name,
                    instance_id
                )
            })?;
        let core_func_type_id = src_module
            .functions
            .get_type_id(FunctionID(core_export.index));
        let (params, results) =
            get_func_type_params_results(src_module.types.get(core_func_type_id).unwrap());

        // Import the core function into the glue module
        let (glue_func_id, glue_type_id) = self.ensure_func_imported(
            instance_id,
            &export_func.name,
            &instance_name,
            &params,
            &results,
        );
        self.core_func_dispatch
            .push((export_func.record_id.0, glue_func_id, glue_type_id));

        // Import realloc if present in canonical options
        let opts = &export_func.opts;
        if let Some(realloc) = &opts.realloc {
            let realloc_instance_name =
                module_name_from_ids(linking.module_id(realloc.mid), realloc.mid);
            let (realloc_func_id, _) = self.ensure_func_imported(
                realloc.mid,
                &realloc.name,
                &realloc_instance_name,
                &[DataType::I32, DataType::I32, DataType::I32, DataType::I32],
                &[DataType::I32],
            );
            self.realloc_export_dispatch
                .push((export_func.record_id.0, realloc_func_id));
        }
        if let Some(memory) = &opts.memory {
            let memory_instance_name =
                module_name_from_ids(linking.module_id(memory.mid), memory.mid);
            let mem_id = self.ensure_memory_imported(memory, &memory_instance_name);
            self.memwrite_export_dispatch
                .push((export_func.record_id.0, mem_id));
        }
        if let Some(post_return) = &opts.post_return {
            let pr_instance_name =
                module_name_from_ids(linking.module_id(post_return.mid), post_return.mid);
            let pr_module = linking.module(post_return.mid);
            let pr_export = pr_module
                .exports
                .get_by_name(post_return.name.clone())
                .ok_or_else(|| {
                    anyhow!(
                        "Post-return '{}' not found in module for instance {:?}",
                        post_return.name,
                        post_return.mid
                    )
                })?;
            let pr_type_id = pr_module.functions.get_type_id(FunctionID(pr_export.index));
            let (pr_params, pr_results) =
                get_func_type_params_results(pr_module.types.get(pr_type_id).unwrap());
            let (pr_func_id, pr_glue_type_id) = self.ensure_func_imported(
                post_return.mid,
                &post_return.name,
                &pr_instance_name,
                &pr_params,
                &pr_results,
            );
            self.post_return_dispatch
                .push((export_func.record_id.0, pr_func_id, pr_glue_type_id));
        }

        Ok(())
    }

    // ====================================
    // ==== Table Helpers ====
    // ====================================

    /// Create a funcref table of the given size and add an active element segment
    /// populating it with the given function IDs at their respective indices.
    /// `entries` is a list of (index, FunctionID). Unspecified slots remain null.
    /// Returns the TableID (index into pending_tables).
    fn add_dispatch_table(&mut self, size: u32, entries: &[(u32, FunctionID)]) -> u32 {
        let table_index = self.pending_tables.len() as u32;
        self.pending_tables.push(Table::new(
            TableType {
                initial: size as u64,
                maximum: Some(size as u64),
                element_type: RefType::FUNCREF,
                shared: false,
                table64: false,
            },
            None,
            None,
        ));
        if !entries.is_empty() {
            // Create one active element segment per entry (each at its own offset)
            // We could batch contiguous entries, but individual segments are simpler
            // and the module is generated once.
            for &(idx, func_id) in entries {
                self.pending_elements.push(Element::new(
                    ElementKind::Active {
                        table_index: Some(table_index),
                        offset_expr: InitExpr::new(vec![InitInstr::Value(Value::I32(idx as i32))]),
                    },
                    ElementItems::Functions(vec![func_id]),
                    None,
                ));
            }
        }
        table_index
    }

    /// Create a pair of dispatch tables (import-side indexed by `next_import_id`,
    /// export-side indexed by max record_id + 1). Returns `(import_table_idx, export_table_idx)`.
    fn add_import_export_table_pair(
        &mut self,
        import_entries: &[(u32, FunctionID)],
        export_entries: &[(u32, FunctionID)],
    ) -> (u32, u32) {
        let import_table_size = self.next_import_id;
        let import_table_idx = self.add_dispatch_table(import_table_size, import_entries);

        let export_table_size = export_entries
            .iter()
            .map(|(id, _)| *id + 1)
            .max()
            .unwrap_or(0);
        let export_table_idx = self.add_dispatch_table(export_table_size, export_entries);

        (import_table_idx, export_table_idx)
    }

    // ====================================
    // ==== Dispatch Function Builders ====
    // ====================================

    /// Build `get_sha256_checksum(checksum_buf: i32)`.
    /// Stores the embedded 32-byte checksum into driver memory at the given pointer.
    fn build_get_sha256_checksum(&mut self) {
        let mut fb = FunctionBuilder::new(&[DataType::I32], &[]);
        fb.set_name("get_sha256_checksum".to_string());
        let buf_ptr = LocalID(0);
        let driver_mem = *self.driver_memory;

        // Write checksum as 4 x i64 stores
        for i in 0..4 {
            let byte_offset = i * 8;
            let qword = u64::from_le_bytes([
                self.checksum[byte_offset],
                self.checksum[byte_offset + 1],
                self.checksum[byte_offset + 2],
                self.checksum[byte_offset + 3],
                self.checksum[byte_offset + 4],
                self.checksum[byte_offset + 5],
                self.checksum[byte_offset + 6],
                self.checksum[byte_offset + 7],
            ]);
            fb.local_get(buf_ptr);
            fb.i64_const(qword as i64);
            fb.i64_store(MemArg {
                align: 3,
                max_align: 3,
                offset: byte_offset as u64,
                memory: driver_mem,
            });
        }

        let func_id = fb.finish_module(&mut self.module);
        self.module
            .exports
            .add_export_func("get_sha256_checksum".to_string(), *func_id);
    }

    /// Build `dispatch_realloc(direction, index, old_addr, old_size, old_align, new_size) -> i32`.
    /// Dispatches to the appropriate imported realloc via table-based `call_indirect`.
    fn build_dispatch_realloc(&mut self) {
        // All reallocs share the same type: (i32, i32, i32, i32) -> i32
        let realloc_type = self.module.types.add_func_type(
            &[DataType::I32, DataType::I32, DataType::I32, DataType::I32],
            &[DataType::I32],
        );

        let import_entries: Vec<(u32, FunctionID)> = self.realloc_import_dispatch.clone();
        let export_entries: Vec<(u32, FunctionID)> = self.realloc_export_dispatch.clone();
        let (import_table_idx, export_table_idx) =
            self.add_import_export_table_pair(&import_entries, &export_entries);

        let mut fb = FunctionBuilder::new(
            &[
                DataType::I32,
                DataType::I32,
                DataType::I32,
                DataType::I32,
                DataType::I32,
                DataType::I32,
            ],
            &[DataType::I32],
        );
        fb.set_name("dispatch_realloc".to_string());
        let direction = LocalID(0);
        let index = LocalID(1);
        let old_addr = LocalID(2);
        let old_size = LocalID(3);
        let old_align = LocalID(4);
        let new_size = LocalID(5);

        emit_direction_dispatch(
            &mut fb,
            direction,
            index,
            &[old_addr, old_size, old_align, new_size],
            realloc_type,
            import_table_idx,
            export_table_idx,
            BlockType::Type(DataType::I32),
        );

        let func_id = fb.finish_module(&mut self.module);
        self.module
            .exports
            .add_export_func("dispatch_realloc".to_string(), *func_id);
    }

    /// Build `dispatch_memory_write(direction, index, offset, bytes_ptr, num_bytes)`.
    /// Copies bytes from driver memory to the appropriate component memory via table dispatch.
    /// Each unique memory gets a wrapper function `(offset, bytes_ptr, num_bytes) -> ()` that
    /// does `memory.copy(target_mem, driver_mem)`.
    fn build_dispatch_memory_write(&mut self) {
        let driver_mem = *self.driver_memory;

        // Wrapper type: (i32, i32, i32) -> ()
        let wrapper_type = self
            .module
            .types
            .add_func_type(&[DataType::I32, DataType::I32, DataType::I32], &[]);

        // Create a wrapper function per unique MemoryID
        let mut mem_to_wrapper: HashMap<u32, FunctionID> = HashMap::new();
        let all_mems: Vec<MemoryID> = self
            .memwrite_import_dispatch
            .iter()
            .map(|(_, m)| *m)
            .chain(self.memwrite_export_dispatch.iter().map(|(_, m)| *m))
            .collect();
        for target_mem_id in &all_mems {
            if mem_to_wrapper.contains_key(&**target_mem_id) {
                continue;
            }
            let mut wfb = FunctionBuilder::new(&[DataType::I32, DataType::I32, DataType::I32], &[]);
            wfb.set_name(format!("memwrite_wrapper_{}", **target_mem_id));
            let w_offset = LocalID(0);
            let w_bytes_ptr = LocalID(1);
            let w_num_bytes = LocalID(2);
            wfb.local_get(w_offset);
            wfb.local_get(w_bytes_ptr);
            wfb.local_get(w_num_bytes);
            wfb.memory_copy(**target_mem_id, driver_mem);
            let wrapper_id = wfb.finish_module(&mut self.module);
            mem_to_wrapper.insert(**target_mem_id, wrapper_id);
        }

        let import_entries: Vec<(u32, FunctionID)> = self
            .memwrite_import_dispatch
            .iter()
            .map(|(idx, mem)| (*idx, mem_to_wrapper[&**mem]))
            .collect();
        let export_entries: Vec<(u32, FunctionID)> = self
            .memwrite_export_dispatch
            .iter()
            .map(|(idx, mem)| (*idx, mem_to_wrapper[&**mem]))
            .collect();
        let (import_table_idx, export_table_idx) =
            self.add_import_export_table_pair(&import_entries, &export_entries);

        let mut fb = FunctionBuilder::new(
            &[
                DataType::I32,
                DataType::I32,
                DataType::I32,
                DataType::I32,
                DataType::I32,
            ],
            &[],
        );
        fb.set_name("dispatch_memory_write".to_string());
        let direction = LocalID(0);
        let index = LocalID(1);
        let offset = LocalID(2);
        let bytes_ptr = LocalID(3);
        let num_bytes = LocalID(4);

        emit_direction_dispatch(
            &mut fb,
            direction,
            index,
            &[offset, bytes_ptr, num_bytes],
            wrapper_type,
            import_table_idx,
            export_table_idx,
            BlockType::Empty,
        );

        let func_id = fb.finish_module(&mut self.module);
        self.module
            .exports
            .add_export_func("dispatch_memory_write".to_string(), *func_id);
    }

    /// Build `dispatch_core_func(export_index, args_ptr, return_bytes_len, return_sizes_len)`.
    /// For each export, creates a wrapper function with uniform type `(args_ptr, return_bytes_len,
    /// return_sizes_len) -> ()` that contains the per-export marshalling logic.
    /// Uses a table + call_indirect for O(1) dispatch.
    fn build_dispatch_core_func(&mut self) {
        let driver_mem = *self.driver_memory;
        let allocate_fn = self.allocate_args_results_buffer;

        // Uniform wrapper type: (i32, i32, i32) -> ()
        let wrapper_type = self
            .module
            .types
            .add_func_type(&[DataType::I32, DataType::I32, DataType::I32], &[]);

        // Create a wrapper function per export
        let entries = self.core_func_dispatch.clone();
        let mut table_entries: Vec<(u32, FunctionID)> = Vec::new();

        for (record_id, core_func_id, type_id) in &entries {
            let func_type = self.module.types.get(*type_id).unwrap().clone();
            let (params, results) = get_func_type_params_results(&func_type);
            let total_bytes: u32 = results.iter().map(|r| data_type_byte_size(r)).sum();
            let num_results: u32 = results.len() as u32;

            let mut wfb = FunctionBuilder::new(&[DataType::I32, DataType::I32, DataType::I32], &[]);
            wfb.set_name(format!("core_func_wrapper_{}", record_id));
            let w_args_ptr = LocalID(0);
            let w_return_bytes_len = LocalID(1);
            let w_return_sizes_len = LocalID(2);

            // Load each argument from args_ptr (driver memory)
            let mut arg_offset: u64 = 0;
            for param_ty in &params {
                wfb.local_get(w_args_ptr);
                arg_offset = emit_typed_load(&mut wfb, param_ty, driver_mem, arg_offset);
            }

            // Call the core function
            wfb.call(*core_func_id);

            // Save results to locals and store into FFI buffer
            if !results.is_empty() {
                let result_locals: Vec<LocalID> =
                    results.iter().map(|ty| wfb.add_local(*ty)).collect();
                for local in result_locals.iter().rev() {
                    wfb.local_set(*local);
                }

                let (_ffi_ptr, w_bytes_ptr, w_sizes_ptr) = emit_alloc_ffi_buffer(
                    &mut wfb,
                    total_bytes,
                    num_results,
                    allocate_fn,
                    driver_mem,
                );

                // Store each result value into bytes_ptr
                let mut byte_offset: u64 = 0;
                for (i, result_ty) in results.iter().enumerate() {
                    wfb.local_get(w_bytes_ptr);
                    wfb.local_get(result_locals[i]);
                    byte_offset = emit_typed_store(&mut wfb, result_ty, driver_mem, byte_offset);
                }

                // Store size descriptors into sizes_ptr (1 byte each)
                emit_store_size_descriptors(&mut wfb, &results, w_sizes_ptr, driver_mem);
            }

            // Write return lengths to out-pointers
            wfb.local_get(w_return_bytes_len);
            wfb.i32_const(total_bytes as i32);
            wfb.i32_store(MemArg {
                align: 2,
                max_align: 2,
                offset: 0,
                memory: driver_mem,
            });

            wfb.local_get(w_return_sizes_len);
            wfb.i32_const(num_results as i32);
            wfb.i32_store(MemArg {
                align: 2,
                max_align: 2,
                offset: 0,
                memory: driver_mem,
            });

            let wrapper_id = wfb.finish_module(&mut self.module);
            table_entries.push((*record_id, wrapper_id));
        }

        // Create the dispatch table
        let table_size = table_entries
            .iter()
            .map(|(id, _)| *id + 1)
            .max()
            .unwrap_or(0);
        let table_idx = self.add_dispatch_table(table_size, &table_entries);

        // Build the dispatch function: just forwards to call_indirect
        let mut fb = FunctionBuilder::new(
            &[DataType::I32, DataType::I32, DataType::I32, DataType::I32],
            &[],
        );
        fb.set_name("dispatch_core_func".to_string());
        let export_index = LocalID(0);
        let args_ptr = LocalID(1);
        let return_bytes_len = LocalID(2);
        let return_sizes_len = LocalID(3);

        fb.local_get(args_ptr);
        fb.local_get(return_bytes_len);
        fb.local_get(return_sizes_len);
        fb.local_get(export_index);
        fb.inject(Operator::CallIndirect {
            type_index: *wrapper_type,
            table_index: table_idx,
        });

        let func_id = fb.finish_module(&mut self.module);
        self.module
            .exports
            .add_export_func("dispatch_core_func".to_string(), *func_id);
    }

    /// Build `dispatch_post_return(export_index, args_ptr)`.
    /// For each export with a post_return, creates a wrapper `(args_ptr) -> ()`.
    /// All table slots default to a no-op function; registered slots are overwritten.
    fn build_dispatch_post_return(&mut self) {
        let driver_mem = *self.driver_memory;

        // Uniform wrapper type: (i32) -> ()
        let wrapper_type = self.module.types.add_func_type(&[DataType::I32], &[]);

        // Create the no-op function for unregistered slots
        let mut noop_fb = FunctionBuilder::new(&[DataType::I32], &[]);
        noop_fb.set_name("post_return_noop".to_string());
        let noop_id = noop_fb.finish_module(&mut self.module);

        // Determine table size from max of all export record_ids (core_func_dispatch
        // contains all exports, post_return_dispatch only those with post_return)
        let table_size = self
            .core_func_dispatch
            .iter()
            .map(|(id, _, _)| *id + 1)
            .max()
            .unwrap_or(0);

        // Create wrappers for exports that have post_return
        let entries = self.post_return_dispatch.clone();
        let mut table_entries: Vec<(u32, FunctionID)> = Vec::new();

        for (record_id, post_return_func_id, type_id) in &entries {
            let func_type = self.module.types.get(*type_id).unwrap().clone();
            let (params, _results) = get_func_type_params_results(&func_type);

            let mut wfb = FunctionBuilder::new(&[DataType::I32], &[]);
            wfb.set_name(format!("post_return_wrapper_{}", record_id));
            let w_args_ptr = LocalID(0);

            // Load each argument from args_ptr
            let mut arg_offset: u64 = 0;
            for param_ty in &params {
                wfb.local_get(w_args_ptr);
                arg_offset = emit_typed_load(&mut wfb, param_ty, driver_mem, arg_offset);
            }

            wfb.call(*post_return_func_id);
            let wrapper_id = wfb.finish_module(&mut self.module);
            table_entries.push((*record_id, wrapper_id));
        }

        // Create the table: fill all slots with no-op, then overwrite registered ones
        let table_index = self.pending_tables.len() as u32;
        self.pending_tables.push(Table::new(
            TableType {
                initial: table_size as u64,
                maximum: Some(table_size as u64),
                element_type: RefType::FUNCREF,
                shared: false,
                table64: false,
            },
            None,
            None,
        ));

        // Fill all slots with no-op via one element segment
        if table_size > 0 {
            self.pending_elements.push(Element::new(
                ElementKind::Active {
                    table_index: Some(table_index),
                    offset_expr: InitExpr::new(vec![InitInstr::Value(Value::I32(0))]),
                },
                ElementItems::Functions(vec![noop_id; table_size as usize]),
                None,
            ));
        }

        // Overwrite registered slots with their wrappers
        for &(idx, func_id) in &table_entries {
            self.pending_elements.push(Element::new(
                ElementKind::Active {
                    table_index: Some(table_index),
                    offset_expr: InitExpr::new(vec![InitInstr::Value(Value::I32(idx as i32))]),
                },
                ElementItems::Functions(vec![func_id]),
                None,
            ));
        }

        // Build the dispatch function: just forwards to call_indirect
        let mut fb = FunctionBuilder::new(&[DataType::I32, DataType::I32], &[]);
        fb.set_name("dispatch_post_return".to_string());
        let export_index = LocalID(0);
        let args_ptr = LocalID(1);

        fb.local_get(args_ptr);
        fb.local_get(export_index);
        fb.inject(Operator::CallIndirect {
            type_index: *wrapper_type,
            table_index: table_index,
        });

        let func_id = fb.finish_module(&mut self.module);
        self.module
            .exports
            .add_export_func("dispatch_post_return".to_string(), *func_id);
    }

    /// Build `init_replayer() -> ()`.
    /// Passthrough that forwards to the driver's init_replayer.
    fn build_init_replayer(&mut self) {
        let mut fb = FunctionBuilder::new(&[], &[]);
        fb.set_name("init_replayer".to_string());
        fb.call(self.init_replayer);
        let func_id = fb.finish_module(&mut self.module);
        self.module
            .exports
            .add_export_func("init_replayer".to_string(), *func_id);
    }

    /// Build `replay_instruction(result: i32) -> i32`.
    /// Passthrough that forwards the instruction result to the driver for validation
    /// and returns the original value.
    fn build_replay_instruction(&mut self) {
        let mut fb = FunctionBuilder::new(&[DataType::I32], &[DataType::I32]);
        fb.set_name("replay_instruction".to_string());
        let result = LocalID(0);

        // Call the driver's replay_instruction to validate
        fb.local_get(result);
        fb.call(self.replay_instruction);
        // Return the recorded result instead of current result

        let func_id = fb.finish_module(&mut self.module);
        self.module
            .exports
            .add_export_func("replay_instruction".to_string(), *func_id);
    }

    // ====================
    // ==== Finish ====
    // ====================

    /// Finalize the glue module: build all dispatch functions and return the module.
    pub fn finish(mut self) -> Module<'a> {
        assert!(
            self.imported_memories.len() <= 1,
            "Expected at most one component memory, but found {}: {:?}",
            self.imported_memories.len(),
            self.imported_memories.keys().collect::<Vec<_>>()
        );
        self.build_get_sha256_checksum();
        self.build_init_replayer();
        self.build_replay_instruction();
        self.build_dispatch_realloc();
        self.build_dispatch_memory_write();
        self.build_dispatch_post_return();
        self.build_dispatch_core_func();

        // Set the accumulated tables and elements on the module
        self.module.tables = ModuleTables::new(self.pending_tables);
        self.module.elements = self.pending_elements;
        self.module
    }
}

// ====================================
// ==== Codegen Helpers ====
// ====================================

/// Emit FFI buffer allocation: call `allocate_args_results_buffer(total_bytes, count)`,
/// then load `bytes_ptr` (offset 0) and `sizes_ptr` (offset 4) from the returned struct.
/// Returns `(ffi_ptr, bytes_ptr, sizes_ptr)` locals.
fn emit_alloc_ffi_buffer(
    fb: &mut FunctionBuilder,
    total_bytes: u32,
    count: u32,
    allocate_fn: FunctionID,
    driver_mem: u32,
) -> (LocalID, LocalID, LocalID) {
    let ffi_ptr = fb.add_local(DataType::I32);
    let bytes_ptr = fb.add_local(DataType::I32);
    let sizes_ptr = fb.add_local(DataType::I32);

    fb.i32_const(total_bytes as i32);
    fb.i32_const(count as i32);
    fb.call(allocate_fn);
    fb.local_set(ffi_ptr);

    // Read bytes_ptr from FFI struct (offset 0)
    fb.local_get(ffi_ptr);
    fb.i32_load(MemArg {
        align: 2,
        max_align: 2,
        offset: 0,
        memory: driver_mem,
    });
    fb.local_set(bytes_ptr);

    // Read sizes_ptr from FFI struct (offset 4)
    fb.local_get(ffi_ptr);
    fb.i32_load(MemArg {
        align: 2,
        max_align: 2,
        offset: 4,
        memory: driver_mem,
    });
    fb.local_set(sizes_ptr);

    (ffi_ptr, bytes_ptr, sizes_ptr)
}

/// Emit direction-based dispatch: branch on `direction` local, push `args` locals,
/// then `call_indirect` into the appropriate table via `index`.
fn emit_direction_dispatch(
    fb: &mut FunctionBuilder,
    direction: LocalID,
    index: LocalID,
    args: &[LocalID],
    call_type: TypeID,
    import_table_idx: u32,
    export_table_idx: u32,
    result_block_type: BlockType,
) {
    fb.local_get(direction);
    fb.if_stmt(result_block_type);
    {
        for arg in args {
            fb.local_get(*arg);
        }
        fb.local_get(index);
        fb.inject(Operator::CallIndirect {
            type_index: *call_type,
            table_index: export_table_idx,
        });
    }
    fb.else_stmt();
    {
        for arg in args {
            fb.local_get(*arg);
        }
        fb.local_get(index);
        fb.inject(Operator::CallIndirect {
            type_index: *call_type,
            table_index: import_table_idx,
        });
    }
    fb.end();
}

/// Byte size of a Wasm value type.
fn data_type_byte_size(ty: &DataType) -> u32 {
    match ty {
        DataType::I32 | DataType::F32 => 4,
        DataType::I64 | DataType::F64 => 8,
        _ => panic!("Unsupported data type {:?}", ty),
    }
}

/// Emit a typed load from memory at the given offset.
/// Expects the base pointer already on the stack. Leaves the loaded value on the stack.
/// Returns the offset advanced past this value.
fn emit_typed_load(fb: &mut FunctionBuilder, ty: &DataType, memory: u32, offset: u64) -> u64 {
    let size = data_type_byte_size(ty);
    match ty {
        DataType::I32 => {
            fb.i32_load(MemArg {
                align: 2,
                max_align: 2,
                offset,
                memory,
            });
        }
        DataType::I64 => {
            fb.i64_load(MemArg {
                align: 3,
                max_align: 3,
                offset,
                memory,
            });
        }
        DataType::F32 => {
            fb.f32_load(MemArg {
                align: 2,
                max_align: 2,
                offset,
                memory,
            });
        }
        DataType::F64 => {
            fb.f64_load(MemArg {
                align: 3,
                max_align: 3,
                offset,
                memory,
            });
        }
        _ => panic!("Unsupported type {:?} in emit_typed_load", ty),
    }
    offset + size as u64
}

/// Emit a typed store to memory at the given offset.
/// Expects [base_ptr, value] already on the stack.
/// Returns the offset advanced past this value.
fn emit_typed_store(fb: &mut FunctionBuilder, ty: &DataType, memory: u32, offset: u64) -> u64 {
    let size = data_type_byte_size(ty);
    match ty {
        DataType::I32 => {
            fb.i32_store(MemArg {
                align: 2,
                max_align: 2,
                offset,
                memory,
            });
        }
        DataType::I64 => {
            fb.i64_store(MemArg {
                align: 3,
                max_align: 3,
                offset,
                memory,
            });
        }
        DataType::F32 => {
            fb.f32_store(MemArg {
                align: 2,
                max_align: 2,
                offset,
                memory,
            });
        }
        DataType::F64 => {
            fb.f64_store(MemArg {
                align: 3,
                max_align: 3,
                offset,
                memory,
            });
        }
        _ => panic!("Unsupported type {:?} in emit_typed_store", ty),
    }
    offset + size as u64
}

/// Emit stores of 1-byte size descriptors for each type into `sizes_ptr`.
fn emit_store_size_descriptors(
    fb: &mut FunctionBuilder,
    types: &[DataType],
    sizes_ptr: LocalID,
    memory: u32,
) {
    for (i, ty) in types.iter().enumerate() {
        fb.local_get(sizes_ptr);
        fb.i32_const(data_type_byte_size(ty) as i32);
        fb.i32_store8(MemArg {
            align: 0,
            max_align: 0,
            offset: i as u64,
            memory,
        });
    }
}

/// Extract params and results from a wirm Types::FuncType.
fn get_func_type_params_results(ty: &Types) -> (Vec<DataType>, Vec<DataType>) {
    match ty {
        Types::FuncType {
            params, results, ..
        } => (params.to_vec(), results.to_vec()),
        _ => panic!("Expected FuncType, got {:?}", ty),
    }
}
