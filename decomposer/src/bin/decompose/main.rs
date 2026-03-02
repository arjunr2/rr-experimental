//! CLI tool to decompose a WebAssembly Component into its constituent modules.

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, ValueEnum};
use decomposer::wasmparser::{
    CanonicalOption, ComponentExternalKind, ExternalKind, InstantiationArgKind, Validator,
};
use decomposer::wirm::ir::module::{GetID, LocalOrImport};
use env_logger;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::{fs, vec};

use decomposer::Component;
use decomposer::ir::{
    ComponentInstanceNode, CoreInstanceNode, Resolve, ResolvedComponent,
    ResolvedComponentFunc, ResolvedComponentInstance, ResolvedCoreFunc, ResolvedCoreInstance,
    ResolvedImport, ResolvedModule,
};
use decomposer::parse_component;
use decomposer::wirm::Module;
use decomposer::wirm::ir::id::FunctionID;
use decomposer::wirm::ir::module::module_exports::Export;
use decomposer::wirm::ir::types::CustomSection;

mod linking;
use linking::*;

mod component_linking;
use component_linking::*;

mod glue;
use glue::*;

macro_rules! unsupported {
    // Single argument: unconditional error
    ($feature:expr) => {
        Err(anyhow!("'{}' is not supported yet...", $feature))
    };
    // Two arguments: conditional error (returns Err if condition is true)
    ($cond:expr, $feature:expr) => {
        if $cond {
            Err(anyhow!("'{}' is not supported yet...", $feature))
        } else {
            Ok(())
        }
    };
}

/// CLI options for how to merge modules
#[derive(ValueEnum, Copy, Clone, Debug, Default)]
enum MergeOptions {
    /// No merging. Decomposed modules, glue, and driver are all kept separate.
    #[default]
    FullSplit,
    /// All decomposed modules from the component and the glue + driver
    /// are combined into a single Wasm module.
    FullMerge,
    /// Decompose modules are merged together and glue + driver are merged together.
    /// The two merged outputs are kept separate.
    DriverSplit,
}

impl MergeOptions {
    /// Whether any merging should be done at all.
    fn any_merge(&self) -> bool {
        !matches!(self, MergeOptions::FullSplit)
    }
}

#[derive(Parser, Debug)]
#[command(name = "decompose")]
#[command(about = "Decompose a WebAssembly Component into its modules")]
struct CLI {
    /// Input component file to decompose.
    #[arg(short, long)]
    component: PathBuf,
    /// Whether to generate output in WAT formatted output as well.
    #[arg(short = 't', long = "wat")]
    wat: bool,
    /// Overwrite the output directory if it exists.
    #[arg(short = 'x', long = "overwrite")]
    overwrite: bool,
    /// Merging technique on the decomposed components with `wasm-merge` on output
    #[arg(short, long)]
    merge: MergeOptions,
    /// When `glue` is false, writes necessary information into a custom section for replay, and relies on
    /// engine support to read this information and drive the replay accordingly.
    ///
    /// When `glue` is true, we generate a glue and replay driver Wasm module, enabling one to run
    /// the replay completely as Wasm without special engine support
    #[arg(short, long, default_value_t = false)]
    glue: bool,
    #[command(flatten)]
    glue_args: GlueArgs,
    /// Output directory for decomposed modules from component.
    #[arg(short, long)]
    outdir: PathBuf,
}

#[derive(Debug, Copy, Clone)]
/// The kind of instance that was created
enum InstanceKind {
    FromExports,
    FromInstantiated(ModuleID),
}

#[derive(Debug, Clone)]
/// Interface for exports from an instance that are linked as args during instantiation
struct InstanceExportInterface {
    exports: Vec<Export>,
    kind: InstanceKind,
}

impl CanonicalOptionsIndex {
    /// Indexes the options for a canonical function within the module's IR
    pub fn from_options<'a>(
        component: &Component<'a>,
        options: &[CanonicalOption],
        instance_map: &HashMap<ModuleInstanceID, ModuleID>,
    ) -> Option<Self> {
        fn func_resolve<'a>(
            component: &Component<'a>,
            func_idx: u32,
            instance_map: &HashMap<ModuleInstanceID, ModuleID>,
        ) -> Option<ModuleInstanceExport> {
            match component.resolve_core_func(func_idx) {
                ResolvedCoreFunc::FromModule {
                    module_idx,
                    func_idx,
                } => {
                    let module_id = ModuleID(module_idx);
                    let export_name = get_export_name_from_kind_idx(
                        component,
                        module_idx,
                        vec![ExternalKind::Func, ExternalKind::FuncExact],
                        func_idx,
                    );
                    Some(ModuleInstanceExport {
                        mid: assumed_instance_id(instance_map, module_id),
                        name: export_name,
                    })
                }
                _ => panic!("Canonical options core references can only come FromModule"),
            }
        }

        let mut opts_ref = CanonicalOptionsIndex::default();
        for opt in options {
            match opt {
                //CanonicalOption::Memory(memory_idx) => opts_ref.memory = Some(name.clone()),
                CanonicalOption::Realloc(func_idx) => {
                    opts_ref.realloc = func_resolve(component, *func_idx, instance_map);
                }
                CanonicalOption::PostReturn(func_idx) => {
                    opts_ref.post_return = func_resolve(component, *func_idx, instance_map);
                }
                CanonicalOption::Memory(memory_idx) => {
                    let memory = component.resolve_core_memory(*memory_idx);
                    let module_id = ModuleID(memory.module_idx);
                    opts_ref.memory = Some(ModuleInstanceExport {
                        mid: assumed_instance_id(instance_map, module_id),
                        name: get_export_name_from_kind_idx(
                            component,
                            memory.module_idx,
                            vec![ExternalKind::Memory],
                            memory.memory_idx,
                        ),
                    });
                }
                CanonicalOption::UTF8 | CanonicalOption::UTF16 => {
                    // These options are implicitly capturing in recording, so do nothing
                }
                _ => panic!("Canonical option variant not supported yet: {:?}", opt),
            }
        }
        (!options.is_empty()).then_some(opts_ref)
    }
}

