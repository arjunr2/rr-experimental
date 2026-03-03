//! Parsing logic to convert raw bytes into our Component IR.

use anyhow::{Result, bail};
use std::cell::RefCell;
use std::rc::Rc;
use wirm::Module;
use wirm::wasmparser::{
    self, CanonicalOption, ComponentAlias, ComponentExport, ComponentExternalKind, ComponentImport,
    ComponentInstance, ComponentTypeRef, ExternalKind, Instance, Parser, Payload,
};

use crate::ir::{
    AliasInfo, Component, ComponentFuncNode, ComponentInstanceNode, ComponentNode, ComponentRef,
    CoreFuncNode, CoreGlobalNode, CoreInstanceNode, CoreMemoryNode, CoreTableNode, CoreTypeNode,
    ModuleNode, ParentScope, TypeNode, ValueNode,
};

/// Parse a WebAssembly Component from bytes into our IR.
///
/// Returns a ComponentRef (Rc<RefCell<Component>>) with nested components also wrapped.
/// Each component stores its parent chain as Weak references for Outer alias resolution.
pub fn parse_component<'a>(bytes: &'a [u8]) -> Result<ComponentRef<'a>> {
    parse_component_with_parents(bytes, vec![])
}

/// Parse a component with an explicit parent chain for Outer alias resolution.
fn parse_component_with_parents<'a>(
    bytes: &'a [u8],
    parents: Vec<ParentScope<'a>>,
) -> Result<ComponentRef<'a>> {
    let parser = Parser::new(0);

    // Create the component wrapped in Rc<RefCell> immediately
    let component_ref: ComponentRef = Rc::new(RefCell::new(Component {
        parents,
        ..Default::default()
    }));

    let mut depth: u32 = 0;

    for payload in parser.parse_all(bytes) {
        let payload = payload?;
        handle_payload(&component_ref, payload, bytes, &mut depth)?;
    }

    Ok(component_ref)
}

fn handle_payload<'a>(
    component_ref: &ComponentRef<'a>,
    payload: Payload<'a>,
    bytes: &'a [u8],
    depth: &mut u32,
) -> Result<()> {
    use Payload::*;

    // If we're inside a nested component/module, skip all payloads except
    // depth tracking. These sections were already parsed recursively when
    // we handled ComponentSection / ModuleSection.
    if *depth > 0 {
        match payload {
            Payload::End { .. } => *depth = depth.saturating_sub(1),
            Payload::ComponentSection { .. } | Payload::ModuleSection { .. } => *depth += 1,
            _ => {}
        }
        return Ok(());
    }

    // Borrow the component mutably for modifications
    let mut component = component_ref.borrow_mut();

    match payload {
        Version { encoding, .. } => {
            if encoding != wasmparser::Encoding::Component {
                bail!("Expected a Component, got a Module");
            }
        }

        // =================================================================
        // Component Type Section
        // =================================================================
        ComponentTypeSection(reader) => {
            for ty in reader {
                let ty = ty?;
                let node = parse_component_type(ty);
                component.types.push(node);
            }
        }

        // =================================================================
        // Component Import Section
        // =================================================================
        ComponentImportSection(reader) => {
            for import in reader {
                let import = import?;
                handle_component_import(&mut component, import);
            }
        }

        // =================================================================
        // Component Instance Section
        // =================================================================
        ComponentInstanceSection(reader) => {
            for instance in reader {
                let instance = instance?;
                let node = parse_component_instance(instance);
                component.instances.push(node);
            }
        }

        // =================================================================
        // Core Instance Section
        // =================================================================
        InstanceSection(reader) => {
            for instance in reader {
                let instance = instance?;
                let node = parse_core_instance(instance);
                component.core_instances.push(node);
            }
        }

        // =================================================================
        // Component Alias Section
        // =================================================================
        ComponentAliasSection(reader) => {
            for alias in reader {
                let alias = alias?;
                handle_component_alias(&mut component, alias);
            }
        }

        // =================================================================
        // Canonical Section (lift/lower)
        // =================================================================
        ComponentCanonicalSection(reader) => {
            for canon in reader {
                let canon = canon?;
                handle_canonical(&mut component, canon);
            }
        }

        // =================================================================
        // Component Export Section
        // =================================================================
        ComponentExportSection(reader) => {
            for export in reader {
                let export = export?;
                handle_component_export(&mut component, export);
            }
        }

        // =================================================================
        // Module Section (inline core module)
        // =================================================================
        ModuleSection {
            parser: _,
            unchecked_range,
        } => {
            // Increment depth to skip payloads emitted from inside this section
            *depth += 1;
            // Extract the module bytes from the range
            let module_bytes = &bytes[unchecked_range.start..unchecked_range.end];
            // Parse the module bytes into wirm Module IR
            let parsed_module = Module::parse(module_bytes, false, false)
                .map_err(|e| anyhow::anyhow!("Failed to parse module: {}", e))?;
            component.modules.push(ModuleNode::Defined {
                module: parsed_module,
            });
        }

        // =================================================================
        // Component Section (nested component)
        // =================================================================
        ComponentSection {
            parser: _,
            unchecked_range,
        } => {
            // Increment depth to skip payloads emitted from inside this section
            *depth += 1;
            let nested_bytes = &bytes[unchecked_range.start..unchecked_range.end];

            // Build parent chain: current component's parents + current component itself
            let mut new_parents = component.parents.clone();
            new_parents.push(ParentScope::Component(Rc::downgrade(component_ref)));

            // Drop the borrow before recursively parsing
            drop(component);

            // Parse nested component with parent chain
            let nested_ref = parse_component_with_parents(nested_bytes, new_parents)?;

            // Re-borrow to add the nested component
            component_ref
                .borrow_mut()
                .components
                .push(ComponentNode::Defined {
                    component: nested_ref,
                });
            return Ok(());
        }

        // =================================================================
        // Core Type Section
        // =================================================================
        CoreTypeSection(reader) => {
            for ty in reader {
                let ty = ty?;
                component.core_types.push(CoreTypeNode::Defined(ty));
            }
        }

        // Ignored payloads
        End { .. } => {
            // Decrement depth when exiting nested module/component
            *depth = depth.saturating_sub(1);
        }
        CustomSection { .. } => {}
        ComponentStartSection { .. } => {
            panic!("Component start sections are not supported");
        }
        _ => {
            // Other payloads we don't handle yet
        }
    }

    Ok(())
}

