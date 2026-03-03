//! Resolution helpers for traversing the component graph.
//!
//! The `Resolve` trait provides a uniform interface for resolving aliases
//! to their root definitions. Resolution traces through:
//! - `InstanceExport` aliases (by looking into instantiated components)
//! - `CoreInstanceExport` aliases (by looking into module internals)
//! - `Outer` aliases (by walking up the parent scope chain)
//!
//! Resolution panics if an alias cannot be resolved (invalid component structure).

use super::{
    AliasInfo, Component, ComponentExternalKind, ComponentFuncNode, ComponentInstanceNode,
    ComponentNode, ComponentRef, CoreFuncNode, CoreGlobalNode, CoreInstanceNode, CoreMemoryNode,
    CoreTableNode, CoreTypeNode, IndexSpace, ModuleNode, ParentScope, ResolvedComponent,
    ResolvedComponentFunc, ResolvedComponentInstance, ResolvedCoreFunc, ResolvedCoreGlobal,
    ResolvedCoreInstance, ResolvedCoreMemory, ResolvedCoreTable, ResolvedCoreType, ResolvedImport,
    ResolvedModule, ResolvedType, ResolvedValue, TypeNode, ValueNode,
};
use wirm::wasmparser::ExternalKind;

// =============================================================================
// Resolve Trait
// =============================================================================

/// Trait for node types that can be resolved to a root (non-aliased) form.
pub trait Resolve<'a>: Sized {
    /// The resolved type without `Aliased` variants.
    type Root;

    /// Resolve this node to its root form.
    /// Follows all alias chains until reaching a definition or import.
    fn resolve(&self, component: &Component<'a>) -> Self::Root;

    /// Get the IndexSpace for this node type from a Component.
    fn index_space<'b>(component: &'b Component<'a>) -> &'b IndexSpace<Self>;

    /// Resolve a node by index from the component's index space.
    /// Looks up the item and calls resolve on it.
    fn resolve_index(component: &Component<'a>, idx: u32) -> Self::Root {
        Self::index_space(component)
            .get(idx)
            .unwrap_or_else(|| panic!("Index {} out of bounds", idx))
            .resolve(component)
    }
}

pub enum ResolvedAliasExportComponentInstance<'a> {
    /// Defined component instance for resolution
    Defined(ComponentRef<'a>, u32),
    /// Imported component instance
    Imported,
}
// =============================================================================
// Helper Methods on Component
// =============================================================================

