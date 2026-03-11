//! Shadow memory instrumentation for core wasm modules.

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use wirm::ir::module::Module;

#[derive(Parser)]
#[command(name = "r3-instrument")]
#[command(about = "Add shadow memory instrumentation to a core wasm module")]
struct Args {
    /// Input core wasm module (.wasm or .wat)
    #[arg(short, long)]
    module: PathBuf,

    /// Output instrumented wasm module
    #[arg(short, long)]
    output: PathBuf,
}

fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();

    let raw_bytes = std::fs::read(&args.module)?;
    // Support both .wasm (binary) and .wat (text) input
    let wasm_bytes = wat::parse_bytes(&raw_bytes)
        .map_err(|e| anyhow::anyhow!("wat parse error: {}", e))?;

    let mut module = Module::parse(&wasm_bytes, true, false)
        .map_err(|e| anyhow::anyhow!("parse error: {}", e))?;

    r3_baseline::instrument_shadow(&mut module, false)?;

    let output_bytes = module
        .encode()
        .map_err(|e| anyhow::anyhow!("encode error: {}", e))?;

    // Validate the output module
    let mut validator =
        wirm::wasmparser::Validator::new_with_features(wirm::wasmparser::WasmFeatures::all());
    validator
        .validate_all(&output_bytes)
        .map_err(|e| anyhow::anyhow!("output validation failed: {}", e))?;

    std::fs::write(&args.output, &output_bytes)?;
    log::info!(
        "Wrote instrumented module ({} bytes) to {:?}",
        output_bytes.len(),
        args.output
    );

    Ok(())
}
