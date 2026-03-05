//! This is the counterpart to `linking.rs` for component linking metadata
//!
//! This metadata is a part of LinkingMetadata but primarily to pass data
//! across the nested component hierarchy
use std::collections::HashMap;
use std::ops::Deref;

use crate::InstantiateOrder;
use component_tools::ir::ComponentRef;

/// Unique index provided to each nested component in a component
///
/// This is the same order as the declared order in the component
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash)]
pub struct ComponentID(pub u32);
impl Deref for ComponentID {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Unique index provided to each component instance in a component (top-level, not nested)
///
/// Note this includes both instantiated and non-instantiated instances
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ComponentInstanceID(pub u32);
impl Deref for ComponentInstanceID {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
/// Index for core imports within a component's IR.
pub struct ComponentImportIndex(pub u32);
impl std::ops::Deref for ComponentImportIndex {
    type Target = u32;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Metadata associated with a component that is instantiated in the component
#[derive(Debug)]
pub struct ComponentMetadata<'a> {
    /// The component
    pub component: ComponentRef<'a>,
    /// Map of its imports (to prevent re-computation when it is instantiated multiple times)
    pub import_map: HashMap<String, ComponentImportIndex>,
}

/// The index of the node in the parent that binds this specific component import
///
/// Only func supported for now
#[derive(Debug, Clone)]
pub enum ComponentImportBindInParent {
    Func(u32),
}

/// Metadata needed to capture the linking information for a component
#[derive(Debug)]
pub struct ComponentInstantiationLinkingMetadata {
    /// The order in which this component should be instantiated w.r.t other components
    #[allow(
        dead_code,
        reason = "for single component instantiations, ordering is unnecessary"
    )]
    pub instantiate_order: InstantiateOrder,
    /// Metadata capturing all the import linking information for a component instantiation.
    ///
    /// Every import in the component being instantiated must have a bindings to a ComponentImportBind.
    /// The node needs to be resolved with respect to the component's parent.
    pub imports: HashMap<String, ComponentImportBindInParent>,
}

/// Metadata needed to capture linking information about nested component instances of a component.
/// This only considers top-level nesting, not fully recursive.
///
/// Counterpart of `LinkingMetadata` just for component instances.
#[derive(Debug, Default)]
pub struct ComponentLinkingMetadata<'a> {
    /// The 'static' metadata for each nested component in the component
    pub cm: HashMap<ComponentID, ComponentMetadata<'a>>,
    /// A reverse mapping from instances to components
    pub instance_map: HashMap<ComponentInstanceID, ComponentID>,
    /// The instance linking information for each component instantiation in the commponent
    pub instantiations: HashMap<ComponentInstanceID, ComponentInstantiationLinkingMetadata>,
}
