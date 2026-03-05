//! IR definitions for the Component Model index spaces.

mod index_space;
mod nodes;
mod resolve;

pub use index_space::IndexSpace;
pub use nodes::*;
pub use resolve::Resolve;

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use wirm::wasmparser::ComponentImport;

/// Shared reference to a Component (Rc + RefCell for interior mutability).
pub type ComponentRef<'a> = Rc<RefCell<Component<'a>>>;
/// Weak reference to a Component for parent chains.
pub type ComponentWeak<'a> = Weak<RefCell<Component<'a>>>;

/// A parent scope for Outer alias resolution.
/// In the Component Model, parent scopes can be Components, ComponentTypes, or InstanceTypes.
#[derive(Debug, Clone)]
pub enum ParentScope<'a> {
    /// Parent is a Component
    Component(ComponentWeak<'a>),
    /// Parent is a ComponentType definition (placeholder - has its own index spaces)
    ComponentType,
    /// Parent is an InstanceType definition (placeholder - has its own index spaces)
    InstanceType,
}

/// A parsed WebAssembly Component with all 12 index spaces accessible.
#[derive(Debug)]
pub struct Component<'a> {
    /// Parent scopes for Outer alias resolution.
    /// Ordered innermost to outermost: parents[0] is the immediate parent.
    pub parents: Vec<ParentScope<'a>>,

    /// All imports in order they appear (name + type reference from wasmparser).
    pub imports: Vec<ComponentImport<'a>>,

    // Component-level index spaces
    pub modules: IndexSpace<ModuleNode<'a>>,
    pub components: IndexSpace<ComponentNode<'a>>,
    pub instances: IndexSpace<ComponentInstanceNode<'a>>,
    pub funcs: IndexSpace<ComponentFuncNode>,
    pub values: IndexSpace<ValueNode>,
    pub types: IndexSpace<TypeNode<'a>>,

    // Core-level index spaces (visible to the component)
    pub core_instances: IndexSpace<CoreInstanceNode<'a>>,
    pub core_funcs: IndexSpace<CoreFuncNode>,
    pub core_memories: IndexSpace<CoreMemoryNode>,
    pub core_tables: IndexSpace<CoreTableNode>,
    pub core_globals: IndexSpace<CoreGlobalNode>,
    pub core_types: IndexSpace<CoreTypeNode<'a>>,

    // Exports
    pub exports: Vec<ComponentExport<'a>>,
}

impl<'a> Default for Component<'a> {
    fn default() -> Self {
        Self {
            parents: Vec::new(),
            imports: Vec::new(),
            modules: IndexSpace::default(),
            components: IndexSpace::default(),
            instances: IndexSpace::default(),
            funcs: IndexSpace::default(),
            values: IndexSpace::default(),
            types: IndexSpace::default(),
            core_instances: IndexSpace::default(),
            core_funcs: IndexSpace::default(),
            core_memories: IndexSpace::default(),
            core_tables: IndexSpace::default(),
            core_globals: IndexSpace::default(),
            core_types: IndexSpace::default(),
            exports: Vec::new(),
        }
    }
}