impl<'a> Component<'a> {
    /// Look up an import by index and create a ResolvedImport.
    pub fn get_resolved_import(&self, import_idx: u32) -> ResolvedImport<'a> {
        let import = self
            .imports
            .get(import_idx as usize)
            .unwrap_or_else(|| panic!("Import index {} out of bounds", import_idx));
        ResolvedImport::Direct {
            name: import.name.0,
            ty: import.ty,
        }
    }

    /// Get a parent Component by count (1 = immediate parent, 2 = grandparent, etc.)
    /// Panics if the parent doesn't exist or is not a Component.
    fn get_parent_component(&self, count: u32) -> ComponentRef<'a> {
        if count == 0 {
            panic!("Outer alias count=0 is invalid (refers to current scope)");
        }
        let parent_scope = self.parents.get((count - 1) as usize).unwrap_or_else(|| {
            panic!(
                "Outer alias count={} but only {} parents available",
                count,
                self.parents.len()
            )
        });
        match parent_scope {
            ParentScope::Component(weak) => weak
                .upgrade()
                .expect("Parent component was dropped (Weak reference expired)"),
            ParentScope::ComponentType => {
                panic!("Cannot resolve Outer alias: parent is a ComponentType (not yet supported)")
            }
            ParentScope::InstanceType => {
                panic!("Cannot resolve Outer alias: parent is an InstanceType (not yet supported)")
            }
        }
    }

    /// Resolve an instance export alias by looking into the instantiated component.
    /// Returns Some(component_ref, export_index) if instance is defined, or None if
    /// it is imported.
    fn resolve_alias_export_component_instance(
        &self,
        instance_idx: u32,
        name: &str,
        expected_kind: ComponentExternalKind,
    ) -> ResolvedAliasExportComponentInstance<'a> {
        let instance = self
            .instances
            .get(instance_idx)
            .unwrap_or_else(|| panic!("Instance index {} out of bounds", instance_idx));

        match instance {
            ComponentInstanceNode::Instantiated { component_idx, .. } => {
                let nested_node = self
                    .components
                    .get(*component_idx)
                    .unwrap_or_else(|| panic!("Component index {} out of bounds", component_idx));
                if let ComponentNode::Defined { component } = nested_node {
                    let nested = component.borrow();
                    let export = nested
                        .exports
                        .iter()
                        .find(|e| e.name.0 == name)
                        .unwrap_or_else(|| {
                            panic!("Export '{}' not found in component {}", name, component_idx)
                        });
                    if export.kind != expected_kind {
                        panic!(
                            "Export '{}' has kind {:?}, expected {:?}",
                            name, export.kind, expected_kind
                        );
                    }
                    ResolvedAliasExportComponentInstance::Defined(component.clone(), export.index)
                } else {
                    panic!(
                        "Component {} is not defined inline (imported or aliased)",
                        component_idx
                    );
                }
            }
            ComponentInstanceNode::FromExports(exports) => {
                // Find the export with the matching name and kind
                let export = exports
                    .iter()
                    .find(|e| e.name.0 == name)
                    .unwrap_or_else(|| {
                        panic!(
                            "Export '{}' not found in FromExports instance {}",
                            name, instance_idx
                        )
                    });
                if export.kind != expected_kind {
                    panic!(
                        "Export '{}' has kind {:?}, expected {:?}",
                        name, export.kind, expected_kind
                    );
                }
                // For FromExports, we need to resolve the referenced index in *this* component
                // This is a bit tricky - we return a self-reference conceptually
                panic!(
                    "Cannot resolve InstanceExport through FromExports instance {} - not yet implemented",
                    instance_idx
                );
            }
            ComponentInstanceNode::Imported(_) => {
                // The instance is imported, so we cannot resolve the export to a defined field.
                ResolvedAliasExportComponentInstance::Imported
            }
            ComponentInstanceNode::Aliased(_) => {
                panic!(
                    "Cannot resolve InstanceExport alias through aliased instance {} - resolve the instance first",
                    instance_idx
                );
            }
            ComponentInstanceNode::Exported(export_idx) => {
                panic!(
                    "Cannot resolve InstanceExport alias through exported instance (export_idx {}) - resolve the instance first",
                    export_idx
                );
            }
        }
    }

    /// Resolve a core instance export to (module_idx, internal_index).
    /// Traces through the core instance to find the originating module.
    fn resolve_alias_export_core_instance(
        &self,
        instance_idx: u32,
        name: &str,
        expected_kind: ExternalKind,
    ) -> (u32, u32) {
        let resolved_instance = self.resolve_core_instance(instance_idx);

        match resolved_instance {
            ResolvedCoreInstance::Instantiated { module_idx, .. } => {
                // Look up the export in the module
                let module_node = self
                    .modules
                    .get(module_idx)
                    .unwrap_or_else(|| panic!("Module index {} out of bounds", module_idx));

                match module_node {
                    ModuleNode::Defined { module } => {
                        // Access module exports directly
                        let export = module
                            .exports
                            .iter()
                            .find(|e| e.name == name)
                            .unwrap_or_else(|| {
                                panic!("Export '{}' not found in module {}", name, module_idx)
                            });

                        // Verify the export kind matches (normalize FuncExact to Func)
                        let actual_kind = match export.kind {
                            ExternalKind::FuncExact => ExternalKind::Func,
                            other => other,
                        };
                        let normalized_expected = match expected_kind {
                            ExternalKind::FuncExact => ExternalKind::Func,
                            other => other,
                        };
                        if actual_kind != normalized_expected {
                            panic!(
                                "Export '{}' has kind {:?}, expected {:?}",
                                name, actual_kind, expected_kind
                            );
                        }

                        (module_idx, export.index)
                    }
                    ModuleNode::Imported(import_idx) => {
                        panic!(
                            "Cannot resolve core instance export through imported module (import_idx {}) at module_idx {}",
                            import_idx, module_idx
                        );
                    }
                    ModuleNode::Aliased(_) => {
                        panic!(
                            "Module {} is aliased but should have been resolved",
                            module_idx
                        );
                    }
                    ModuleNode::Exported(_) => {
                        panic!(
                            "Module {} is exported but should have been resolved",
                            module_idx
                        );
                    }
                }
            }
            ResolvedCoreInstance::FromExports(exports) => {
                // Find the matching export and resolve it recursively
                let export = exports.iter().find(|e| e.name == name).unwrap_or_else(|| {
                    panic!(
                        "Export '{}' not found in FromExports core instance {}",
                        name, instance_idx
                    )
                });

                if export.kind != expected_kind {
                    panic!(
                        "Export '{}' has kind {:?}, expected {:?}",
                        name, export.kind, expected_kind
                    );
                }

                // The export.index refers to an item in the corresponding core index space
                // We need to resolve that item to trace back to the module
                match expected_kind {
                    ExternalKind::Func | ExternalKind::FuncExact => {
                        let resolved = self.resolve_core_func(export.index);
                        match resolved {
                            ResolvedCoreFunc::FromModule {
                                module_idx,
                                func_idx,
                            } => (module_idx, func_idx),
                            ResolvedCoreFunc::Lowered { .. }
                            | ResolvedCoreFunc::ResourceDrop { .. }
                            | ResolvedCoreFunc::ResourceNew { .. }
                            | ResolvedCoreFunc::ResourceRep { .. } => {
                                panic!(
                                    "FromExports can only export a module func for instance {:?}",
                                    instance_idx
                                );
                            }
                        }
                    }
                    ExternalKind::Memory => {
                        let resolved = self.resolve_core_memory(export.index);
                        (resolved.module_idx, resolved.memory_idx)
                    }
                    ExternalKind::Table => {
                        let resolved = self.resolve_core_table(export.index);
                        (resolved.module_idx, resolved.table_idx)
                    }
                    ExternalKind::Global => {
                        let resolved = self.resolve_core_global(export.index);
                        (resolved.module_idx, resolved.global_idx)
                    }
                    ExternalKind::Tag => {
                        panic!("Tag resolution not yet implemented");
                    }
                }
            }
        }
    }
}