/// Prefix all function names in the custom name section with `"{module_name}::"`.
/// This makes function names unique and identifiable after merging.
fn prefix_function_names(module: &mut Module<'_>) {
    let prefix = module
        .module_name
        .as_ref()
        .expect("Module name should be set")
        .clone();
    log::trace!("Changing prefixes for module: {:?}", prefix);
    // There is some weirdness with setting import function names here... just ignore
    let fids: Vec<FunctionID> = module
        .functions
        .iter()
        .filter(|f| f.is_local())
        .map(|f| FunctionID(f.get_id()))
        .collect();
    for fid in fids {
        if let Some(name) = module.functions.get_name(fid).clone() {
            module
                .functions
                .set_local_fn_name(fid, format!("{}::{}", prefix, name));
        } else {
            module
                .functions
                .set_local_fn_name(fid, format!("{}::func{}", prefix, *fid));
        }
    }
}

/// Run `wasm-merge` on the modules at the path, produc
///
/// IMPORTANT NOTE: The order of the arguments for the merge matters! Always put the expected
/// wasm module that exports main as the first argument
fn merge_modules<T: AsRef<Path> + AsRef<OsStr>>(input: Vec<PathBuf>, output: T) -> Result<()> {
    let mut cmd = Command::new("wasm-merge");
    cmd.arg("--all-features");
    cmd.arg("--debuginfo");
    cmd.arg("--rename-export-conflicts");
    for module_path in &input {
        cmd.arg(module_path);
        cmd.arg(module_path.file_stem().unwrap());
    }
    cmd.arg("-o").arg(&output);
    log::debug!("Running: {:?}", cmd);
    let output = cmd.output().unwrap();
    if !output.status.success() {
        bail!(
            "Failed to merge decomposed modules: {}",
            str::from_utf8(&output.stderr)?
        );
    }
    // Delete the individual modules
    for module_path in &input {
        fs::remove_file(&module_path)?;
    }
    Ok(())
}

/// Validate assumptions about the component that must hold for decomposition to be valid
///
/// Relax these as we build out this tool. Currently, we stop the following:
/// * Imported core modules
/// * Imported components
/// * FromExport main component instances
/// * Nested components that are imported or have modules/core instances
///     (essentially nested components can only import/export things)
///
/// Note: Imports can still use things like component, module. We are not testing for
/// full recursive enforcement of these assumptions.
fn validate_assumptions<'a>(component: &Component<'a>) -> Result<()> {
    for inst in component.instances.iter_resolved(component) {
        match inst {
            ResolvedComponentInstance::Imported(_)
            | ResolvedComponentInstance::Instantiated {
                component_idx: _,
                args: _,
            } => {}
            ResolvedComponentInstance::FromExports(_) => {
                unsupported!("Main, inline FromExport component instances")?;
            }
        }
    }

    for module in component.modules.iter_resolved(&component) {
        if let ResolvedModule::Imported { .. } = module {
            unsupported!("Imported modules")?;
        }
    }

    for subcomponent in component.components.iter_resolved(component) {
        match subcomponent {
            ResolvedComponent::Imported { .. } => {
                unsupported!("Imported components")?;
            }
            ResolvedComponent::Defined {
                component: subcomponent_ref,
            } => {
                let sc = &subcomponent_ref.borrow();
                unsupported!(
                    sc.modules.iter_resolved(&sc).count() > 0,
                    "Nested components should not have modules"
                )?;
                unsupported!(
                    sc.core_instances.iter_resolved(&sc).count() > 0,
                    "Nested components should not have core instances"
                )?;
            }
        }
    }

    Ok(())
}

pub(crate) fn get_export_name_from_kind_idx(
    component: &Component,
    module_idx: u32,
    kinds: Vec<ExternalKind>,
    kind_idx: u32,
) -> String {
    // This is safe since we assume no imported modules for now
    let link_module = component.resolve_module(module_idx).defined();
    let export = link_module
        .exports
        .iter()
        .find(|export| kinds.contains(&export.kind) && export.index == kind_idx)
        .expect(
            format!(
                "Export {:?}, {:?} should be found in module {:?}",
                kinds, kind_idx, module_idx,
            )
            .as_str(),
        );
    export.name.clone()
}

