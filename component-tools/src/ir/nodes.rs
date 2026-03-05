//! Node type definitions for each index space.
//!
//! Each node type has specialized variants reflecting how items
//! in that index space can be introduced (Import, Alias, Definition, etc.)

use wirm::Module;

// Re-export wasmparser types we use in our API
use super::ComponentRef;
pub use wirm::wasmparser::{
    CanonicalOption, ComponentExport, ComponentExternalKind, ComponentInstantiationArg,
    ComponentType, ComponentTypeRef, CoreType, Export, ExternalKind, InstantiationArg,
};

// =============================================================================
// Common Info Types
// =============================================================================

/// Information about an alias.
#[derive(Debug, Clone)]
pub enum AliasInfo {
    /// Alias to an export of a component instance
    InstanceExport { instance_idx: u32, name: String },
    /// Alias to an export of a core instance
    CoreInstanceExport { instance_idx: u32, name: String },
    /// Alias to an outer scope item
    Outer { count: u32, index: u32 },
}

// =============================================================================
// Component-Level Nodes
// =============================================================================

/// A core WebAssembly module in the module index space.
#[derive(Debug, Clone)]
pub enum ModuleNode<'a> {
    /// Imported module - index into component's imports vector
    Imported(u32),
    Aliased(AliasInfo),
    Defined {
        /// Parsed module IR from wirm
        module: Module<'a>,
    },
    /// Exported - index into component's exports vector
    Exported(u32),
}

/// A nested component in the component index space.
#[derive(Debug, Clone)]
pub enum ComponentNode<'a> {
    /// Imported component - index into component's imports vector
    Imported(u32),
    Aliased(AliasInfo),
    Defined {
        /// Recursively parsed component (Rc<RefCell> for shared access and parent chain setup)
        component: ComponentRef<'a>,
    },
    /// Exported - index into component's exports vector
    Exported(u32),
}

/// A component instance in the instance index space.
#[derive(Debug, Clone)]
pub enum ComponentInstanceNode<'a> {
    /// Imported instance - index into component's imports vector
    Imported(u32),
    Aliased(AliasInfo),
    /// Created by instantiating a component
    Instantiated {
        component_idx: u32,
        args: Vec<ComponentInstantiationArg<'a>>,
    },
    /// Created inline from a list of exports
    FromExports(Vec<ComponentExport<'a>>),
    /// Exported - index into component's exports vector
    Exported(u32),
}

/// A component function in the func index space.
#[derive(Debug, Clone)]
pub enum ComponentFuncNode {
    /// Imported function - index into component's imports vector
    Imported(u32),
    Aliased(AliasInfo),
    /// Created by `canon lift`
    Lifted {
        core_func_idx: u32,
        type_idx: u32,
        options: Vec<CanonicalOption>,
    },
    /// Exported - index into component's exports vector
    Exported(u32),
}

/// A value in the value index space.
#[derive(Debug, Clone)]
pub enum ValueNode {
    /// Imported value - index into component's imports vector
    Imported(u32),
    Aliased(AliasInfo),
    /// Exported - index into component's exports vector
    Exported(u32),
}

/// A type in the type index space.
#[derive(Debug, Clone)]
pub enum TypeNode<'a> {
    /// Component type defined inline (from wasmparser)
    Defined(ComponentType<'a>),
    Aliased(AliasInfo),
    /// Imported type - index into component's imports vector
    Imported(u32),
    /// Exported - index into component's exports vector
    Exported(u32),
}

// =============================================================================
// Core-Level Nodes (visible at component level)
// =============================================================================

/// A core instance in the core instance index space.
#[derive(Debug, Clone)]
pub enum CoreInstanceNode<'a> {
    Aliased(AliasInfo),
    /// Created by instantiating a module
    Instantiated {
        module_idx: u32,
        args: Vec<InstantiationArg<'a>>,
    },
    /// Created inline from exports
    FromExports(Vec<Export<'a>>),
}

/// A core function in the core func index space.
#[derive(Debug, Clone)]
pub enum CoreFuncNode {
    Aliased(AliasInfo),
    /// Created by `canon lower`
    Lowered {
        func_idx: u32,
        options: Vec<CanonicalOption>,
    },
    /// Created by `canon resource.drop`
    ResourceDrop {
        resource: u32,
    },
    /// Created by `canon resource.new`
    ResourceNew {
        resource: u32,
    },
    /// Created by `canon resource.rep`
    ResourceRep {
        resource: u32,
    },
}

/// A core memory in the core memory index space.
#[derive(Debug, Clone)]
pub enum CoreMemoryNode {
    Aliased(AliasInfo),
}

/// A core table in the core table index space.
#[derive(Debug, Clone)]
pub enum CoreTableNode {
    Aliased(AliasInfo),
}