// =============================================================================
// Resolve Implementations
// =============================================================================

impl<'a> Resolve<'a> for ModuleNode<'a> {
    type Root = ResolvedModule<'a>;

    fn index_space<'b>(component: &'b Component<'a>) -> &'b IndexSpace<Self> {
        &component.modules
    }

    fn resolve(&self, component: &Component<'a>) -> Self::Root {
        match self {
            ModuleNode::Defined { module } => ResolvedModule::Defined {
                module: module.clone(),
            },
            ModuleNode::Imported(import_idx) => {
                ResolvedModule::Imported(component.get_resolved_import(*import_idx))
            }
            ModuleNode::Aliased(alias) => Self::follow_alias(component, alias),
            ModuleNode::Exported(export_idx) => {
                let export = &component.exports[*export_idx as usize];
                Self::resolve_index(component, export.index)
            }
        }
    }
}

impl<'a> ModuleNode<'a> {
    fn follow_alias(component: &Component<'a>, alias: &AliasInfo) -> ResolvedModule<'a> {
        match alias {
            AliasInfo::InstanceExport { instance_idx, name } => {
                let (nested_ref, export_idx) = match component
                    .resolve_alias_export_component_instance(
                        *instance_idx,
                        name,
                        ComponentExternalKind::Module,
                    ) {
                    ResolvedAliasExportComponentInstance::Defined(component_ref, export_idx) => {
                        (component_ref, export_idx)
                    }
                    ResolvedAliasExportComponentInstance::Imported => {
                        return ResolvedModule::Imported(ResolvedImport::Indirect);
                    }
                };
                let nested = nested_ref.borrow();
                nested.resolve_module(export_idx)
            }
            AliasInfo::Outer { count, index } => {
                let parent_ref = component.get_parent_component(*count);
                let parent = parent_ref.borrow();
                parent.resolve_module(*index)
            }
            AliasInfo::CoreInstanceExport { .. } => {
                panic!("Module cannot be aliased from CoreInstanceExport");
            }
        }
    }
}

impl<'a> Resolve<'a> for ComponentNode<'a> {
    type Root = ResolvedComponent<'a>;