/// Gather linking information for a single `InstantiationArg` into `link_imports`
fn gather_instance_link(
    link_imports: &mut HashMap<ModuleImportIndex, ImportKind>,
    mut member_imports: HashMap<String, ModuleImportIndex>,
    component: &Component,
    export_if: &InstanceExportInterface,
    instance_map: &HashMap<ModuleInstanceID, ModuleID>,
) -> Result<()> {
    for export in export_if.exports.iter() {
        let core_import_idx: ModuleImportIndex = member_imports
            .remove(&export.name.to_string())
            .expect("export should be matched by an import")
            .into();
        match export_if.kind {
            InstanceKind::FromExports => {
                match export.kind {
                    ExternalKind::Func => {
                        let core_func = component.resolve_core_func(export.index);
                        match core_func {
                            ResolvedCoreFunc::Lowered { func_idx, options } => {
                                let comp_func = component.resolve_component_func(func_idx);
                                log::trace!(
                                    "CoreFunc[{:?}] lowered from ComponentFunc[{:?}] with options {:?}",
                                    export.index,
                                    comp_func,
                                    options
                                );
                                match comp_func {
                                    ResolvedComponentFunc::Imported { .. } => {
                                        link_imports.insert(
                                            core_import_idx,
                                            ImportKind::TrueImport(
                                                CanonicalOptionsIndex::from_options(
                                                    &component,
                                                    &options,
                                                    instance_map,
                                                ),
                                            ),
                                        );
                                    }
                                    ResolvedComponentFunc::Lifted { .. } => {
                                        panic!(
                                            "Lowered CoreFunc should not come from lifted ComponentFunc"
                                        )
                                    }
                                }
                            }
                            ResolvedCoreFunc::FromModule {
                                module_idx,
                                func_idx,
                            } => {
                                let module_id = ModuleID(module_idx);
                                log::trace!(
                                    "CoreFunc[{:?}] from module {:?} func idx {:?}",
                                    export.index,
                                    module_idx,
                                    func_idx
                                );
                                // This is safe since we assume no imported modules for now
                                let export_name = get_export_name_from_kind_idx(
                                    component,
                                    module_idx,
                                    vec![ExternalKind::Func, ExternalKind::FuncExact],
                                    func_idx,
                                );
                                link_imports.insert(
                                    core_import_idx,
                                    ImportKind::Rename {
                                        // The module_idx is being used for the module ID for now since we don't have nested/imported modules
                                        package: assumed_instance_id(instance_map, module_id),
                                        member: export_name,
                                    },
                                );
                            }
                            ResolvedCoreFunc::ResourceDrop { .. } => {
                                log::trace!("CoreFunc[{:?}] is a resource drop", export.index);
                                link_imports.insert(core_import_idx, ImportKind::Builtin);
                            }
                        }
                    }
                    ExternalKind::Table => {
                        let table = component.resolve_core_table(export.index);
                        let module_id = ModuleID(table.module_idx);
                        log::trace!("CoreTable resolved to {:?}", table);
                        let export_name = get_export_name_from_kind_idx(
                            component,
                            table.module_idx,
                            vec![ExternalKind::Table],
                            table.table_idx,
                        );
                        link_imports.insert(
                            core_import_idx,
                            ImportKind::Rename {
                                package: assumed_instance_id(instance_map, module_id),
                                member: export_name,
                            },
                        );
                    }
                    ExternalKind::Memory => {
                        println!("Resolving CoreMemory export: {:?}", export);
                        let memory = component.resolve_core_memory(export.index);
                        let module_id = ModuleID(memory.module_idx);
                        log::trace!("CoreMemory resolved to {:?}", memory);
                        let export_name = get_export_name_from_kind_idx(
                            component,
                            memory.module_idx,
                            vec![ExternalKind::Memory],
                            memory.memory_idx,
                        );
                        link_imports.insert(
                            core_import_idx,
                            ImportKind::Rename {
                                package: assumed_instance_id(instance_map, module_id),
                                member: export_name,
                            },
                        );
                    }
                    _ => {
                        unsupported!(format!("Linking of export kind {:?}", export.kind))?;
                    }
                }
            }
            InstanceKind::FromInstantiated(module_id) => {
                // If exports are linked directly from instantiated modules, this operates exactly
                // like core Wasm module linking, and hence we only need to do Renaming
                link_imports.insert(
                    core_import_idx,
                    ImportKind::Rename {
                        package: assumed_instance_id(instance_map, module_id),
                        member: export.name.to_string(),
                    },
                );
            }
        }
    }
    assert!(
        member_imports.is_empty(),
        "All imports should be matched by exports"
    );
    Ok(())
}

