use anyhow::{Error, Result, anyhow, bail};
use std::collections::HashMap;
use std::ops::Deref;

use decomposer::wasmparser::TypeRef;
use decomposer::wirm::Module;
use decomposer::wirm::ir::id::{ImportsID as WirmImportsID, TypeID as WirmTypeID};
use decomposer::wirm::ir::module::module_types::Types;
use serde::Serialize;

use crate::glue::{GLUE_MODULE_NAME, GlueBuilder};

/// Unified naming of instances from IDs
pub fn module_name_from_ids(module_id: ModuleID, instance_id: ModuleInstanceID) -> String {
    format!("module{}_instance{}", module_id.0, instance_id.0)
}

/// Stub function to highlight locations where assumptions of single instantiation per module are made.
pub fn assumed_instance_id(
    instance_map: &HashMap<ModuleInstanceID, ModuleID>,
    module_id: ModuleID,
) -> ModuleInstanceID {
    *instance_map
        .iter()
        .find(|(_, mid)| **mid == module_id)
        .unwrap()
        .0
}

pub type Checksum = [u8; 32];

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
/// Index for core imports within a module's IR.
pub struct ModuleImportIndex(pub u32);
impl std::ops::Deref for ModuleImportIndex {
    type Target = u32;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl From<WirmImportsID> for ModuleImportIndex {
    fn from(id: WirmImportsID) -> Self {
        Self(id.0)
    }
}

#[derive(Debug)]
/// Metadata associated with a module that is instantiated in the component
pub struct ModuleMetadata<'a> {
    /// The module
    pub module: Module<'a>,
    /// Map of its imports (to prevent re-computation when it is instantiated multiple times)
    pub import_map: HashMap<String, HashMap<String, ModuleImportIndex>>,
}