    fn index_space<'b>(component: &'b Component<'a>) -> &'b IndexSpace<Self> {
        &component.components
    }

    fn resolve(&self, component: &Component<'a>) -> Self::Root {
        match self {
            ComponentNode::Defined { component: nested } => ResolvedComponent::Defined {
                component: nested.clone(),
            },
            ComponentNode::Imported(import_idx) => {
                ResolvedComponent::Imported(component.get_resolved_import(*import_idx))
            }
            ComponentNode::Aliased(alias) => Self::follow_alias(component, alias),
            ComponentNode::Exported(export_idx) => {
                let export = &component.exports[*export_idx as usize];
                Self::resolve_index(component, export.index)
            }
        }
    }
}

impl<'a> ComponentNode<'a> {
    fn follow_alias(component: &Component<'a>, alias: &AliasInfo) -> ResolvedComponent<'a> {
        match alias {
            AliasInfo::InstanceExport { instance_idx, name } => {
                let (nested_ref, export_idx) = match component
                    .resolve_alias_export_component_instance(
                        *instance_idx,
                        name,
                        ComponentExternalKind::Component,
                    ) {
                    ResolvedAliasExportComponentInstance::Defined(component_ref, export_idx) => {
                        (component_ref, export_idx)
                    }
                    ResolvedAliasExportComponentInstance::Imported => {
                        return ResolvedComponent::Imported(ResolvedImport::Indirect);
                    }
                };
                let nested = nested_ref.borrow();
                nested.resolve_component(export_idx)
            }
            AliasInfo::Outer { count, index } => {
                let parent_ref = component.get_parent_component(*count);
                let parent = parent_ref.borrow();
                parent.resolve_component(*index)
            }
            AliasInfo::CoreInstanceExport { .. } => {
                panic!("Component cannot be aliased from CoreInstanceExport");
            }
        }
    }
}

impl<'a> Resolve<'a> for ComponentInstanceNode<'a> {
    type Root = ResolvedComponentInstance<'a>;

    fn index_space<'b>(component: &'b Component<'a>) -> &'b IndexSpace<Self> {
        &component.instances
    }

    fn resolve(&self, component: &Component<'a>) -> Self::Root {
        match self {
            ComponentInstanceNode::Instantiated {
                component_idx,
                args,
            } => ResolvedComponentInstance::Instantiated {
                component_idx: *component_idx,
                args: args.clone(),
            },
            ComponentInstanceNode::FromExports(exports) => {
                ResolvedComponentInstance::FromExports(exports.clone())
            }
            ComponentInstanceNode::Imported(import_idx) => {
                ResolvedComponentInstance::Imported(component.get_resolved_import(*import_idx))
            }
            ComponentInstanceNode::Aliased(alias) => Self::follow_alias(component, alias),
            ComponentInstanceNode::Exported(export_idx) => {
                let export = &component.exports[*export_idx as usize];
                Self::resolve_index(component, export.index)
            }
        }
    }
}

impl<'a> ComponentInstanceNode<'a> {
    fn follow_alias(component: &Component<'a>, alias: &AliasInfo) -> ResolvedComponentInstance<'a> {
        match alias {
            AliasInfo::InstanceExport { instance_idx, name } => {
                let (nested_ref, export_idx) = match component
                    .resolve_alias_export_component_instance(
                        *instance_idx,
                        name,
                        ComponentExternalKind::Instance,
                    ) {
                    ResolvedAliasExportComponentInstance::Defined(component_ref, export_idx) => {
                        (component_ref, export_idx)
                    }
                    ResolvedAliasExportComponentInstance::Imported => {
                        return ResolvedComponentInstance::Imported(ResolvedImport::Indirect);
                    }
                };
                let nested = nested_ref.borrow();
                nested.resolve_component_instance(export_idx)
            }
            AliasInfo::Outer { count, index } => {
                let parent_ref = component.get_parent_component(*count);
                let parent = parent_ref.borrow();
                parent.resolve_component_instance(*index)
            }
            AliasInfo::CoreInstanceExport { .. } => {
                panic!("Component instance cannot be aliased from CoreInstanceExport");
            }
        }
    }
}

impl<'a> Resolve<'a> for ComponentFuncNode {
    type Root = ResolvedComponentFunc<'a>;