/// Gather exported functions from the component and return them with appropriate CRIMP recorded index
///
/// For exported instances, the exports within the instance get linearized
fn gather_component_exports<'a>(
    export_funcs: &mut HashMap<ModuleInstanceID, Vec<ExportFuncMetadata>>,
    component: &Component<'a>,
    instance_map: &HashMap<ModuleInstanceID, ModuleID>,
    clm: &ComponentLinkingMetadata<'a>,
) -> Result<()> {
    let mut export_id = 0;
    for export in component.exports.iter() {
        log::trace!(
            "Processing export {:?} from component with parents {:?}",
            export.name,
            component.parents
        );

        #[derive(Debug)]
        enum ComponentContext<'a, 'b> {
            Main {
                component: &'b Component<'a>,
                instance_map: &'b HashMap<ModuleInstanceID, ModuleID>,
            },
            Sub {
                sub_component: &'b Component<'a>,
                main_component: &'b Component<'a>,
                main_instance_map: &'b HashMap<ModuleInstanceID, ModuleID>,
                import_binds: &'b HashMap<String, ComponentImportBindInParent>,
            },
        }

        fn handle_func(
            context: ComponentContext,
            func_index: u32,
            export_id: &mut usize,
            export_funcs: &mut HashMap<ModuleInstanceID, Vec<ExportFuncMetadata>>,
        ) {
            println!("Handling func export with export_id and func_index: {:?} | {:?}", export_id, func_index);
            let (target_component, target_instance_map) = match context {
                ComponentContext::Main {
                    component,
                    instance_map,
                } => (component, instance_map),
                ComponentContext::Sub {
                    sub_component,
                    main_component: _,
                    main_instance_map,
                    import_binds: _,
                } => (sub_component, main_instance_map),
            };
            match target_component.resolve_component_func(func_index) {
                ResolvedComponentFunc::Imported(import) => {
                    match import {
                        ResolvedImport::Direct { name, ty: _ } => {
                            // Do nothing since this will be resolved by the parent that instantiates this component
                            let (main_component, main_instance_map, binds) =
                                match context {
                                    ComponentContext::Main { .. } => panic!(
                                        "Direct imports in main component should have been ruled out by assumptions"
                                    ),
                                    ComponentContext::Sub {
                                        main_component,
                                        main_instance_map,
                                        import_binds,
                                        ..
                                    } => (
                                        main_component,
                                        main_instance_map,
                                        import_binds,
                                    ),
                                };
                            log::trace!("Binds: {:?}; Name: {}", binds, name);
                            match binds.get(name)
                                .expect("Import should be matched by an import bind in the component instance") {
                                    ComponentImportBindInParent::Func(func_idx) => {
                                        // Handle func in the parent content
                                        handle_func(ComponentContext::Main { component: main_component, instance_map: main_instance_map }, 
                                            *func_idx, export_id, export_funcs)
                                    }
                                }
                        }
                        _ => panic!("Imported component funcs should only be direct imports"),
                    }
                }
                ResolvedComponentFunc::Lifted {
                    core_func_idx,
                    type_idx: _type_idx,
                    options,
                } => {
                    let core_func = target_component.resolve_core_func(core_func_idx);
                    println!("Got into lifted");
                    match core_func {
                        ResolvedCoreFunc::FromModule {
                            module_idx,
                            func_idx,
                        } => {
                            export_funcs
                                .entry(assumed_instance_id(
                                    target_instance_map,
                                    ModuleID(module_idx),
                                ))
                                .or_default()
                                .push(ExportFuncMetadata {
                                    record_id: RecordExportIndex(*export_id as u32),
                                    name: get_export_name_from_kind_idx(
                                        target_component,
                                        module_idx,
                                        vec![ExternalKind::Func, ExternalKind::FuncExact],
                                        func_idx,
                                    ),
                                    opts: CanonicalOptionsIndex::from_options(
                                        target_component,
                                        &options,
                                        target_instance_map,
                                    ),
                                });
                            *export_id += 1;
                        }
                        _ => {
                            panic!("Lifted ComponentFunc sourced from non-FromModule CoreFuncs");
                        }
                    }
                }
            }
        }

        match export.kind {
            ComponentExternalKind::Func => {
                handle_func(
                    ComponentContext::Main {
                        component,
                        instance_map,
                    },
                    export.index,
                    &mut export_id,
                    export_funcs,
                );
            }
            ComponentExternalKind::Instance => {
                log::warn!("Yet to support Export(Instance)");
                let sc_instance_id = ComponentInstanceID(export.index);
                let sc_id = clm.instance_map.get(&sc_instance_id).unwrap();
                let sc = clm.cm.get(sc_id).unwrap();
                let sc_instance_metadata = clm.instantiations.get(&sc_instance_id).unwrap();

                // Iterate through all the instance's subexports (only func for now)
                for subexport in clm.cm.get(sc_id).unwrap().component.borrow().exports.iter() {
                    log::trace!("Subexport from exported instance: {:?}", subexport);
                    match subexport.kind {
                        ComponentExternalKind::Func => {
                            handle_func(
                                ComponentContext::Sub {
                                    sub_component: &sc.component.borrow(),
                                    main_component: component,
                                    main_instance_map: instance_map,
                                    import_binds: &sc_instance_metadata.imports,
                                },
                                subexport.index,
                                &mut export_id,
                                export_funcs,
                            );
                        }
                        _ => {
                            panic!("Subexport kind is not supported for access yet..",);
                        }
                    }
                }
            }

            _ => {
                log::warn!(
                    "Export kind from {:?} is not supported for access yet..",
                    export
                );
            }
        }
    }
    log::debug!("Gathered export funcs: {:?}", export_funcs);
    Ok(())
}