/// Unique index provided to each module for [`LinkingMetadata`]
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ModuleID(pub u32);
impl Deref for ModuleID {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Unique index provided to each module instance for [`LinkingMetadata`]
///
/// Note this includes both instantiated and non-instantiated instances
#[derive(Debug, Copy, Clone, Serialize, PartialEq, Eq, Hash)]
pub struct ModuleInstanceID(pub u32);
impl Deref for ModuleInstanceID {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub type InstantiateOrder = u32;
/// Index type to identify a particular export from a module instance.
///
/// Index into [`LinkingMetadata::instantiations`] (with to identify a particular instance
#[derive(Debug, Clone, Serialize)]
pub struct ModuleInstanceExport {
    pub mid: ModuleInstanceID,
    pub name: String,
}

/// The export index as assigned by the recorder in RR.
#[derive(Debug, Clone, PartialEq, Eq, Copy, Hash, Serialize)]
pub struct RecordExportIndex(pub u32);

/// Index for canonical options adapters within a module's IR.
///
/// We specifically identify instances as opposed to just modules because if a module
/// is instantiated multiple times, this points to only one specific instances of adapter functions
#[derive(Debug, Default, Serialize, Clone)]
pub struct CanonicalOptionsIndex {
    pub memory: Option<ModuleInstanceExport>,
    pub realloc: Option<ModuleInstanceExport>,
    pub post_return: Option<ModuleInstanceExport>,
}

#[derive(Debug, Clone)]
/// The kind of a resolved imports when linked within the component
pub enum ImportKind {
    /// These are 'real' imports that are not linked into from sister instances.
    /// For canon lowers, the args are provided in the optional canonical options.
    TrueImport(Option<CanonicalOptionsIndex>),
    /// The IDs for the imports in this module's instance that are builtins (e.g. from canonical options)
    Builtin,
    /// Renames for import with the target module's instance(package) and member name that it must be linked to
    Rename {
        package: ModuleInstanceID,
        member: String,
    },
}

/// Metadata needed to capture the linking information for a module for CRIMP replay custom section
#[derive(Debug)]
pub struct InstantiationLinkingMetadata {
    /// The order in which this module should be instantiated w.r.t other modules
    pub instantiate_order: InstantiateOrder,
    /// Metadata capturing all the import linking information for a module instantiation.
    /// Every import ID in the module being instantiated must have a mapping to an ImportKind in this struct.
    pub imports: HashMap<ModuleImportIndex, ImportKind>,
}

#[derive(Debug, Serialize)]
/// Metadata to identify core functions being exported.
pub struct ExportFuncMetadata {
    pub name: String,
    /// ID, as assigned to this export by the CRIMP recorder.
    pub record_id: RecordExportIndex,
    pub opts: Option<CanonicalOptionsIndex>,
}

/// Metadata needed to capture complete linking information for a component for CRIMP replay custom section
#[derive(Debug, Default)]
pub struct LinkingMetadata<'a> {
    /// The checksum of the component for which this linking metadata is generated
    pub checksum: Checksum,
    /// The 'static' metadata for each module in the component
    pub mm: HashMap<ModuleID, ModuleMetadata<'a>>,
    /// A reverse mapping from instances to modules
    pub instance_map: HashMap<ModuleInstanceID, ModuleID>,
    /// The instance linking information for each module instantiation in the commponent
    pub instantiations: HashMap<ModuleInstanceID, InstantiationLinkingMetadata>,
    /// The exported functions from this component arranged by the instance they are sourced from.
    pub export_funcs: HashMap<ModuleInstanceID, Vec<ExportFuncMetadata>>,
}

#[derive(Debug, Serialize, Clone)]
/// Information about canonical adapters (just Lower for now) for imports in the custom section
struct ImportAdapterCrimpData {
    /// The import index that this adapter is for
    target: ModuleImportIndex,
    /// The memory to use for adapter
    memory: Option<ModuleInstanceExport>,
    /// The realloc to use for adapter
    realloc: Option<ModuleInstanceExport>,
    /// Whether this import is a builtin
    is_builtin: bool,
}

#[derive(Debug, Serialize)]
/// The CRIMP replay custom section serializable data
struct CrimpSectionData<'a> {
    checksum: Checksum,
    instance_id: ModuleInstanceID,
    instantiate_order: InstantiateOrder,
    import_adapters: Vec<ImportAdapterCrimpData>,
    exports: Vec<&'a ExportFuncMetadata>,
}

impl<'a> LinkingMetadata<'a> {
    pub fn module_id(&self, instance_id: ModuleInstanceID) -> ModuleID {
        self.instance_map[&instance_id]
    }