    fn index_space<'b>(component: &'b Component<'a>) -> &'b IndexSpace<Self> {
        &component.funcs
    }

    fn resolve(&self, component: &Component<'a>) -> Self::Root {
        match self {
            ComponentFuncNode::Lifted {
                core_func_idx,
                type_idx,
                options,
            } => ResolvedComponentFunc::Lifted {
                core_func_idx: *core_func_idx,
                type_idx: *type_idx,
                options: options.clone(),
            },
            ComponentFuncNode::Imported(import_idx) => {
                ResolvedComponentFunc::Imported(component.get_resolved_import(*import_idx))
            }
            ComponentFuncNode::Aliased(alias) => Self::follow_alias(component, alias),
            ComponentFuncNode::Exported(export_idx) => {
                let export = &component.exports[*export_idx as usize];
                Self::resolve_index(component, export.index)
            }
        }
    }
}

impl<'a> ComponentFuncNode {
    fn follow_alias(component: &Component<'a>, alias: &AliasInfo) -> ResolvedComponentFunc<'a> {
        match alias {
            AliasInfo::InstanceExport { instance_idx, name } => {
                let (nested_ref, export_idx) = match component
                    .resolve_alias_export_component_instance(
                        *instance_idx,
                        name,
                        ComponentExternalKind::Func,
                    ) {
                    ResolvedAliasExportComponentInstance::Defined(component_ref, export_idx) => {
                        (component_ref, export_idx)
                    }
                    ResolvedAliasExportComponentInstance::Imported => {
                        return ResolvedComponentFunc::Imported(ResolvedImport::Indirect);
                    }
                };
                let nested = nested_ref.borrow();
                nested.resolve_component_func(export_idx)
            }
            AliasInfo::Outer { count, index } => {
                let parent_ref = component.get_parent_component(*count);
                let parent = parent_ref.borrow();
                parent.resolve_component_func(*index)
            }
            AliasInfo::CoreInstanceExport { .. } => {
                panic!("Component func cannot be aliased from CoreInstanceExport");
            }
        }
    }
}

impl<'a> Resolve<'a> for ValueNode {
    type Root = ResolvedValue<'a>;

    fn index_space<'b>(component: &'b Component<'a>) -> &'b IndexSpace<Self> {
        &component.values
    }

    fn resolve(&self, component: &Component<'a>) -> Self::Root {
        match self {
            ValueNode::Imported(import_idx) => component.get_resolved_import(*import_idx),
            ValueNode::Aliased(alias) => Self::follow_alias(component, alias),
            ValueNode::Exported(export_idx) => {
                let export = &component.exports[*export_idx as usize];
                Self::resolve_index(component, export.index)
            }
        }
    }
}

impl<'a> ValueNode {
    fn follow_alias(component: &Component<'a>, alias: &AliasInfo) -> ResolvedValue<'a> {
        match alias {
            AliasInfo::InstanceExport { instance_idx, name } => {
                let (nested_ref, export_idx) = match component
                    .resolve_alias_export_component_instance(
                        *instance_idx,
                        name,
                        ComponentExternalKind::Value,
                    ) {
                    ResolvedAliasExportComponentInstance::Defined(component_ref, export_idx) => {
                        (component_ref, export_idx)
                    }
                    ResolvedAliasExportComponentInstance::Imported => {
                        return ResolvedValue::Indirect;
                    }
                };
                let nested = nested_ref.borrow();
                nested.resolve_value(export_idx)
            }
            AliasInfo::Outer { count, index } => {
                let parent_ref = component.get_parent_component(*count);
                let parent = parent_ref.borrow();
                parent.resolve_value(*index)
            }
            AliasInfo::CoreInstanceExport { .. } => {
                panic!("Value cannot be aliased from CoreInstanceExport");
            }
        }
    }
}

impl<'a> Resolve<'a> for TypeNode<'a> {
    type Root = ResolvedType<'a>;

    fn index_space<'b>(component: &'b Component<'a>) -> &'b IndexSpace<Self> {
        &component.types
    }