// =============================================================================
// Import Handling
// =============================================================================

fn handle_component_import<'a>(component: &mut Component<'a>, import: ComponentImport<'a>) {
    // Get the index this import will have in the imports vector
    let import_idx = component.imports.len() as u32;

    // Add to the appropriate index space based on type
    match import.ty {
        ComponentTypeRef::Module(_) => {
            component.modules.push(ModuleNode::Imported(import_idx));
        }
        ComponentTypeRef::Func(_) => {
            component
                .funcs
                .push(ComponentFuncNode::Imported(import_idx));
        }
        ComponentTypeRef::Value(_) => {
            component.values.push(ValueNode::Imported(import_idx));
        }
        ComponentTypeRef::Type(..) => {
            component.types.push(TypeNode::Imported(import_idx));
        }
        ComponentTypeRef::Instance(_) => {
            component
                .instances
                .push(ComponentInstanceNode::Imported(import_idx));
        }
        ComponentTypeRef::Component(_) => {
            component
                .components
                .push(ComponentNode::Imported(import_idx));
        }
    };

    // Store the full import (name + type reference)
    component.imports.push(import);
}

// =============================================================================
// Alias Handling
// =============================================================================

fn handle_component_alias(component: &mut Component, alias: ComponentAlias) {
    match alias {
        ComponentAlias::InstanceExport {
            kind,
            instance_index,
            name,
        } => {
            let info = AliasInfo::InstanceExport {
                instance_idx: instance_index,
                name: name.to_string(),
            };
            push_alias_by_kind(component, kind, info);
        }
        ComponentAlias::CoreInstanceExport {
            kind,
            instance_index,
            name,
        } => {
            let info = AliasInfo::CoreInstanceExport {
                instance_idx: instance_index,
                name: name.to_string(),
            };
            push_core_alias_by_kind(component, kind, info);
        }
        ComponentAlias::Outer { kind, count, index } => {
            let info = AliasInfo::Outer { count, index };
            push_outer_alias_by_kind(component, kind, info);
        }
    }
}

fn push_alias_by_kind(component: &mut Component, kind: ComponentExternalKind, info: AliasInfo) {
    match kind {
        ComponentExternalKind::Module => {
            component.modules.push(ModuleNode::Aliased(info));
        }
        ComponentExternalKind::Func => {
            component.funcs.push(ComponentFuncNode::Aliased(info));
        }
        ComponentExternalKind::Value => {
            component.values.push(ValueNode::Aliased(info));
        }
        ComponentExternalKind::Type => {
            component.types.push(TypeNode::Aliased(info));
        }
        ComponentExternalKind::Instance => {
            component
                .instances
                .push(ComponentInstanceNode::Aliased(info));
        }
        ComponentExternalKind::Component => {
            component.components.push(ComponentNode::Aliased(info));
        }
    }
}