fn gather_component_instance_map<'a>(
    instance_map: &mut HashMap<ComponentInstanceID, ComponentID>,
    component: &Component<'a>,
) {
    for (instance_idx, instance) in component.instances.iter().enumerate() {
        match instance.resolve(component) {
            ResolvedComponentInstance::Instantiated {
                component_idx,
                args: _,
            } => {
                instance_map.insert(
                    ComponentInstanceID(instance_idx as u32),
                    ComponentID(component_idx),
                );
            }
            _ => {}
        }
    }
    log::debug!("Gathered component instance map: {:?}", instance_map);
}

/// Construct the flattened mapping of core instances to modules for the component
fn gather_instance_map(
    instance_map: &mut HashMap<ModuleInstanceID, ModuleID>,
    component: &Component,
) {
    for (instance_idx, instance) in component.core_instances.iter().enumerate() {
        match instance.resolve(&component) {
            ResolvedCoreInstance::Instantiated {
                module_idx,
                args: _,
            } => {
                instance_map.insert(ModuleInstanceID(instance_idx as u32), ModuleID(module_idx));
            }
            _ => {}
        }
    }
    log::debug!("Gathered instance map: {:?}", instance_map);
}

/// Construct the [`LinkingMetadata`] for the component
///
/// Right now, with the lack of nested components and instances, it can be assumed that:
/// * `ModuleID` == module_idx for a module in the component
/// * `ModuleInstanceID` == instance_idx for a core instance in the component
///
/// But this assumption may change in the future.
fn linking_metadata<'a>(
    component: &Component<'a>,
    checksum: Checksum,
) -> Result<LinkingMetadata<'a>> {
    // Keep track of synthetic export instances (NOT with InstanceID, but with the actual index value in the component)
    let mut synthetic_core_instances_exports = HashMap::<u32, InstanceExportInterface>::new();
    let mut linking = LinkingMetadata {
        checksum,
        ..Default::default()
    };

    //// Linking metadata for subcomponents is gathered recursively.
    //let sub_linking = component
    //    .components
    //    .iter_resolved(component)
    //    .map(|subcomponent| {
    //        let subcomponent = match subcomponent {
    //            ResolvedComponent::Imported(_) => {
    //                panic!("Imported subcomponents should have been ruled out by assumptions");
    //            }
    //            ResolvedComponent::Defined { component } => component,
    //        };
    //        linking_metadata(&subcomponent.borrow(), Checksum::default())
    //    })
    //    .collect::<Result<Vec<_>>>()?;

    gather_instance_map(&mut linking.instance_map, component);

    // Core instance linking handling populates most of linking
    for (instance_idx, instance) in component.core_instances.iter().enumerate() {
        let instance_id = ModuleInstanceID(instance_idx as u32);
        if let CoreInstanceNode::Aliased(alias) = instance {
            unsupported!(format!("Aliased core instance: {:?}", alias))?;
        }
        match instance.resolve(&component) {
            ResolvedCoreInstance::FromExports(exports) => {
                synthetic_core_instances_exports.insert(
                    instance_idx as u32,
                    InstanceExportInterface {
                        exports: exports.iter().map(|e| (*e).into()).collect(),
                        kind: InstanceKind::FromExports,
                    },
                );
            }
            ResolvedCoreInstance::Instantiated { module_idx, args } => {
                let module_id = ModuleID(module_idx);
                // Register the exports for this core instance
                synthetic_core_instances_exports.insert(
                    instance_idx as u32,
                    InstanceExportInterface {
                        exports: component
                            .resolve_module(module_idx)
                            .defined()
                            .exports
                            .iter()
                            .map(|export| export.clone())
                            .collect(),
                        kind: InstanceKind::FromInstantiated(module_id),
                    },
                );

                // Gather the necessary imports to bind from the module
                let module_metadata = linking.mm.entry(module_id).or_insert_with(|| {
                    // Populate import map for the module being instantiated
                    let mut metadata = ModuleMetadata {
                        module: component.resolve_module(module_idx).defined(),
                        import_map: HashMap::new(),
                    };
                    for (i, import) in metadata.module.imports.iter().enumerate() {
                        let members = metadata
                            .import_map
                            .entry(import.module.as_ref().to_owned())
                            .or_default();
                        members.insert(import.name.to_string(), ModuleImportIndex(i as u32));
                    }
                    metadata
                });

                // Gather linking information from args
                let mut expected_imports = module_metadata.import_map.clone();
                assert_eq!(args.len(), expected_imports.len());
                log::debug!(
                    "Linking for CoreInstance[{:?}] from Module[{:?}]",
                    instance_id,
                    module_idx
                );
                let mut instance_metadata = InstantiationLinkingMetadata {
                    // Works for now since we only consider core instances in the main component, not nested
                    instantiate_order: *instance_id,
                    imports: Default::default(),
                };
                for arg in args {
                    // Ensure no new kinds of instantiation args are introduced
                    match arg.kind {
                        InstantiationArgKind::Instance => {}
                    };
                    // Get the export for instance providing 'arg.name' package
                    let instance_export_if = synthetic_core_instances_exports
                        .get(&arg.index)
                        .expect("exported core instance should be already populated");
                    // Get imports for 'arg.name' package
                    let member_imports = expected_imports
                        .remove(arg.name)
                        .expect("import should be populated");
                    gather_instance_link(
                        &mut instance_metadata.imports,
                        member_imports,
                        &component,
                        &instance_export_if,
                        &linking.instance_map,
                    )?;
                }
                log::info!(
                    "Instantiated CoreInstance[{:?}]: {:?}",
                    instance_id,
                    instance_metadata
                );
                linking
                    .instantiations
                    .insert(instance_id, instance_metadata);
            }
        }
    }

    // Component instances
    gather_component_instance_map(&mut linking.clm.instance_map, component);

    for (instance_idx, instance) in component.instances.iter().enumerate() {
        let instance_id = ComponentInstanceID(instance_idx as u32);
        if let ComponentInstanceNode::Exported(_) = instance {
            // Do not gather for exported instances
            continue;
        }
        match instance.resolve(&component) {
            ResolvedComponentInstance::Imported(_) => {}
            ResolvedComponentInstance::FromExports(_) => {
                unsupported!("Main, inline FromExport component instances")?;
            }
            ResolvedComponentInstance::Instantiated {
                component_idx,
                args,
            } => {
                let component_id = ComponentID(component_idx);
                // Gather the necessary imports to bind from the component
                let component_metadata = linking.clm.cm.entry(component_id).or_insert_with(|| {
                    // Populate import map for the component being instantiated
                    let mut metadata = ComponentMetadata {
                        component: component.resolve_component(*component_id).defined(),
                        import_map: HashMap::new(),
                    };
                    for (i, import) in metadata.component.borrow().imports.iter().enumerate() {
                        metadata
                            .import_map
                            .insert(import.name.0.to_string(), ComponentImportIndex(i as u32));
                    }
                    metadata
                });

                // Gather linking information from args
                let mut expected_imports: HashSet<&str> = component_metadata
                    .import_map
                    .keys()
                    .map(|k| k.as_str())
                    .collect();
                assert_eq!(args.len(), expected_imports.len());
                log::debug!("Linking for {:?} from {:?}", instance_id, component_id);
                let mut instance_metadata = ComponentInstantiationLinkingMetadata {
                    // Works for now since we only consider core instances in the main component, not nested
                    instantiate_order: *instance_id,
                    imports: Default::default(),
                };

                // Get all the import mapping
                for arg in args {
                    assert!(
                        expected_imports.remove(arg.name),
                        "import should be populated"
                    );
                    let bind = match arg.kind {
                        ComponentExternalKind::Func => ComponentImportBindInParent::Func(arg.index),
                        _ => {
                            panic!(
                                "Linking of import kind {:?} for component instances is not supported yet..",
                                arg.kind
                            );
                        }
                    };
                    instance_metadata.imports.insert(arg.name.to_string(), bind);
                }
                assert!(
                    expected_imports.is_empty(),
                    "All imports should be matched by args for component instance"
                );
                log::info!("Instantiated {:?}: {:?}", instance_id, instance_metadata);
                linking
                    .clm
                    .instantiations
                    .insert(instance_id, instance_metadata);
            }
        }
    }

    gather_component_exports(
        &mut linking.export_funcs,
        &component,
        &linking.instance_map,
        &linking.clm,
    )?;
    Ok(linking)
}