    fn resolve(&self, component: &Component<'a>) -> Self::Root {
        match self {
            TypeNode::Defined(def) => ResolvedType::Defined(def.clone()),
            TypeNode::Imported(import_idx) => {
                ResolvedType::Imported(component.get_resolved_import(*import_idx))
            }
            TypeNode::Aliased(alias) => Self::follow_alias(component, alias),
            TypeNode::Exported(export_idx) => {
                let export = &component.exports[*export_idx as usize];
                Self::resolve_index(component, export.index)
            }
        }
    }
}

impl<'a> TypeNode<'a> {
    fn follow_alias(component: &Component<'a>, alias: &AliasInfo) -> ResolvedType<'a> {
        match alias {
            AliasInfo::InstanceExport { instance_idx, name } => {
                let (nested_ref, export_idx) = match component
                    .resolve_alias_export_component_instance(
                        *instance_idx,
                        name,
                        ComponentExternalKind::Type,
                    ) {
                    ResolvedAliasExportComponentInstance::Defined(component_ref, export_idx) => {
                        (component_ref, export_idx)
                    }
                    ResolvedAliasExportComponentInstance::Imported => {
                        return ResolvedType::Imported(ResolvedImport::Indirect);
                    }
                };
                let nested = nested_ref.borrow();
                nested.resolve_type(export_idx)
            }
            AliasInfo::Outer { count, index } => {
                let parent_ref = component.get_parent_component(*count);
                let parent = parent_ref.borrow();
                parent.resolve_type(*index)
            }
            AliasInfo::CoreInstanceExport { .. } => {
                panic!("Type cannot be aliased from CoreInstanceExport");
            }
        }
    }
}

// =============================================================================
// Core Node Resolution
// =============================================================================

impl<'a> Resolve<'a> for CoreInstanceNode<'a> {
    type Root = ResolvedCoreInstance<'a>;

    fn index_space<'b>(component: &'b Component<'a>) -> &'b IndexSpace<Self> {
        &component.core_instances
    }

    fn resolve(&self, component: &Component<'a>) -> Self::Root {
        match self {
            CoreInstanceNode::Instantiated { module_idx, args } => {
                ResolvedCoreInstance::Instantiated {
                    module_idx: *module_idx,
                    args: args.clone(),
                }
            }
            CoreInstanceNode::FromExports(exports) => {
                ResolvedCoreInstance::FromExports(exports.clone())
            }
            CoreInstanceNode::Aliased(alias) => Self::follow_alias(component, alias),
        }
    }
}

impl<'a> CoreInstanceNode<'a> {
    fn follow_alias(component: &Component<'a>, alias: &AliasInfo) -> ResolvedCoreInstance<'a> {
        match alias {
            AliasInfo::Outer { count, index } => {
                let parent_ref = component.get_parent_component(*count);
                let parent = parent_ref.borrow();
                parent.resolve_core_instance(*index)
            }
            AliasInfo::InstanceExport { .. } => {
                panic!("Core instance cannot be aliased from component InstanceExport");
            }
            AliasInfo::CoreInstanceExport { .. } => {
                panic!("Core instance cannot be aliased from CoreInstanceExport");
            }
        }
    }
}

impl<'a> Resolve<'a> for CoreFuncNode {
    type Root = ResolvedCoreFunc;

    fn index_space<'b>(component: &'b Component<'a>) -> &'b IndexSpace<Self> {
        &component.core_funcs
    }

    fn resolve(&self, component: &Component<'a>) -> Self::Root {
        match self {
            CoreFuncNode::Lowered { func_idx, options } => ResolvedCoreFunc::Lowered {
                func_idx: *func_idx,
                options: options.clone(),
            },
            CoreFuncNode::ResourceDrop { resource } => ResolvedCoreFunc::ResourceDrop {
                resource: *resource,
            },
            CoreFuncNode::ResourceNew { resource } => ResolvedCoreFunc::ResourceNew {
                resource: *resource,
            },
            CoreFuncNode::ResourceRep { resource } => ResolvedCoreFunc::ResourceRep {
                resource: *resource,
            },
            CoreFuncNode::Aliased(alias) => Self::follow_alias(component, alias),
        }
    }
}

impl<'a> CoreFuncNode {
    fn follow_alias(component: &Component<'a>, alias: &AliasInfo) -> ResolvedCoreFunc {
        match alias {
            AliasInfo::CoreInstanceExport { instance_idx, name } => {
                let (module_idx, func_idx) = component.resolve_alias_export_core_instance(
                    *instance_idx,
                    name,
                    ExternalKind::Func,
                );
                ResolvedCoreFunc::FromModule {
                    module_idx,
                    func_idx,
                }
            }
            AliasInfo::Outer { count, index } => {
                let parent_ref = component.get_parent_component(*count);
                let parent = parent_ref.borrow();
                parent.resolve_core_func(*index)
            }
            AliasInfo::InstanceExport { .. } => {
                panic!("Core func cannot be aliased from component InstanceExport");
            }
        }
    }
}

