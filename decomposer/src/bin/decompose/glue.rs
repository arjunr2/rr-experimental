use clap::Args;
use decomposer::wirm::Module;

const GLUE_MODULE_NAME: &str = "crimp_glue";
const DRIVER_MODULE_NAME: &str = "crimp_driver";
#[derive(Debug, Default)]
pub struct DriverGlueModules<'a> {
    pub driver: Module<'a>,
    pub glue: Module<'a>,
}

impl<'a> DriverGlueModules<'a> {
    /// Construct from trace path to build the driver and a glue builder
    pub fn from_path_and_builder(trace_path: String, builder: GlueBuilder<'a>) -> Self {
        Self {
            driver: Module::default(),
            glue: builder.finish(),
        }
    }
}

#[derive(Debug, Args)]
pub struct GlueArgs {
    /// Only valid with `glue` is true - the path to the trace file to be embedded in the replay driver
    /// module for use during replay
    #[arg(short = 'p', long = "trace-path")]
    pub trace_path: Option<String>,
}

/// Builder for glue modules
#[derive(Debug)]
pub struct GlueBuilder<'a> {
    module: Module<'a>,
}

impl<'a> GlueBuilder<'a> {
    pub fn new() -> Self {
        Self {
            module: Module::default(),
        }
    }
    pub fn finish(self) -> Module<'a> {
        self.module
    }
}