    pub fn module(&self, instance_id: ModuleInstanceID) -> &Module<'a> {
        let module_id = self.module_id(instance_id);
        &self.mm[&module_id].module
    }

    /// Construct the adapted module (with imports and module renamed) from the instance, and get its adapter data
    fn construct_adapted_module(
        &self,
        instance_id: ModuleInstanceID,
    ) -> (Module<'a>, Vec<ImportAdapterCrimpData>) {
        let assigned_name =
            |instance_id| module_name_from_ids(self.module_id(instance_id), instance_id);

        // Module wiring
        let mut module = self.module(instance_id).clone();
        module.module_name = Some(assigned_name(instance_id));
        let mut populated = vec![false; module.imports.len()];
        let mut import_adapters = vec![];
        let imports = &self.instantiations[&instance_id].imports;
        for (idx, import_kind) in imports {
            populated[**idx as usize] = true;
            match import_kind {
                ImportKind::Builtin => {
                    let wirm_idx = WirmImportsID(**idx);
                    // Engine will stub with the replay result
                    // Just provide a nice readable name
                    module.imports.set_import_name(
                        "crimp_replay".into(),
                        module.imports.get(wirm_idx).name.to_string(),
                        wirm_idx,
                    );
                    import_adapters.push(ImportAdapterCrimpData {
                        target: *idx,
                        memory: None,
                        realloc: None,
                        is_builtin: true,
                    });
                }
                ImportKind::TrueImport(opts) => {
                    let wirm_idx = WirmImportsID(**idx);
                    // Engine will stub with the replay result
                    // Just provide a nice readable name
                    module.imports.set_import_name(
                        "crimp_replay".into(),
                        module.imports.get(wirm_idx).name.to_string(),
                        wirm_idx,
                    );
                    let (mut memory, mut realloc) = (None, None);
                    if let Some(opts) = opts {
                        assert!(
                            opts.post_return.is_none(),
                            "Post return should never be present for module imports"
                        );
                        memory = opts.memory.clone();
                        realloc = opts.realloc.clone();
                    };
                    // When memory and realloc are always set together, if present
                    import_adapters.push(ImportAdapterCrimpData {
                        target: *idx,
                        memory,
                        realloc,
                        is_builtin: false,
                    });
                }
                ImportKind::Rename { package, member } => {
                    module.imports.set_import_name(
                        assigned_name(*package),
                        member.clone(),
                        WirmImportsID(**idx),
                    );
                }
            }
        }
        assert!(
            populated.iter().all(|b| *b),
            "Not all imports were populated for instance {:?} | {:?}",
            instance_id,
            populated
        );
        (module, import_adapters)
    }

    /// Adapt the module and add bindings for the instance to the glue
    pub fn adapt_and_update_glue(
        &self,
        instance_id: ModuleInstanceID,
        glue: &mut GlueBuilder<'a>,
    ) -> Result<Module<'a>> {
        let (mut module, import_adapters) = self.construct_adapted_module(instance_id);
        let instance_name = module_name_from_ids(self.module_id(instance_id), instance_id);

        // For each import adapter (TrueImport/Builtin), rename from "crimp-replay" to "crimp_glue"
        // with a namespaced name, and add a corresponding replay stub to the glue module.
        for adapter in &import_adapters {
            let wirm_idx = WirmImportsID(*adapter.target);

            // Extract data from the import before mutating
            let import = module.imports.get(wirm_idx);
            let original_name = import.name.to_string();
            let type_idx = match import.ty {
                TypeRef::Func(u) => u,
                _ => bail!(
                    "Expected function import at index {:?}, got {:?}",
                    adapter.target,
                    import.ty
                ),
            };

            // Rename this import to point to the glue module
            let namespaced_name = format!("{}:{}", instance_name, original_name);
            module.imports.set_import_name(
                GLUE_MODULE_NAME.into(),
                namespaced_name.clone(),
                wirm_idx,
            );

            // Get the signature of the import
            let func_type = module
                .types
                .get(WirmTypeID(type_idx))
                .ok_or_else(|| anyhow!("Type {} not found in module", type_idx))?;
            let (params, results) = match func_type {
                Types::FuncType {
                    params, results, ..
                } => (params.to_vec(), results.to_vec()),
                _ => bail!("Expected FuncType at index {}", type_idx),
            };

            glue.add_replay_stub(&namespaced_name, &params, &results, adapter.is_builtin);
        }

        // Register component export functions for dispatch from the driver
        if let Some(export_funcs) = self.export_funcs.get(&instance_id) {
            for export_func in export_funcs {
                glue.register_export(export_func, instance_id, self)?;
            }
        }

        Ok(module)
    }

    /// Adapt and serialize required linking information into the crimp custom section for a single module's instance
    pub fn adapt_and_serialize_crimp_section(
        &self,
        instance_id: ModuleInstanceID,
    ) -> Result<(Module<'a>, Vec<u8>)> {
        let (module, import_adapters) = self.construct_adapted_module(instance_id);
        // Custom section
        let empty = vec![];
        let data = CrimpSectionData {
            checksum: self.checksum,
            instance_id,
            instantiate_order: self.instantiations[&instance_id].instantiate_order,
            import_adapters,
            exports: self
                .export_funcs
                .get(&instance_id)
                .unwrap_or(&empty)
                .iter()
                .collect(),
        };
        let section = postcard::to_stdvec(&data).map_err(Into::<Error>::into)?;
        Ok((module, section))
    }
}