impl<'a> Resolve<'a> for CoreMemoryNode {
    type Root = ResolvedCoreMemory;

    fn index_space<'b>(component: &'b Component<'a>) -> &'b IndexSpace<Self> {
        &component.core_memories
    }

    fn resolve(&self, component: &Component<'a>) -> Self::Root {
        match self {
            CoreMemoryNode::Aliased(alias) => Self::follow_alias(component, alias),
        }
    }
}

impl<'a> CoreMemoryNode {
    fn follow_alias(component: &Component<'a>, alias: &AliasInfo) -> ResolvedCoreMemory {
        match alias {
            AliasInfo::CoreInstanceExport { instance_idx, name } => {
                let (module_idx, memory_idx) = component.resolve_alias_export_core_instance(
                    *instance_idx,
                    name,
                    ExternalKind::Memory,
                );
                ResolvedCoreMemory {
                    module_idx,
                    memory_idx,
                }
            }
            AliasInfo::Outer { count, index } => {
                let parent_ref = component.get_parent_component(*count);
                let parent = parent_ref.borrow();
                parent.resolve_core_memory(*index)
            }
            AliasInfo::InstanceExport { .. } => {
                panic!("Core memory cannot be aliased from component InstanceExport");
            }
        }
    }
}

impl<'a> Resolve<'a> for CoreTableNode {
    type Root = ResolvedCoreTable;

    fn index_space<'b>(component: &'b Component<'a>) -> &'b IndexSpace<Self> {
        &component.core_tables
    }

    fn resolve(&self, component: &Component<'a>) -> Self::Root {
        match self {
            CoreTableNode::Aliased(alias) => Self::follow_alias(component, alias),
        }
    }
}

impl<'a> CoreTableNode {
    fn follow_alias(component: &Component<'a>, alias: &AliasInfo) -> ResolvedCoreTable {
        match alias {
            AliasInfo::CoreInstanceExport { instance_idx, name } => {
                let (module_idx, table_idx) = component.resolve_alias_export_core_instance(
                    *instance_idx,
                    name,
                    ExternalKind::Table,
                );
                ResolvedCoreTable {
                    module_idx,
                    table_idx,
                }
            }
            AliasInfo::Outer { count, index } => {
                let parent_ref = component.get_parent_component(*count);
                let parent = parent_ref.borrow();
                parent.resolve_core_table(*index)
            }
            AliasInfo::InstanceExport { .. } => {
                panic!("Core table cannot be aliased from component InstanceExport");
            }
        }
    }
}

impl<'a> Resolve<'a> for CoreGlobalNode {
    type Root = ResolvedCoreGlobal;

    fn index_space<'b>(component: &'b Component<'a>) -> &'b IndexSpace<Self> {
        &component.core_globals
    }

    fn resolve(&self, component: &Component<'a>) -> Self::Root {
        match self {
            CoreGlobalNode::Aliased(alias) => Self::follow_alias(component, alias),
        }
    }
}

impl<'a> CoreGlobalNode {
    fn follow_alias(component: &Component<'a>, alias: &AliasInfo) -> ResolvedCoreGlobal {
        match alias {
            AliasInfo::CoreInstanceExport { instance_idx, name } => {
                let (module_idx, global_idx) = component.resolve_alias_export_core_instance(
                    *instance_idx,
                    name,
                    ExternalKind::Global,
                );
                ResolvedCoreGlobal {
                    module_idx,
                    global_idx,
                }
            }
            AliasInfo::Outer { count, index } => {
                let parent_ref = component.get_parent_component(*count);
                let parent = parent_ref.borrow();
                parent.resolve_core_global(*index)
            }
            AliasInfo::InstanceExport { .. } => {
                panic!("Core global cannot be aliased from component InstanceExport");
            }
        }
    }
}