fn push_core_alias_by_kind(component: &mut Component, kind: ExternalKind, info: AliasInfo) {
    match kind {
        ExternalKind::Func | ExternalKind::FuncExact => {
            component.core_funcs.push(CoreFuncNode::Aliased(info));
        }
        ExternalKind::Table => {
            component.core_tables.push(CoreTableNode::Aliased(info));
        }
        ExternalKind::Memory => {
            component.core_memories.push(CoreMemoryNode::Aliased(info));
        }
        ExternalKind::Global => {
            component.core_globals.push(CoreGlobalNode::Aliased(info));
        }
        ExternalKind::Tag => {
            // Tags not tracked separately for now
        }
    }
}

fn push_outer_alias_by_kind(
    component: &mut Component,
    kind: wasmparser::ComponentOuterAliasKind,
    info: AliasInfo,
) {
    use wasmparser::ComponentOuterAliasKind::*;
    match kind {
        CoreModule => {
            component.modules.push(ModuleNode::Aliased(info));
        }
        CoreType => {
            component.core_types.push(CoreTypeNode::Aliased(info));
        }
        Type => {
            component.types.push(TypeNode::Aliased(info));
        }
        Component => {
            component.components.push(ComponentNode::Aliased(info));
        }
    }
}

// =============================================================================
// Canonical Function Handling
// =============================================================================

fn handle_canonical(component: &mut Component, canon: wasmparser::CanonicalFunction) {
    use wasmparser::CanonicalFunction::*;

    match canon {
        Lift {
            core_func_index,
            type_index,
            options,
        } => {
            let opts = parse_canon_options(&options);
            component.funcs.push(ComponentFuncNode::Lifted {
                core_func_idx: core_func_index,
                type_idx: type_index,
                options: opts,
            });
        }
        Lower {
            func_index,
            options,
        } => {
            let opts = parse_canon_options(&options);
            component.core_funcs.push(CoreFuncNode::Lowered {
                func_idx: func_index,
                options: opts,
            });
        }
        ResourceDrop { resource } => {
            component
                .core_funcs
                .push(CoreFuncNode::ResourceDrop { resource });
        }
        ResourceNew { resource } => {
            component
                .core_funcs
                .push(CoreFuncNode::ResourceNew { resource });
        }
        ResourceRep { resource } => {
            component
                .core_funcs
                .push(CoreFuncNode::ResourceRep { resource });
        }
        // Other canonical functions - not supported yet
        _ => {
            panic!("Canonical function variant not supported yet: {:?}", canon);
        }
    }
}

fn parse_canon_options(options: &[wasmparser::CanonicalOption]) -> Vec<CanonicalOption> {
    options.to_vec()
}

// =============================================================================
// Export Handling
// =============================================================================

fn handle_component_export<'a>(component: &mut Component<'a>, export: ComponentExport<'a>) {
    let export_idx = component.exports.len() as u32;
    component.exports.push(export);

    // Add an Exported entry to the appropriate index space
    let export_ref = &component.exports[export_idx as usize];
    match export_ref.kind {
        ComponentExternalKind::Module => {
            component.modules.push(ModuleNode::Exported(export_idx));
        }
        ComponentExternalKind::Func => {
            component
                .funcs
                .push(ComponentFuncNode::Exported(export_idx));
        }
        ComponentExternalKind::Value => {
            component.values.push(ValueNode::Exported(export_idx));
        }
        ComponentExternalKind::Type => {
            component.types.push(TypeNode::Exported(export_idx));
        }
        ComponentExternalKind::Instance => {
            component
                .instances
                .push(ComponentInstanceNode::Exported(export_idx));
        }
        ComponentExternalKind::Component => {
            component
                .components
                .push(ComponentNode::Exported(export_idx));
        }
    }
}

// =============================================================================
// Instance Parsing
// =============================================================================

fn parse_component_instance<'a>(instance: ComponentInstance<'a>) -> ComponentInstanceNode<'a> {
    match instance {
        ComponentInstance::Instantiate {
            component_index,
            args,
        } => ComponentInstanceNode::Instantiated {
            component_idx: component_index,
            args: args.into_vec(),
        },
        ComponentInstance::FromExports(exports) => {
            ComponentInstanceNode::FromExports(exports.into_vec())
        }
    }
}

fn parse_core_instance<'a>(instance: Instance<'a>) -> CoreInstanceNode<'a> {
    match instance {
        Instance::Instantiate { module_index, args } => CoreInstanceNode::Instantiated {
            module_idx: module_index,
            args: args.into_vec(),
        },
        Instance::FromExports(exports) => CoreInstanceNode::FromExports(exports.into_vec()),
    }
}

// =============================================================================
// Type Parsing
// =============================================================================

fn parse_component_type(ty: wasmparser::ComponentType) -> TypeNode {
    TypeNode::Defined(ty)
}
