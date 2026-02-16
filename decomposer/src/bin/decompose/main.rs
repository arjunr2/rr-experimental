//! CLI tool to decompose a WebAssembly Component into its constituent modules.

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use decomposer::wasmparser::{
    CanonicalOption, ComponentExternalKind, ExternalKind, InstantiationArgKind, Validator,
};
use env_logger;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use decomposer::Component;
use decomposer::ir::{
    CoreInstanceNode, Resolve, ResolvedComponentFunc, ResolvedComponentInstance, ResolvedCoreFunc,
    ResolvedCoreInstance, ResolvedModule,
};
use decomposer::parse_component;
use decomposer::wirm::Module;
use decomposer::wirm::ir::module::module_exports::Export;
use decomposer::wirm::ir::types::CustomSection;

mod linking;
use linking::*;

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

#[derive(Parser, Debug)]
#[command(name = "decompose")]
#[command(about = "Decompose a WebAssembly Component into its modules")]
struct CLI {
    /// Input component file to decompose.
    #[arg(short, long)]
    component: PathBuf,
    /// Whether to generate output in WAT format (as opposed to binary format).
    #[arg(short = 't', long = "wat")]
    wat: bool,
    /// Overwrite the output directory if it exists.
    #[arg(short = 'x', long = "overwrite")]
    overwrite: bool,
    /// Merge the decomposed components with `wasm-merge` on output
    #[arg(short = 'm', long = "merge")]
    merge: bool,
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

/// Run `wasm-merge` on the modules at the path, produc
fn merge_modules<T: AsRef<Path> + AsRef<OsStr>>(input: Vec<PathBuf>, output: T) -> Result<()> {
    let mut cmd = Command::new("wasm-merge");
    cmd.arg("--all-features");
    for module_path in &input {
        cmd.arg(module_path);
        cmd.arg(module_path.file_stem().unwrap());
    }
    cmd.arg("-o").arg(&output);
    log::debug!("Running: {:?}", cmd);
    let output = cmd.output().unwrap();
    if !output.status.success() {
        return Err(anyhow!(
            "Failed to merge decomposed modules: {}",
            str::from_utf8(&output.stderr)?
        ));
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
/// * Main component instances
/// * Nested components
///
/// Note: Imports can still use things like component, module. We are not testing for
/// full recursive enforcement of these assumptions.
fn validate_assumptions<'a>(component: &Component<'a>) -> Result<()> {
    unsupported!(!component.components.is_empty(), "Nested components")?;

    for inst in component.instances.iter_resolved(component) {
        match inst {
            ResolvedComponentInstance::Imported(_) => {}
            _ => {
                unsupported!("Main, inline component instances")?;
            }
        }
    }

    for module in component.modules.iter_resolved(&component) {
        match module {
            ResolvedModule::Imported { .. } => {
                unsupported!("Imported modules")?;
            }
            _ => {}
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

/// Gather linking information for a single `InstantiationArg` into `import_md`
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

/// Gather exported functions from the component and return them
///
/// TODO: Change when handling component instances - need to consider exports from nested components as well.
/// Right now, we can simply use the `export_id` from enumerating exports, but this will change when we have exports from instances
fn gather_component_exports(
    export_funcs: &mut HashMap<ModuleInstanceID, Vec<ExportFuncMetadata>>,
    component: &Component,
    instance_map: &HashMap<ModuleInstanceID, ModuleID>,
) -> Result<()> {
    for (export_id, export) in component.exports.iter().enumerate() {
        match export.kind {
            ComponentExternalKind::Func => match component.resolve_component_func(export.index) {
                ResolvedComponentFunc::Imported(_) => {
                    unsupported!("Export of imported component functions")?;
                }
                ResolvedComponentFunc::Lifted {
                    core_func_idx,
                    type_idx: _type_idx,
                    options,
                } => {
                    let core_func = component.resolve_core_func(core_func_idx);
                    match core_func {
                        ResolvedCoreFunc::FromModule {
                            module_idx,
                            func_idx,
                        } => {
                            export_funcs
                                .entry(assumed_instance_id(instance_map, ModuleID(module_idx)))
                                .or_default()
                                .push(ExportFuncMetadata {
                                    record_id: RecordExportIndex(export_id as u32),
                                    name: get_export_name_from_kind_idx(
                                        component,
                                        module_idx,
                                        vec![ExternalKind::Func, ExternalKind::FuncExact],
                                        func_idx,
                                    ),
                                    opts: CanonicalOptionsIndex::from_options(
                                        &component,
                                        &options,
                                        instance_map,
                                    ),
                                });
                        }
                        _ => {
                            unsupported!(
                                "Lifted ComponentFunc sourced from non-FromModule CoreFuncs"
                            )?;
                        }
                    }
                }
            },

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

/// Construct the flattened mapping of instances to modules for the component
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

    gather_instance_map(&mut linking.instance_map, component);

    // Only needs to handle core instances for now
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

                // Gather linking information from args
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
                    // Works for now since we only consider core instances
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

    gather_component_exports(&mut linking.export_funcs, &component, &linking.instance_map)?;
    Ok(linking)
}

/// Decomposed representation of a component into its constituent modules with linking metadata
#[derive(Default)]
struct ComponentDecomposed<'a> {
    modules: Vec<Module<'a>>,
}

impl<'a> ComponentDecomposed<'a> {
    /// Validate all modules in the decomposed representation.
    fn validate_modules(&self) -> Result<()> {
        for module in &self.modules {
            Validator::new()
                .validate_all(&module.encode())
                .with_context(|| "Module validation failed")?;
        }
        Ok(())
    }

    fn from_linking_metadata(linking: LinkingMetadata<'a>) -> Result<Self> {
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

        let crimp_modules = linking
            .instantiations
            .keys()
            .map(|instance_id| {
                let (mut crimp_module, crimp_section) =
                    linking.serialize_crimp_section(*instance_id)?;
                let _cid = crimp_module.add_custom_section(CustomSection {
                    name: "crimp-replay",
                    data: Cow::from(crimp_section),
                });
                Ok(crimp_module)
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            modules: crimp_modules,
        })
    }

    /// Produce a [ComponentDecomposed] from a [Component]
    fn from_component(
        component_rc: Rc<RefCell<Component<'a>>>,
        checksum: Checksum,
    ) -> Result<Self> {
        let component = component_rc.borrow();
        validate_assumptions(&component)?;
        let lm = linking_metadata(&component, checksum)?;
        let decomposed = Self::from_linking_metadata(lm)?;
        decomposed.validate_modules()?;
        Ok(decomposed)
    }

    fn dump_to_files(self, wat: bool, outdir: &PathBuf, merge: bool) -> Result<()> {
        let mut module_paths = vec![];
        for module in self.modules {
            let bytes = if wat && !merge {
                wasmprinter::print_bytes(module.encode())?.into_bytes()
            } else {
                module.encode()
            };
            let mut module_path = outdir.join(
                module
                    .module_name
                    .clone()
                    .expect("The module name should always be set for decomposed modules"),
            );
            if !module_path.set_extension(if wat && !merge { "wat" } else { "wasm" }) {
                panic!("Failed to add extension to module path: {:?}", module_path);
            }
            log::info!("Writing module: {:?}", module_path);
            fs::write(&module_path, bytes)?;
            module_paths.push(module_path);
        }

        if merge {
            let mut merged_path = outdir.join("merged.wasm");
            merge_modules(module_paths, &merged_path)?;
            // Optionally convert to WAT if the flag is set
            if wat {
                let bytes = fs::read(&merged_path)?;
                let wat_bytes = wasmprinter::print_bytes(bytes)?.into_bytes();
                merged_path.set_extension("wat");
                fs::write(&merged_path, wat_bytes)?;
            }
            log::info!("Merged modules into {:?}", merged_path);
        }
        Ok(())
    }
}

fn main() -> Result<()> {
    env_logger::init();
    let cli = CLI::parse();
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

    let decomposed = ComponentDecomposed::from_component(component_rc, checksum)?;
    decomposed.dump_to_files(cli.wat, &cli.outdir, cli.merge)?;
    Ok(())
}
