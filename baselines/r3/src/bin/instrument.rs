//! Shadow memory instrumentation for wasm modules and components.
//!
//! Auto-detects whether the input is a core module or component.
//! For components, instruments each core module with component_mode=true
//! and wires component-level r3 imports.

use anyhow::Result;
use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;
use wirm::ir::component::visitor::{self, ComponentVisitor, VisitCtx};
use wirm::ir::module::Module;
use wirm::wasmparser::{
    CanonicalFunction, ComponentAlias, ComponentExportName, ComponentExternalKind,
    ComponentFuncType, ComponentImport, ComponentImportName, ComponentType, ComponentTypeRef,
    ComponentValType, Export, ExternalKind, Instance, InstanceTypeDeclaration, InstantiationArg,
    InstantiationArgKind, PrimitiveValType, TypeRef,
};

/// Collects the assumed ID for each core instance that is an Instantiate.
#[derive(Default)]
struct CoreInstanceCollector {
    /// Maps module_index → assumed core instance ID
    module_to_instance_id: HashMap<u32, u32>,
}

impl<'a> ComponentVisitor<'a> for CoreInstanceCollector {
    fn visit_core_instance(&mut self, _cx: &VisitCtx<'a>, id: u32, inst: &Instance<'a>) {
        if let Instance::Instantiate { module_index, .. } = inst {
            self.module_to_instance_id.insert(*module_index, id);
        }
    }
}

#[derive(Parser)]
#[command(name = "r3-instrument")]
#[command(about = "Add shadow memory instrumentation to a wasm module or component")]
struct Args {
    /// Input wasm file (.wasm or .wat, module or component)
    #[arg(short, long)]
    input: PathBuf,

    /// Output instrumented wasm file
    #[arg(short, long)]
    output: PathBuf,
}

fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();

    let raw_bytes = std::fs::read(&args.input)?;
    let wasm_bytes =
        wat::parse_bytes(&raw_bytes).map_err(|e| anyhow::anyhow!("wat parse error: {}", e))?;

    // Detect component vs module from the binary preamble (byte 4)
    let is_component = wasm_bytes.get(4) == Some(&0x0d);

    let output_bytes = if is_component {
        instrument_component(&wasm_bytes)?
    } else {
        instrument_module(&wasm_bytes)?
    };

    // Write the output (before validation so we can examine on failure)
    std::fs::write(&args.output, &output_bytes)?;

    // Validate the output
    let mut validator =
        wirm::wasmparser::Validator::new_with_features(wirm::wasmparser::WasmFeatures::all());
    validator
        .validate_all(&output_bytes)
        .map_err(|e| anyhow::anyhow!("output validation failed: {}", e))?;
    let kind = if is_component { "component" } else { "module" };
    log::info!(
        "Wrote instrumented {} ({} bytes) to {:?}",
        kind,
        output_bytes.len(),
        args.output
    );

    Ok(())
}

fn instrument_module(wasm_bytes: &[u8]) -> Result<Vec<u8>> {
    let mut module = Module::parse(wasm_bytes, true, false)
        .map_err(|e| anyhow::anyhow!("parse error: {}", e))?;

    r3_baseline::instrument_shadow(&mut module, false)?;

    module
        .encode()
        .map_err(|e| anyhow::anyhow!("encode error: {}", e))
}

fn instrument_component(wasm_bytes: &[u8]) -> Result<Vec<u8>> {
    let mut component = wirm::ir::component::Component::parse(wasm_bytes, true, false)
        .map_err(|e| anyhow::anyhow!("parse error: {}", e))?;

    // Instrument each core module with component_mode=true.
    // Track which modules have local vs imported memory.
    let mut local_memory_modules = Vec::new();
    let mut imported_memory_modules = Vec::new();
    for (i, module) in component.modules.iter_mut().enumerate() {
        if r3_baseline::instrument_shadow(module, true)? {
            let has_memory_import = module
                .imports
                .iter()
                .any(|imp| matches!(imp.ty, TypeRef::Memory(_)));
            if has_memory_import {
                imported_memory_modules.push(i as u32);
            } else {
                local_memory_modules.push(i as u32);
            }
        }
    }

    let all_instrumented: Vec<u32> = local_memory_modules
        .iter()
        .chain(imported_memory_modules.iter())
        .copied()
        .collect();

    if !all_instrumented.is_empty() {
        wire_r3_component_imports(
            &mut component,
            &all_instrumented,
            &local_memory_modules,
            &imported_memory_modules,
        )?;
    }

    component
        .encode()
        .map_err(|e| anyhow::anyhow!("encode error: {}", e))
}