/// Decomposed representation of a component into its constituent modules with linking metadata
#[derive(Default)]
struct ComponentDecomposed<'a> {
    modules: Vec<Module<'a>>,
    glue: Option<DriverGlueModules<'a>>,
    merge_opts: MergeOptions,
}

impl<'a> ComponentDecomposed<'a> {
    /// Validate all modules in the decomposed representation.
    fn validate_modules(&self) -> Result<()> {
        for module in self
            .modules
            .iter()
            .chain(self.glue.iter().flat_map(|glue| [&glue.driver, &glue.glue]))
        {
            Validator::new()
                .validate_all(&module.encode()?)
                .with_context(|| {
                    format!(
                        "Module validation failed for module {:?}",
                        module.module_name
                    )
                })?;
        }
        Ok(())
    }

    fn from_linking_metadata(
        linking: LinkingMetadata<'a>,
        glue_args: Option<GlueArgs>,
        merge_opts: MergeOptions,
    ) -> Result<Self> {
        // Sanity checks on the linking metadata before we use it for decomposition
        let l1 = linking.mm.keys().collect::<HashSet<_>>();
        let l2 = linking
            .instantiations
            .keys()
            .map(|instance_id| &linking.instance_map[instance_id])
            .collect::<HashSet<_>>();
        assert_eq!(
            l1, l2,
            "Each module should be instantiated exactly once for now"
        );
        let instantiated_modules = linking.instantiations.keys().collect::<HashSet<_>>();
        let export_func_modules = linking.export_funcs.keys().collect::<HashSet<_>>();
        assert!(
            export_func_modules.is_subset(&instantiated_modules),
            "Exported functions should only come from instantiated modules"
        );

        let (modules, glue) = if let Some(args) = glue_args {
            // Generate glue and driver bindings
            let mut builder = GlueBuilder::new(linking.checksum);
            let crimp_modules = linking
                .instantiations
                .keys()
                .map(|instance_id| linking.adapt_and_update_glue(*instance_id, &mut builder))
                .collect::<Result<Vec<_>>>()?;
            (
                crimp_modules,
                Some(DriverGlueModules::from_path_and_builder(
                    args.trace_path.unwrap(),
                    builder,
                )?),
            )
        } else {
            // Serialize into custom section
            let crimp_modules = linking
                .instantiations
                .keys()
                .map(|instance_id| {
                    let (mut crimp_module, crimp_section) =
                        linking.adapt_and_serialize_crimp_section(*instance_id)?;
                    let _cid = crimp_module.add_custom_section(CustomSection {
                        name: "crimp-replay",
                        data: Cow::from(crimp_section),
                    });
                    Ok(crimp_module)
                })
                .collect::<Result<Vec<_>>>()?;
            (crimp_modules, None)
        };

        Ok(Self {
            modules,
            glue,
            merge_opts,
        })
    }