impl<'a> Resolve<'a> for CoreTypeNode<'a> {
    type Root = ResolvedCoreType<'a>;

    fn index_space<'b>(component: &'b Component<'a>) -> &'b IndexSpace<Self> {
        &component.core_types
    }

    fn resolve(&self, component: &Component<'a>) -> Self::Root {
        match self {
            CoreTypeNode::Defined(def) => ResolvedCoreType::Defined(def.clone()),
            CoreTypeNode::Aliased(alias) => Self::follow_alias(component, alias),
        }
    }
}

impl<'a> CoreTypeNode<'a> {
    fn follow_alias(component: &Component<'a>, alias: &AliasInfo) -> ResolvedCoreType<'a> {
        match alias {
            AliasInfo::Outer { count, index } => {
                let parent_ref = component.get_parent_component(*count);
                let parent = parent_ref.borrow();
                parent.resolve_core_type(*index)
            }
            AliasInfo::InstanceExport { .. } => {
                panic!("Core type cannot be aliased from component InstanceExport");
            }
            AliasInfo::CoreInstanceExport { .. } => {
                panic!("Core type cannot be aliased from CoreInstanceExport");
            }
        }
    }
}

// =============================================================================
// Convenience methods on Component
// =============================================================================

impl<'a> Component<'a> {
    /// Resolve a module by index.
    pub fn resolve_module(&self, idx: u32) -> ResolvedModule<'a> {
        ModuleNode::resolve_index(self, idx)
    }

    /// Resolve a component by index.
    pub fn resolve_component(&self, idx: u32) -> ResolvedComponent<'a> {
        ComponentNode::resolve_index(self, idx)
    }

    /// Resolve a component instance by index.
    pub fn resolve_component_instance(&self, idx: u32) -> ResolvedComponentInstance<'a> {
        ComponentInstanceNode::resolve_index(self, idx)
    }

    /// Resolve a component function by index.
    pub fn resolve_component_func(&self, idx: u32) -> ResolvedComponentFunc<'a> {
        ComponentFuncNode::resolve_index(self, idx)
    }

    /// Resolve a value by index.
    pub fn resolve_value(&self, idx: u32) -> ResolvedValue<'a> {
        ValueNode::resolve_index(self, idx)
    }

    /// Resolve a type by index.
    pub fn resolve_type(&self, idx: u32) -> ResolvedType<'a> {
        TypeNode::resolve_index(self, idx)
    }

    /// Resolve a core instance by index.
    pub fn resolve_core_instance(&self, idx: u32) -> ResolvedCoreInstance<'a> {
        CoreInstanceNode::resolve_index(self, idx)
    }

    /// Resolve a core function by index.
    pub fn resolve_core_func(&self, idx: u32) -> ResolvedCoreFunc {
        CoreFuncNode::resolve_index(self, idx)
    }

    /// Resolve a core memory by index.
    pub fn resolve_core_memory(&self, idx: u32) -> ResolvedCoreMemory {
        CoreMemoryNode::resolve_index(self, idx)
    }

    /// Resolve a core table by index.
    pub fn resolve_core_table(&self, idx: u32) -> ResolvedCoreTable {
        CoreTableNode::resolve_index(self, idx)
    }

    /// Resolve a core global by index.
    pub fn resolve_core_global(&self, idx: u32) -> ResolvedCoreGlobal {
        CoreGlobalNode::resolve_index(self, idx)
    }

    /// Resolve a core type by index.
    pub fn resolve_core_type(&self, idx: u32) -> ResolvedCoreType<'a> {
        CoreTypeNode::resolve_index(self, idx)
    }

    // =========================================================================
    // Utility: Get all dependencies of an instance
    // =========================================================================

    /// Returns the module index and all argument indices for an instantiated core instance.
    pub fn core_instance_dependencies(&self, idx: u32) -> Option<(u32, Vec<u32>)> {
        match self.core_instances.get(idx)? {
            CoreInstanceNode::Instantiated { module_idx, args } => {
                let arg_indices: Vec<u32> = args.iter().map(|a| a.index).collect();
                Some((*module_idx, arg_indices))
            }
            _ => None,
        }
    }
}