/// Wire r3 imports at the component level.
///
/// Creates two separate r3 core instances to avoid circular dependencies:
/// - `r3_funcs_inst`: just the two lowered functions (for local-memory modules)
/// - `r3_full_inst`: functions + aliased shadow memory (for imported-memory modules)
///
/// The local-memory module defines shadow memory locally (memory 1), so its r3
/// instance must NOT depend on its own core instance export. Imported-memory
/// modules get shadow memory aliased from the local-memory module's instance.
fn wire_r3_component_imports(
    component: &mut wirm::ir::component::Component<'_>,
    _instrumented_modules: &[u32],
    local_memory_modules: &[u32],
    imported_memory_modules: &[u32],
) -> Result<()> {
    // Use the visitor to collect correct assumed IDs for core instances.
    let mut collector = CoreInstanceCollector::default();
    visitor::walk_structural(&*component, &mut collector);

    // Step 1: Define instance type with two function exports
    let decls = vec![
        InstanceTypeDeclaration::Type(ComponentType::Func(ComponentFuncType {
            async_: false,
            params: Box::new([(
                "func-idx",
                ComponentValType::Primitive(PrimitiveValType::U32),
            )]),
            result: None,
        })),
        InstanceTypeDeclaration::Export {
            name: ComponentExportName("record-import-call"),
            ty: ComponentTypeRef::Func(0),
        },
        InstanceTypeDeclaration::Type(ComponentType::Func(ComponentFuncType {
            async_: false,
            params: Box::new([
                ("addr", ComponentValType::Primitive(PrimitiveValType::U32)),
                ("size", ComponentValType::Primitive(PrimitiveValType::U32)),
                ("lo", ComponentValType::Primitive(PrimitiveValType::U64)),
                ("hi", ComponentValType::Primitive(PrimitiveValType::U64)),
            ]),
            result: None,
        })),
        InstanceTypeDeclaration::Export {
            name: ComponentExportName("record-memory-write"),
            ty: ComponentTypeRef::Func(1),
        },
    ];
    let (inst_ty_id, _) = component.add_type_instance(decls);

    // Step 2: Import the "r3" instance
    let r3_inst_id = component.add_import(ComponentImport {
        name: ComponentImportName("r3"),
        ty: ComponentTypeRef::Instance(*inst_ty_id),
    });

    // Step 3: Alias each function from the r3 instance
    let (import_call_alias, _) = component.add_alias_func(ComponentAlias::InstanceExport {
        instance_index: r3_inst_id,
        kind: ComponentExternalKind::Func,
        name: "record-import-call",
    });
    let (memory_write_alias, _) = component.add_alias_func(ComponentAlias::InstanceExport {
        instance_index: r3_inst_id,
        kind: ComponentExternalKind::Func,
        name: "record-memory-write",
    });

    // Step 4: Canon lower each function
    let import_call_core = component.add_canon_func(CanonicalFunction::Lower {
        func_index: *import_call_alias,
        options: vec![].into_boxed_slice(),
    });
    let memory_write_core = component.add_canon_func(CanonicalFunction::Lower {
        func_index: *memory_write_alias,
        options: vec![].into_boxed_slice(),
    });

    // Step 5: Create functions-only r3 core instance (for local-memory modules).
    // This instance has NO dependency on any core instance, breaking cycles.
    let r3_funcs_exports = vec![
        Export {
            name: "record_import_call",
            kind: ExternalKind::Func,
            index: *import_call_core,
        },
        Export {
            name: "record_memory_write",
            kind: ExternalKind::Func,
            index: *memory_write_core,
        },
    ];
    let r3_funcs_inst =
        component.add_core_instance(Instance::FromExports(r3_funcs_exports.into_boxed_slice()));

    // Step 6: If any module needs imported shadow memory, create a second r3 core
    // instance that includes the shadow memory alias.
    let r3_full_inst = if !imported_memory_modules.is_empty() {
        let source_module_idx = local_memory_modules
            .first()
            .ok_or_else(|| anyhow::anyhow!("imported-memory module but no local-memory module"))?;

        let source_instance_id = *collector
            .module_to_instance_id
            .get(source_module_idx)
            .ok_or_else(|| {
                anyhow::anyhow!("no instance for local-memory module {}", source_module_idx)
            })?;

        let (core_mem_id, _) =
            component.add_alias_core_memory(ComponentAlias::CoreInstanceExport {
                instance_index: source_instance_id,
                kind: ExternalKind::Memory,
                name: r3_baseline::SHADOW_MEMORY_EXPORT,
            });

        let r3_full_exports = vec![
            Export {
                name: "record_import_call",
                kind: ExternalKind::Func,
                index: *import_call_core,
            },
            Export {
                name: "record_memory_write",
                kind: ExternalKind::Func,
                index: *memory_write_core,
            },
            Export {
                name: r3_baseline::SHADOW_MEMORY_EXPORT,
                kind: ExternalKind::Memory,
                index: core_mem_id,
            },
        ];
        Some(component.add_core_instance(Instance::FromExports(r3_full_exports.into_boxed_slice())))
    } else {
        None
    };

    // Step 7: Wire the appropriate r3 core instance into each module's instantiation
    for inst in component.instances.iter_mut() {
        if let Instance::Instantiate {
            module_index, args, ..
        } = inst
        {
            let r3_inst = if imported_memory_modules.contains(module_index) {
                r3_full_inst.as_ref()
            } else if local_memory_modules.contains(module_index) {
                Some(&r3_funcs_inst)
            } else {
                None
            };
            if let Some(r3_id) = r3_inst {
                let mut new_args: Vec<_> = args.iter().cloned().collect();
                new_args.push(InstantiationArg {
                    name: "r3",
                    kind: InstantiationArgKind::Instance,
                    index: **r3_id,
                });
                *args = new_args.into_boxed_slice();
            }
        }
    }

    Ok(())
}