/// A core global in the core global index space.
#[derive(Debug, Clone)]
pub enum CoreGlobalNode {
    Aliased(AliasInfo),
}

/// A core type in the core type index space.
#[derive(Debug, Clone)]
pub enum CoreTypeNode<'a> {
    Aliased(AliasInfo),
    /// Defined inline (from wasmparser)
    Defined(CoreType<'a>),
}

// =============================================================================
// Resolved Types (without Aliased variants)
// =============================================================================

/// A resolved import with name and type reference.
#[derive(Debug, Clone)]
pub enum ResolvedImport<'a> {
    /// This import type is direct defined in the space it was resolved
    Direct {
        /// The import name
        name: &'a str,
        /// The type reference (Module, Func, Value, Type, Instance, or Component)
        ty: ComponentTypeRef,
    },
    /// This type is within another type (e.g. within an instance type or a component type)
    Indirect,
}

/// Resolved module - either imported or defined inline.
#[derive(Debug, Clone)]
pub enum ResolvedModule<'a> {
    Imported(ResolvedImport<'a>),
    /// A defined module with its parsed IR.
    Defined {
        module: Module<'a>,
    },
}

impl<'a> ResolvedModule<'a> {
    pub fn defined(self) -> Module<'a> {
        match self {
            Self::Imported(_) => panic!("Expected defined module, found imported"),
            Self::Defined { module } => module,
        }
    }
}

/// Resolved component - either imported or defined inline.
#[derive(Debug)]
pub enum ResolvedComponent<'a> {
    Imported(ResolvedImport<'a>),
    Defined { component: ComponentRef<'a> },
}

impl<'a> Clone for ResolvedComponent<'a> {
    fn clone(&self) -> Self {
        match self {
            Self::Imported(import) => Self::Imported(import.clone()),
            Self::Defined { component } => Self::Defined {
                component: component.clone(),
            },
        }
    }
}

impl<'a> ResolvedComponent<'a> {
    pub fn defined(self) -> ComponentRef<'a> {
        match self {
            Self::Imported(_) => panic!("Expected defined component, found imported"),
            Self::Defined { component } => component,
        }
    }
}

/// Resolved component instance.
#[derive(Debug, Clone)]
pub enum ResolvedComponentInstance<'a> {
    Imported(ResolvedImport<'a>),
    Instantiated {
        component_idx: u32,
        args: Vec<ComponentInstantiationArg<'a>>,
    },
    FromExports(Vec<ComponentExport<'a>>),
}

/// Resolved component function.
#[derive(Debug, Clone)]
pub enum ResolvedComponentFunc<'a> {
    Imported(ResolvedImport<'a>),
    Lifted {
        core_func_idx: u32,
        type_idx: u32,
        options: Vec<CanonicalOption>,
    },
}

/// Resolved value - only imported (values have no other definition form).
pub type ResolvedValue<'a> = ResolvedImport<'a>;

/// Resolved component type.
#[derive(Debug, Clone)]
pub enum ResolvedType<'a> {
    Imported(ResolvedImport<'a>),
    Defined(ComponentType<'a>),
}

/// Resolved core instance.
#[derive(Debug, Clone)]
pub enum ResolvedCoreInstance<'a> {
    Instantiated {
        module_idx: u32,
        args: Vec<InstantiationArg<'a>>,
    },
    FromExports(Vec<Export<'a>>),
}

/// Resolved core function - either lowered or from a module.
#[derive(Debug, Clone)]
pub enum ResolvedCoreFunc {
    /// Created by `canon lower`
    Lowered {
        func_idx: u32,
        options: Vec<CanonicalOption>,
    },
    /// Created by `canon resource.drop`
    ResourceDrop { resource: u32 },
    /// Created by `canon resource.new`
    ResourceNew { resource: u32 },
    /// Created by `canon resource.rep`
    ResourceRep { resource: u32 },
    /// From a module's export (traced through core instance)
    FromModule { module_idx: u32, func_idx: u32 },
}

/// Resolved core memory - always from a module.
#[derive(Debug, Clone)]
pub struct ResolvedCoreMemory {
    pub module_idx: u32,
    pub memory_idx: u32,
}

/// Resolved core table - always from a module.
#[derive(Debug, Clone)]
pub struct ResolvedCoreTable {
    pub module_idx: u32,
    pub table_idx: u32,
}

/// Resolved core global - always from a module.
#[derive(Debug, Clone)]
pub struct ResolvedCoreGlobal {
    pub module_idx: u32,
    pub global_idx: u32,
}

/// Resolved core type.
#[derive(Debug, Clone)]
pub enum ResolvedCoreType<'a> {
    Defined(CoreType<'a>),
}
