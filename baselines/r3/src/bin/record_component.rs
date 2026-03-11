//! Runner binary that executes an instrumented wasm component with wasmtime,
//! implements the r3 host functions, and writes a postcard-encoded trace.

use anyhow::{Context, Result};
use clap::Parser;
use r3_baseline::R3Event;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

#[derive(Parser)]
#[command(name = "r3-record-component")]
#[command(about = "Run an instrumented wasm component and record an r3 trace")]
#[command(trailing_var_arg = true)]
struct Args {
    /// Output trace file path (omit to discard trace)
    #[arg(short, long)]
    trace: Option<PathBuf>,

    /// Environment variables (KEY=VALUE)
    #[arg(short, long)]
    env: Vec<String>,

    /// Preopened directories (HOST_PATH::GUEST_PATH)
    #[arg(long)]
    dir: Vec<String>,

    /// Enable deterministic execution (NaN canonicalization + relaxed SIMD determinism)
    #[arg(long)]
    deterministic: bool,

    /// Input instrumented wasm component, followed by its arguments
    #[arg(required = true, allow_hyphen_values = true)]
    component_and_args: Vec<String>,
}

struct RecordState {
    wasi_ctx: WasiCtx,
    table: ResourceTable,
    writer: Box<dyn Write + Send>,
}

impl WasiView for RecordState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi_ctx,
            table: &mut self.table,
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();

    let component_path = args
        .component_and_args
        .first()
        .ok_or_else(|| anyhow::anyhow!("missing component path"))?;
    let wasm_args: Vec<&str> = args.component_and_args[1..]
        .iter()
        .map(|s| s.as_str())
        .collect();

    let mut config = Config::new();
    if args.deterministic {
        config.relaxed_simd_deterministic(true);
        config.cranelift_nan_canonicalization(true);
    }
    config.async_support(true);
    let engine = Engine::new(&config)?;

    let wasm_bytes = std::fs::read(component_path)?;
    let component = match Engine::detect_precompiled(&wasm_bytes) {
        Some(wasmtime::Precompiled::Component) => unsafe {
            Component::deserialize(&engine, &wasm_bytes)?
        },
        Some(wasmtime::Precompiled::Module) => {
            anyhow::bail!("expected a component, got a core module")
        }
        None => Component::from_file(&engine, component_path)?,
    };

    let mut linker = Linker::<RecordState>::new(&engine);

    // Add WASI p2 host functions
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;

    // r3 host functions — the instrumented wasm computes diffs and passes
    // raw bytes packed into scalar (lo, hi) i64 parameters.
    {
        let mut r3 = linker.instance("r3")?;
        r3.func_wrap(
            "record-import-call",
            |mut store: wasmtime::StoreContextMut<'_, RecordState>, (func_idx,): (u32,)| {
                let event = R3Event::ImportCall { func_idx };
                postcard::to_io(&event, &mut store.data_mut().writer)
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                Ok(())
            },
        )?;
        r3.func_wrap(
            "record-memory-write",
            |mut store: wasmtime::StoreContextMut<'_, RecordState>,
             (addr, size, lo, hi): (u32, u32, u64, u64)| {
                let mut bytes = [0u8; 16];
                bytes[..8].copy_from_slice(&lo.to_le_bytes());
                bytes[8..16].copy_from_slice(&hi.to_le_bytes());
                let data = bytes[..size as usize].to_vec();
                let event = R3Event::MemoryWrite { addr, data };
                postcard::to_io(&event, &mut store.data_mut().writer)
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                Ok(())
            },
        )?;
    }

    let mut wasi_builder = WasiCtxBuilder::new();
    wasi_builder.inherit_stdio();
    wasi_builder.allow_blocking_current_thread(true);

    // argv[0] = component path, followed by the wasm arguments
    let mut full_args = vec![component_path.as_str()];
    full_args.extend(wasm_args);
    wasi_builder.args(&full_args);

    if args.env.is_empty() {
        wasi_builder.inherit_env();
    } else {
        for kv in &args.env {
            let (k, v) = kv
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("invalid env var (expected KEY=VALUE): {}", kv))?;
            wasi_builder.env(k, v);
        }
    }

    for dir_spec in &args.dir {
        let (host, guest) = dir_spec
            .split_once("::")
            .unwrap_or((dir_spec, dir_spec));
        wasi_builder.preopened_dir(host, guest, DirPerms::all(), FilePerms::all())?;
    }

    let wasi_ctx = wasi_builder.build();

    let state = RecordState {
        wasi_ctx,
        table: ResourceTable::new(),
        writer: match &args.trace {
            Some(path) => Box::new(BufWriter::new(File::create(path)?)),
            None => Box::new(std::io::sink()),
        },
    };

    let mut store = Store::new(&engine, state);
    let instance = linker.instantiate_async(&mut store, &component).await?;

    // Run via wasi:cli/run
    let command = wasmtime_wasi::p2::bindings::Command::new(&mut store, &instance)?;
    let result = command
        .wasi_cli_run()
        .call_run(&mut store)
        .await
        .context("failed to invoke `run` function")?;

    match result {
        Ok(()) => Ok(()),
        Err(()) => Err(wasmtime_wasi::I32Exit(1).into()),
    }
}
