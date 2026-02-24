use escargot::format::Message;
use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use clap::Args;
use decomposer::wasmparser::MemArg;
use decomposer::wasmparser::MemoryType;
use decomposer::wirm::Module;
use decomposer::wirm::ir::function::FunctionBuilder;
use decomposer::wirm::ir::id::{FunctionID, LocalID, MemoryID, TypeID};
use decomposer::wirm::ir::module::module_types::Types;
use decomposer::wirm::ir::types::BlockType;
use decomposer::wirm::module_builder::AddLocal;
use decomposer::wirm::opcode::Opcode;

use crate::linking::{
    Checksum, ExportFuncMetadata, ImportAdapterCrimpData, LinkingMetadata, ModuleInstanceExport,
    ModuleInstanceID, module_name_from_ids,
};

pub const GLUE_MODULE_NAME: &str = "crimp_glue";
pub const DRIVER_MODULE_NAME: &str = "crimp_driver";

use decomposer::wirm::DataType;

#[derive(Debug, Default)]
pub struct DriverGlueModules<'a> {
    pub driver: Module<'a>,
    pub glue: Module<'a>,
}

impl<'a> DriverGlueModules<'a> {
    /// Build the crimp-glue-driver crate targeting wasm32-wasip1 with the given trace path,
    /// parse the resulting .wasm into a Module, and finalize the glue module from the builder.
    pub fn from_path_and_builder(trace_path: PathBuf, builder: GlueBuilder<'a>) -> Result<Self> {
        let driver_manifest = PathBuf::from(env!("CRIMP_DRIVER_MANIFEST"));
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
        // This is acceptable since the decomposer is a short-lived CLI tool.
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
    allocate_args_results_buffer: FunctionID,

    // Dedup caches for component imports
    imported_memories: HashMap<(ModuleInstanceID, String), MemoryID>,
    imported_funcs: HashMap<(ModuleInstanceID, String), (FunctionID, TypeID)>,

    // Global import counter (unique across all modules)
    next_import_id: u32,

    // Dispatch tables (populated per-export and per-import, consumed in finish)
    // (direction: 0=Import/1=Export, index, func/mem in glue module)
    realloc_dispatch: Vec<(i32, u32, FunctionID)>,
    memory_write_dispatch: Vec<(i32, u32, MemoryID)>,
    core_func_dispatch: Vec<(u32, FunctionID, TypeID)>,
    post_return_dispatch: Vec<(u32, FunctionID, TypeID)>,
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
        // replay_builtin_call: (i32) -> i32
        let replay_builtin_type = module
            .types
            .add_func_type(&[DataType::I32], &[DataType::I32]);
        let (replay_builtin_call, _) = module.add_import_func(
            DRIVER_MODULE_NAME.to_string(),
            "replay_builtin_call".to_string(),
            replay_builtin_type,
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
            allocate_args_results_buffer,
            next_import_id: 0,
            imported_memories: HashMap::new(),
            imported_funcs: HashMap::new(),
            realloc_dispatch: Vec::new(),
            memory_write_dispatch: Vec::new(),
            core_func_dispatch: Vec::new(),
            post_return_dispatch: Vec::new(),
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
            self.realloc_dispatch.push((0, import_id, realloc_func_id));
        }
        if let Some(memory) = &adapter.memory {
            let memory_instance_name =
                module_name_from_ids(linking.module_id(memory.mid), memory.mid);
            let mem_id = self.ensure_memory_imported(memory, &memory_instance_name);
            self.memory_write_dispatch.push((0, import_id, mem_id));
        }
        let mut fb = FunctionBuilder::new(params, results);
        fb.set_name(format!("stub_{}", export_name));
        let driver_mem = *self.driver_memory;

        let replay_func = if adapter.is_builtin {
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

        let ffi_ptr = fb.add_local(DataType::I32);
        let bytes_ptr = fb.add_local(DataType::I32);
        let sizes_ptr = fb.add_local(DataType::I32);

        // Allocate FFI buffer for params
        let total_param_bytes: u32 = params.iter().map(|ty| data_type_byte_size(ty)).sum();
        let num_params: u32 = params.len() as u32;
        fb.i32_const(total_param_bytes as i32);
        fb.i32_const(num_params as i32);
        fb.call(self.allocate_args_results_buffer);
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

        // Store each param into bytes_ptr (driver memory)
        let mut byte_offset: u64 = 0;
        for (local_id, param_ty) in &param_locals {
            fb.local_get(bytes_ptr);
            fb.local_get(*local_id);
            let size = data_type_byte_size(param_ty);
            match param_ty {
                DataType::I32 => {
                    fb.i32_store(MemArg {
                        align: 2,
                        max_align: 2,
                        offset: byte_offset,
                        memory: driver_mem,
                    });
                }
                DataType::I64 => {
                    fb.i64_store(MemArg {
                        align: 3,
                        max_align: 3,
                        offset: byte_offset,
                        memory: driver_mem,
                    });
                }
                DataType::F32 => {
                    fb.f32_store(MemArg {
                        align: 2,
                        max_align: 2,
                        offset: byte_offset,
                        memory: driver_mem,
                    });
                }
                DataType::F64 => {
                    fb.f64_store(MemArg {
                        align: 3,
                        max_align: 3,
                        offset: byte_offset,
                        memory: driver_mem,
                    });
                }
                _ => panic!(
                    "Unsupported param type {:?} in replay stub for {}",
                    param_ty, export_name
                ),
            }
            byte_offset += size as u64;
        }

        // Store size descriptors into sizes_ptr (1 byte each)
        for (i, param_ty) in params.iter().enumerate() {
            fb.local_get(sizes_ptr);
            fb.i32_const(data_type_byte_size(param_ty) as i32);
            fb.i32_store8(MemArg {
                align: 0,
                max_align: 0,
                offset: i as u64,
                memory: driver_mem,
            });
        }

        // Call the replay function with (import_index, params_bytes_len, params_sizes_len)
        fb.i32_const(import_id as i32);
        if adapter.is_builtin {
            fb.call(replay_func);
        } else {
            fb.i32_const(total_param_bytes as i32);
            fb.i32_const(num_params as i32);
            fb.call(replay_func);
        }

        if results.is_empty() {
            // No return values: drop the pointer
            fb.drop();
        } else {
            // Save the returned pointer to a local
            let ret_ptr = fb.add_local(DataType::I32);
            fb.local_set(ret_ptr);

            // Load each result from driver memory at ret_ptr + offset
            let mut offset: u64 = 0;
            for result_ty in results {
                fb.local_get(ret_ptr);
                match result_ty {
                    DataType::I32 => {
                        fb.i32_load(MemArg {
                            align: 2,
                            max_align: 2,
                            offset,
                            memory: driver_mem,
                        });
                        offset += 4;
                    }
                    DataType::I64 => {
                        fb.i64_load(MemArg {
                            align: 3,
                            max_align: 3,
                            offset,
                            memory: driver_mem,
                        });
                        offset += 8;
                    }
                    DataType::F32 => {
                        fb.f32_load(MemArg {
                            align: 2,
                            max_align: 2,
                            offset,
                            memory: driver_mem,
                        });
                        offset += 4;
                    }
                    DataType::F64 => {
                        fb.f64_load(MemArg {
                            align: 3,
                            max_align: 3,
                            offset,
                            memory: driver_mem,
                        });
                        offset += 8;
                    }
                    _ => panic!(
                        "Unsupported return type {:?} in replay stub for {}",
                        result_ty, export_name
                    ),
                }
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
        if let Some(opts) = &export_func.opts {
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
                self.realloc_dispatch
                    .push((1, export_func.record_id.0, realloc_func_id));
            }
            if let Some(memory) = &opts.memory {
                let memory_instance_name =
                    module_name_from_ids(linking.module_id(memory.mid), memory.mid);
                let mem_id = self.ensure_memory_imported(memory, &memory_instance_name);
                self.memory_write_dispatch
                    .push((1, export_func.record_id.0, mem_id));
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
                let pr_type_id = pr_module
                    .functions
                    .get_type_id(FunctionID(pr_export.index));
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
        }

        Ok(())
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
    /// Dispatches to the appropriate imported realloc based on direction and index.
    fn build_dispatch_realloc(&mut self) {
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

        let entries = self.realloc_dispatch.clone();
        for (entry_dir, entry_index, realloc_func_id) in &entries {
            // if (direction == entry_dir && index == entry_index) { call realloc; return }
            fb.local_get(direction);
            fb.i32_const(*entry_dir);
            fb.i32_eq();
            fb.local_get(index);
            fb.i32_const(*entry_index as i32);
            fb.i32_eq();
            fb.i32_and();
            fb.if_stmt(BlockType::Type(DataType::I32));
            {
                fb.local_get(old_addr);
                fb.local_get(old_size);
                fb.local_get(old_align);
                fb.local_get(new_size);
                fb.call(*realloc_func_id);
                fb.return_stmt();
            }
            fb.else_stmt();
        }

        // Default: unreachable
        fb.unreachable();

        // Close all if/else blocks
        for _ in &entries {
            fb.end();
        }

        let func_id = fb.finish_module(&mut self.module);
        self.module
            .exports
            .add_export_func("dispatch_realloc".to_string(), *func_id);
    }

    /// Build `dispatch_memory_write(direction, index, offset, bytes_ptr, num_bytes)`.
    /// Copies bytes from driver memory to the appropriate component memory.
    fn build_dispatch_memory_write(&mut self) {
        let mut fb = FunctionBuilder::new(
            &[DataType::I32, DataType::I32, DataType::I32, DataType::I32, DataType::I32],
            &[],
        );
        fb.set_name("dispatch_memory_write".to_string());
        let direction = LocalID(0);
        let index = LocalID(1);
        let offset = LocalID(2);
        let bytes_ptr = LocalID(3);
        let num_bytes = LocalID(4);

        let driver_mem = *self.driver_memory;
        let entries = self.memory_write_dispatch.clone();
        for (entry_dir, entry_index, target_mem_id) in &entries {
            // if (direction == entry_dir && index == entry_index) { memory.copy; return }
            fb.local_get(direction);
            fb.i32_const(*entry_dir);
            fb.i32_eq();
            fb.local_get(index);
            fb.i32_const(*entry_index as i32);
            fb.i32_eq();
            fb.i32_and();
            fb.if_stmt(BlockType::Empty);
            {
                // memory.copy(dst=component_mem, src=driver_mem)
                // stack: [dst_offset, src_offset, len]
                fb.local_get(offset);
                fb.local_get(bytes_ptr);
                fb.local_get(num_bytes);
                fb.memory_copy(**target_mem_id, driver_mem);
                fb.return_stmt();
            }
            fb.else_stmt();
        }

        fb.unreachable();

        for _ in &entries {
            fb.end();
        }

        let func_id = fb.finish_module(&mut self.module);
        self.module
            .exports
            .add_export_func("dispatch_memory_write".to_string(), *func_id);
    }

    /// Build `dispatch_core_func(export_index, args_ptr, return_bytes_len, return_sizes_len)`.
    /// Reads args from driver memory, calls the appropriate component function,
    /// writes results into the FFI backing buffers via `allocate_args_results_buffer`,
    /// and writes the return lengths to the out-pointers.
    fn build_dispatch_core_func(&mut self) {
        let mut fb = FunctionBuilder::new(
            &[DataType::I32, DataType::I32, DataType::I32, DataType::I32],
            &[],
        );
        fb.set_name("dispatch_core_func".to_string());
        let export_index = LocalID(0);
        let args_ptr = LocalID(1);
        let return_bytes_len = LocalID(2);
        let return_sizes_len = LocalID(3);
        let driver_mem = *self.driver_memory;

        // Locals for FFI struct interaction
        let ffi_ptr = fb.add_local(DataType::I32);
        let bytes_ptr = fb.add_local(DataType::I32);
        let sizes_ptr = fb.add_local(DataType::I32);

        let entries = self.core_func_dispatch.clone();
        for (record_id, core_func_id, type_id) in &entries {
            let func_type = self.module.types.get(*type_id).unwrap().clone();
            let (params, results) = get_func_type_params_results(&func_type);

            let total_bytes: u32 = results.iter().map(|r| data_type_byte_size(r)).sum();
            let num_results: u32 = results.len() as u32;

            fb.local_get(export_index);
            fb.i32_const(*record_id as i32);
            fb.i32_eq();
            fb.if_stmt(BlockType::Empty);
            {
                // Load each argument from args_ptr (driver memory)
                let mut arg_offset: u64 = 0;
                for param_ty in &params {
                    fb.local_get(args_ptr);
                    match param_ty {
                        DataType::I32 => {
                            fb.i32_load(MemArg {
                                align: 2,
                                max_align: 2,
                                offset: arg_offset,
                                memory: driver_mem,
                            });
                            arg_offset += 4;
                        }
                        DataType::I64 => {
                            fb.i64_load(MemArg {
                                align: 3,
                                max_align: 3,
                                offset: arg_offset,
                                memory: driver_mem,
                            });
                            arg_offset += 8;
                        }
                        DataType::F32 => {
                            fb.f32_load(MemArg {
                                align: 2,
                                max_align: 2,
                                offset: arg_offset,
                                memory: driver_mem,
                            });
                            arg_offset += 4;
                        }
                        DataType::F64 => {
                            fb.f64_load(MemArg {
                                align: 3,
                                max_align: 3,
                                offset: arg_offset,
                                memory: driver_mem,
                            });
                            arg_offset += 8;
                        }
                        _ => panic!(
                            "Unsupported param type {:?} in dispatch_core_func",
                            param_ty
                        ),
                    }
                }

                // Call the core function
                fb.call(*core_func_id);

                // Save results to locals (pop in reverse since stack is LIFO)
                if !results.is_empty() {
                    let result_locals: Vec<LocalID> =
                        results.iter().map(|ty| fb.add_local(*ty)).collect();
                    for local in result_locals.iter().rev() {
                        fb.local_set(*local);
                    }

                    // Allocate the return buffer now that we have results
                    fb.i32_const(total_bytes as i32);
                    fb.i32_const(num_results as i32);
                    fb.call(self.allocate_args_results_buffer);
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

                    // Store each result value into bytes_ptr (driver memory)
                    let mut byte_offset: u64 = 0;
                    for (i, result_ty) in results.iter().enumerate() {
                        fb.local_get(bytes_ptr);
                        fb.local_get(result_locals[i]);
                        let size = data_type_byte_size(result_ty);
                        match result_ty {
                            DataType::I32 => {
                                fb.i32_store(MemArg {
                                    align: 2,
                                    max_align: 2,
                                    offset: byte_offset,
                                    memory: driver_mem,
                                });
                            }
                            DataType::I64 => {
                                fb.i64_store(MemArg {
                                    align: 3,
                                    max_align: 3,
                                    offset: byte_offset,
                                    memory: driver_mem,
                                });
                            }
                            DataType::F32 => {
                                fb.f32_store(MemArg {
                                    align: 2,
                                    max_align: 2,
                                    offset: byte_offset,
                                    memory: driver_mem,
                                });
                            }
                            DataType::F64 => {
                                fb.f64_store(MemArg {
                                    align: 3,
                                    max_align: 3,
                                    offset: byte_offset,
                                    memory: driver_mem,
                                });
                            }
                            _ => panic!(
                                "Unsupported result type {:?} in dispatch_core_func",
                                result_ty
                            ),
                        }
                        byte_offset += size as u64;
                    }

                    // Store size descriptors into sizes_ptr (1 byte each)
                    for (i, result_ty) in results.iter().enumerate() {
                        fb.local_get(sizes_ptr);
                        fb.i32_const(data_type_byte_size(result_ty) as i32);
                        fb.i32_store8(MemArg {
                            align: 0,
                            max_align: 0,
                            offset: i as u64,
                            memory: driver_mem,
                        });
                    }
                }

                // Write return lengths to out-pointers in driver memory
                fb.local_get(return_bytes_len);
                fb.i32_const(total_bytes as i32);
                fb.i32_store(MemArg {
                    align: 2,
                    max_align: 2,
                    offset: 0,
                    memory: driver_mem,
                });

                fb.local_get(return_sizes_len);
                fb.i32_const(num_results as i32);
                fb.i32_store(MemArg {
                    align: 2,
                    max_align: 2,
                    offset: 0,
                    memory: driver_mem,
                });

                fb.return_stmt();
            }
            fb.else_stmt();
        }

        // Default: unreachable
        fb.unreachable();

        for _ in &entries {
            fb.end();
        }

        let func_id = fb.finish_module(&mut self.module);
        self.module
            .exports
            .add_export_func("dispatch_core_func".to_string(), *func_id);
    }

    /// Build `dispatch_post_return(export_index, args_ptr)`.
    /// Reads args from driver memory and calls the appropriate post_return function.
    fn build_dispatch_post_return(&mut self) {
        let mut fb = FunctionBuilder::new(
            &[DataType::I32, DataType::I32],
            &[],
        );
        fb.set_name("dispatch_post_return".to_string());
        let export_index = LocalID(0);
        let args_ptr = LocalID(1);
        let driver_mem = *self.driver_memory;

        let entries = self.post_return_dispatch.clone();
        for (record_id, post_return_func_id, type_id) in &entries {
            let func_type = self.module.types.get(*type_id).unwrap().clone();
            let (params, _results) = get_func_type_params_results(&func_type);

            fb.local_get(export_index);
            fb.i32_const(*record_id as i32);
            fb.i32_eq();
            fb.if_stmt(BlockType::Empty);
            {
                // Load each argument from args_ptr (driver memory)
                let mut arg_offset: u64 = 0;
                for param_ty in &params {
                    fb.local_get(args_ptr);
                    match param_ty {
                        DataType::I32 => {
                            fb.i32_load(MemArg {
                                align: 2,
                                max_align: 2,
                                offset: arg_offset,
                                memory: driver_mem,
                            });
                            arg_offset += 4;
                        }
                        DataType::I64 => {
                            fb.i64_load(MemArg {
                                align: 3,
                                max_align: 3,
                                offset: arg_offset,
                                memory: driver_mem,
                            });
                            arg_offset += 8;
                        }
                        DataType::F32 => {
                            fb.f32_load(MemArg {
                                align: 2,
                                max_align: 2,
                                offset: arg_offset,
                                memory: driver_mem,
                            });
                            arg_offset += 4;
                        }
                        DataType::F64 => {
                            fb.f64_load(MemArg {
                                align: 3,
                                max_align: 3,
                                offset: arg_offset,
                                memory: driver_mem,
                            });
                            arg_offset += 8;
                        }
                        _ => panic!(
                            "Unsupported param type {:?} in dispatch_post_return",
                            param_ty
                        ),
                    }
                }

                fb.call(*post_return_func_id);
                fb.return_stmt();
            }
            fb.else_stmt();
        }

        // Default: no-op return for exports without an explicit post_return
        for _ in &entries {
            fb.end();
        }

        let func_id = fb.finish_module(&mut self.module);
        self.module
            .exports
            .add_export_func("dispatch_post_return".to_string(), *func_id);
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
        self.build_dispatch_realloc();
        self.build_dispatch_memory_write();
        self.build_dispatch_post_return();
        self.build_dispatch_core_func();
        self.module
    }
}

/// Byte size of a Wasm value type.
fn data_type_byte_size(ty: &DataType) -> u32 {
    match ty {
        DataType::I32 | DataType::F32 => 4,
        DataType::I64 | DataType::F64 => 8,
        _ => panic!("Unsupported data type {:?}", ty),
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