    /// Produce a [ComponentDecomposed] from a [Component]
    fn from_component(
        component_rc: Rc<RefCell<Component<'a>>>,
        checksum: Checksum,
        glue_args: Option<GlueArgs>,
        merge_opts: MergeOptions,
    ) -> Result<Self> {
        let component = component_rc.borrow();
        validate_assumptions(&component)?;
        let lm = linking_metadata(&component, checksum)?;
        let decomposed = Self::from_linking_metadata(lm, glue_args, merge_opts)?;
        decomposed.validate_modules()?;
        Ok(decomposed)
    }

    /// Prepare modules for DriverSplit merging by namespacing exports and renaming
    /// cross-group import module/member names so they match after separate merges.
    ///
    /// After this step:
    /// - Decomposed module exports are namespaced: `{module_name}:{export_name}`
    /// - Inter-module imports within the decomposed group have namespaced members
    /// - Decomposed modules' `crimp_glue` imports point to `crimp_driver` (merged driver name)
    /// - Glue module imports from decomposed modules point to `decomposed_component`
    fn prepare_driver_split(&mut self) {
        let module_names: HashSet<String> = self
            .modules
            .iter()
            .filter_map(|m| m.module_name.clone())
            .collect();

        // Namespace all exports in decomposed modules
        for module in &mut self.modules {
            let module_name = module.module_name.as_ref().unwrap().clone();
            for export in module.exports.iter_mut() {
                export.name = format!("{}:{}", module_name, export.name);
            }
        }

        // Update imports in decomposed modules
        for module in &mut self.modules {
            for import in module.imports.iter_mut() {
                let import_module = import.module.to_string();
                if module_names.contains(&import_module) {
                    // Inter-module import: namespace the member name to match namespaced exports
                    import.name = Cow::Owned(format!("{}:{}", import_module, import.name));
                } else if import_module == GLUE_MODULE_NAME {
                    // Cross-group: crimp_glue → crimp_driver (the merged driver+glue output name)
                    // Member name already namespaced by adapt_and_update_glue, don't touch it
                    import.module = Cow::Owned(DRIVER_MODULE_NAME.to_string());
                }
            }
        }

        // Update imports in the glue module
        if let Some(ref mut glue_modules) = self.glue {
            for import in glue_modules.glue.imports.iter_mut() {
                let import_module = import.module.to_string();
                if module_names.contains(&import_module) {
                    // Imports from decomposed modules → point to merged decomposed_component
                    // Namespace member to match the namespaced exports
                    import.name = Cow::Owned(format!("{}:{}", import_module, import.name));
                    import.module = Cow::Owned(DECOMPOSED_COMPONENT_NAME.to_string());
                }
                // crimp_driver imports stay as-is (resolved within Group 2 merge)
            }
        }
    }

