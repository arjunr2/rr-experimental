//! Shadow memory instrumentation for wasm components.
//!
//! Instruments each core module with shadow memory (component_mode=true),
//! then wires component-level r3 imports so the host can provide
//! `record_import_call` and `record_memory_write` as scalar-only functions.

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use wirm::wasmparser::{
    CanonicalFunction, ComponentAlias, ComponentExportName, ComponentExternalKind,
    ComponentFuncType, ComponentImport, ComponentImportName, ComponentType, ComponentTypeRef,
    ComponentValType, Export, ExternalKind, Instance, InstanceTypeDeclaration, InstantiationArg,
    InstantiationArgKind, PrimitiveValType,
};

#[derive(Parser)]
#[command(name = "r3-instrument-component")]
#[command(about = "Add shadow memory instrumentation to a wasm component")]
struct Args {
    /// Input wasm component
    #[arg(short, long)]
    component: PathBuf,

    /// Output instrumented wasm component
    #[arg(short, long)]
    output: PathBuf,
}

fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();

    let wasm_bytes = std::fs::read(&args.component)?;

    let mut component = wirm::ir::component::Component::parse(&wasm_bytes, true, false)
        .map_err(|e| anyhow::anyhow!("parse error: {}", e))?;

    // Phase 1: Instrument each core module with component_mode=true.
    // Track which module indices were actually instrumented.
    let mut instrumented_modules = Vec::new();
    for (i, module) in component.modules.iter_mut().enumerate() {
        if r3_baseline::instrument_shadow(module, true)? {
            instrumented_modules.push(i as u32);
        }
    }

    if !instrumented_modules.is_empty() {
        // Phase 2: Wire component-level r3 imports.
        wire_r3_component_imports(&mut component, &instrumented_modules)?;
    }

    let output_bytes = component
        .encode()
        .map_err(|e| anyhow::anyhow!("encode error: {}", e))?;

    // Validate the output component
    let mut validator =
        wirm::wasmparser::Validator::new_with_features(wirm::wasmparser::WasmFeatures::all());
    validator
        .validate_all(&output_bytes)
        .map_err(|e| anyhow::anyhow!("output validation failed: {}", e))?;

    std::fs::write(&args.output, &output_bytes)?;
    log::info!(
        "Wrote instrumented component ({} bytes) to {:?}",
        output_bytes.len(),
        args.output
    );

    Ok(())
}

/// Wire r3 imports at the component level:
/// 1. Define component func types for record-import-call and record-memory-write
/// 2. Define instance type exporting both functions
/// 3. Import "r3" instance with that type
/// 4. Alias each function from the instance
/// 5. Canon lower each (scalar-only, no memory option needed)
/// 6. Bundle lowered core functions into a core instance
/// 7. Add that core instance as "r3" arg to each instrumented module's instantiation
fn wire_r3_component_imports(
    component: &mut wirm::ir::component::Component<'_>,
    instrumented_modules: &[u32],
) -> Result<()> {
    // Step 1: Define instance type with two function exports
    let decls = vec![
        // Type 0: record-import-call (func (param "func-idx" u32))
        InstanceTypeDeclaration::Type(ComponentType::Func(ComponentFuncType {
            async_: false,
            params: Box::new([("func-idx", ComponentValType::Primitive(PrimitiveValType::U32))]),
            result: None,
        })),
        InstanceTypeDeclaration::Export {
            name: ComponentExportName("record-import-call"),
            ty: ComponentTypeRef::Func(0),
        },
        // Type 1: record-memory-write (func (param "addr" u32) (param "size" u32) (param "lo" u64) (param "hi" u64))
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

    // Step 4: Canon lower each function (all scalar params, no memory option needed)
    let import_call_core = component.add_canon_func(CanonicalFunction::Lower {
        func_index: *import_call_alias,
        options: vec![].into_boxed_slice(),
    });
    let memory_write_core = component.add_canon_func(CanonicalFunction::Lower {
        func_index: *memory_write_alias,
        options: vec![].into_boxed_slice(),
    });

    // Step 5: Bundle lowered core functions into a core instance
    let r3_core_inst = component.add_core_instance(Instance::FromExports(Box::new([
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
    ])));

    // Step 6: Wire r3 core instance into each instrumented module's instantiation
    for inst in component.instances.iter_mut() {
        if let Instance::Instantiate {
            module_index, args, ..
        } = inst
        {
            if instrumented_modules.contains(module_index) {
                let mut new_args: Vec<_> = args.iter().cloned().collect();
                new_args.push(InstantiationArg {
                    name: "r3",
                    kind: InstantiationArgKind::Instance,
                    index: *r3_core_inst,
                });
                *args = new_args.into_boxed_slice();
            }
        }
    }

    Ok(())
}
