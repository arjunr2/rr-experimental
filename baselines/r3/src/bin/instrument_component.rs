//! Shadow memory instrumentation for wasm components.

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

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

    for module in component.modules.iter_mut() {
        r3_baseline::instrument_shadow(module)?;
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