    /// Helper to write a single module to a file, returning the .wasm path.
    /// Always writes .wasm; additionally writes .wat when the flag is set.
    fn write_module(module: Module<'_>, wat: bool, outdir: &PathBuf) -> Result<PathBuf> {
        let wasm_bytes = module.encode()?;
        let module_name = module
            .module_name
            .clone()
            .expect("The module name should always be set for decomposed modules");
        let mut wasm_path = outdir.join(&module_name);
        wasm_path.set_extension("wasm");
        log::info!("Writing module: {:?}", wasm_path);
        fs::write(&wasm_path, &wasm_bytes)?;
        if wat {
            let wat_bytes = wasmprinter::print_bytes(&wasm_bytes)?.into_bytes();
            let mut wat_path = outdir.join(&module_name);
            wat_path.set_extension("wat");
            fs::write(&wat_path, wat_bytes)?;
        }
        Ok(wasm_path)
    }

    /// Optionally convert a merged wasm file to WAT format.
    fn convert_to_wat_if_needed(path: &PathBuf, wat: bool) -> Result<()> {
        if wat {
            let bytes = fs::read(path)?;
            let wat_bytes = wasmprinter::print_bytes(bytes)?.into_bytes();
            let mut wat_path = path.clone();
            wat_path.set_extension("wat");
            fs::write(&wat_path, wat_bytes)?;
        }
        Ok(())
    }

    /// Write the decomposed modules to files in the output directory, optionally merging them with `wasm-merge`
    fn dump_to_files(mut self, wat: bool, outdir: &PathBuf) -> Result<()> {
        // Apply renaming for DriverSplit before writing files
        if matches!(self.merge_opts, MergeOptions::DriverSplit) {
            self.prepare_driver_split();
        }

        // For merge modes, don't write .wat for intermediate files — only for final merged outputs.
        let intermediate_wat = !self.merge_opts.any_merge() && wat;

        // Prefix all function names with the module name for debuggability
        for module in &mut self.modules {
            prefix_function_names(module);
        }
        if let Some(ref mut glue) = self.glue {
            prefix_function_names(&mut glue.driver);
            prefix_function_names(&mut glue.glue);
        }

        // Write decomposed modules
        let mut decomposed_paths = vec![];
        for module in self.modules.into_iter() {
            decomposed_paths.push(Self::write_module(module, intermediate_wat, outdir)?);
        }

        // Write glue/driver modules (driver first — it exports _start)
        let mut glue_driver_paths = vec![];
        if let Some(glue) = self.glue {
            for module in [glue.driver, glue.glue] {
                glue_driver_paths.push(Self::write_module(module, intermediate_wat, outdir)?);
            }
        }

        match self.merge_opts {
            MergeOptions::FullSplit => {}
            MergeOptions::FullMerge => {
                let all_paths: Vec<PathBuf> = glue_driver_paths
                    .into_iter()
                    .chain(decomposed_paths.into_iter())
                    .collect();
                let merged_path = outdir.join("decomposed_component_replay.wasm");
                merge_modules(all_paths, &merged_path)?;
                Self::convert_to_wat_if_needed(&merged_path, wat)?;
                log::info!("Merged all modules into {:?}", merged_path);
            }
            MergeOptions::DriverSplit => {
                // Merge 1: Decomposed modules
                let decomposed_merged = outdir.join(format!("{}.wasm", DECOMPOSED_COMPONENT_NAME));
                merge_modules(decomposed_paths, &decomposed_merged)?;
                Self::convert_to_wat_if_needed(&decomposed_merged, wat)?;
                log::info!("Merged decomposed modules into {:?}", decomposed_merged);

                // Merge 2: Driver + glue
                let driver_merged_tmp = outdir.join("crimp_replay_driver.wasm");
                merge_modules(glue_driver_paths, &driver_merged_tmp)?;
                let driver_merged = outdir.join(format!("{}.wasm", DRIVER_MODULE_NAME));
                fs::rename(&driver_merged_tmp, &driver_merged)?;
                Self::convert_to_wat_if_needed(&driver_merged, wat)?;
                log::info!("Merged driver+glue into {:?}", driver_merged);
            }
        }
        Ok(())
    }
}

fn main() -> Result<()> {
    env_logger::init();
    let cli = CLI::parse();
    if cli.glue ^ cli.glue_args.trace_path.is_some() {
        bail!("Glue args must be provided when glue is enabled, and vice versa");
    }
    if matches!(cli.merge, MergeOptions::DriverSplit) && !cli.glue {
        bail!("DriverSplit merge mode requires --glue to be enabled");
    }
    let glue_args = cli.glue.then_some(cli.glue_args);

    let file = wat::parse_file(&cli.component)?;

    // Validate with wasmparser
    Validator::new()
        .validate_all(&file)
        .with_context(|| "Validation failed")?;

    let checksum: Checksum = Sha256::digest(&file).as_slice().try_into().unwrap();
    let component_rc = parse_component(&file).with_context(|| "Failed to parse component")?;

    if cli.outdir.exists() {
        fs::remove_dir(&cli.outdir)?;
    }
    fs::create_dir(&cli.outdir)?;

    let decomposed =
        ComponentDecomposed::from_component(component_rc, checksum, glue_args, cli.merge)?;
    decomposed.dump_to_files(cli.wat, &cli.outdir)?;
    Ok(())
}
