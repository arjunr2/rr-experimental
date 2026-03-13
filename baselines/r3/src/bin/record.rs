//! Runner binary that executes an instrumented wasm module or component with
//! wasmtime, implements the r3 host functions, and writes a postcard-encoded trace.
//!
//! Auto-detects whether the input is a core module or component.

use anyhow::{Context, Result};
use clap::Parser;
use r3_baseline::{R3Event, SHADOW_MEMORY_EXPORT};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use wasmtime::component::{Component, Linker as ComponentLinker, ResourceTable};
use wasmtime::{Caller, Config, Engine, Linker, Module, Store};
use wasmtime_wasi::p1::{self, WasiP1Ctx};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

#[derive(Parser)]
#[command(name = "r3-record")]
#[command(about = "Run an instrumented wasm module or component and record an r3 trace")]
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

    /// Input instrumented wasm file (module or component), followed by its arguments
    #[arg(required = true, allow_hyphen_values = true)]
    wasm_and_args: Vec<String>,
}

enum WasmKind {
    Module,
    Component,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();

    let wasm_path = args
        .wasm_and_args
        .first()
        .ok_or_else(|| anyhow::anyhow!("missing wasm file path"))?;
    let wasm_args: Vec<&str> = args.wasm_and_args[1..]
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

    let wasm_bytes = std::fs::read(wasm_path)?;

    // Detect: precompiled artifacts first, then check binary preamble
    let kind = match Engine::detect_precompiled(&wasm_bytes) {
        Some(wasmtime::Precompiled::Module) => WasmKind::Module,
        Some(wasmtime::Precompiled::Component) => WasmKind::Component,
        None => {
            // Normalize text format to binary, then check byte 4
            let binary = wat::parse_bytes(&wasm_bytes)
                .map_err(|e| anyhow::anyhow!("wat parse error: {}", e))?;
            if binary.get(4) == Some(&0x0d) {
                WasmKind::Component
            } else {
                WasmKind::Module
            }
        }
    };

    // Build shared WASI context
    let mut wasi_builder = WasiCtxBuilder::new();
    wasi_builder.inherit_stdio();
    wasi_builder.allow_blocking_current_thread(true);

    let mut full_args = vec![wasm_path.as_str()];
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

    match kind {
        WasmKind::Module => {
            run_module(&engine, wasm_path, &wasm_bytes, wasi_builder, args.trace.as_deref()).await
        }
        WasmKind::Component => {
            run_component(&engine, wasm_path, &wasm_bytes, wasi_builder, args.trace.as_deref())
                .await
        }
    }
}

// --- Core module path (WASI p1) ---

struct ModuleRecordState {
    wasi: WasiP1Ctx,
    writer: Box<dyn Write + Send>,
}

async fn run_module(
    engine: &Engine,
    module_path: &str,
    wasm_bytes: &[u8],
    mut wasi_builder: WasiCtxBuilder,
    trace_path: Option<&Path>,
) -> Result<()> {
    let module = match Engine::detect_precompiled(wasm_bytes) {
        Some(wasmtime::Precompiled::Module) => unsafe {
            Module::deserialize(engine, wasm_bytes)?
        },
        _ => Module::from_file(engine, module_path)?,
    };

    let mut linker = Linker::<ModuleRecordState>::new(engine);
    p1::add_to_linker_async(&mut linker, |s| &mut s.wasi)?;

    // r3::record_import_call(func_idx: i32)
    linker.func_wrap(
        "r3",
        "record_import_call",
        |mut caller: Caller<'_, ModuleRecordState>, func_idx: i32| -> Result<()> {
            let state = caller.data_mut();
            let event = R3Event::ImportCall {
                func_idx: func_idx as u32,
            };
            postcard::to_io(&event, &mut state.writer)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            Ok(())
        },
    )?;

    // r3::record_memory_diff(addr: i32, size: i32)
    linker.func_wrap(
        "r3",
        "record_memory_diff",
        |mut caller: Caller<'_, ModuleRecordState>, addr: i32, size: i32| -> Result<()> {
            let addr = addr as u32;
            let size = size as usize;

            let mem0 = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .ok_or_else(|| anyhow::anyhow!("missing memory export"))?;
            let mem1 = caller
                .get_export(SHADOW_MEMORY_EXPORT)
                .and_then(|e| e.into_memory())
                .ok_or_else(|| anyhow::anyhow!("missing shadow memory export"))?;

            let start = addr as usize;

            // Copy both slices to stack (size is at most 16 for v128)
            let mut buf0 = [0u8; 16];
            let mut buf1 = [0u8; 16];
            buf0[..size].copy_from_slice(&mem0.data(&caller)[start..start + size]);
            buf1[..size].copy_from_slice(&mem1.data(&caller)[start..start + size]);

            let mut i = 0;
            while i < size {
                if buf0[i] == buf1[i] {
                    i += 1;
                    continue;
                }
                let run_start = i;
                while i < size && buf0[i] != buf1[i] {
                    i += 1;
                }
                let run = &buf0[run_start..i];
                mem1.write(&mut caller, start + run_start, run)?;
                let event = R3Event::MemoryWrite {
                    addr: addr + run_start as u32,
                    data: run.to_vec(),
                };
                postcard::to_io(&event, &mut caller.data_mut().writer)
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
            }
            Ok(())
        },
    )?;

    let wasi = wasi_builder.build_p1();
    let state = ModuleRecordState {
        wasi,
        writer: make_writer(trace_path)?,
    };

    let mut store = Store::new(engine, state);
    let instance = linker.instantiate_async(&mut store, &module).await?;

    let start = instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .or_else(|_| instance.get_typed_func::<(), ()>(&mut store, "main"))?;
    start.call_async(&mut store, ()).await?;

    Ok(())
}

// --- Component path (WASI p2) ---

struct ComponentRecordState {
    wasi_ctx: WasiCtx,
    table: ResourceTable,
    writer: Box<dyn Write + Send>,
}

impl WasiView for ComponentRecordState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi_ctx,
            table: &mut self.table,
        }
    }
}

async fn run_component(
    engine: &Engine,
    component_path: &str,
    wasm_bytes: &[u8],
    mut wasi_builder: WasiCtxBuilder,
    trace_path: Option<&Path>,
) -> Result<()> {
    let component = match Engine::detect_precompiled(wasm_bytes) {
        Some(wasmtime::Precompiled::Component) => unsafe {
            Component::deserialize(engine, wasm_bytes)?
        },
        _ => Component::from_file(engine, component_path)?,
    };

    let mut linker = ComponentLinker::<ComponentRecordState>::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;

    // r3 host functions — the instrumented wasm computes diffs and passes
    // raw bytes packed into scalar (lo, hi) i64 parameters.
    {
        let mut r3 = linker.instance("r3")?;
        r3.func_wrap(
            "record-import-call",
            |mut store: wasmtime::StoreContextMut<'_, ComponentRecordState>,
             (func_idx,): (u32,)| {
                let event = R3Event::ImportCall { func_idx };
                postcard::to_io(&event, &mut store.data_mut().writer)
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                Ok(())
            },
        )?;
        r3.func_wrap(
            "record-memory-write",
            |mut store: wasmtime::StoreContextMut<'_, ComponentRecordState>,
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

    let wasi_ctx = wasi_builder.build();
    let state = ComponentRecordState {
        wasi_ctx,
        table: ResourceTable::new(),
        writer: make_writer(trace_path)?,
    };

    let mut store = Store::new(engine, state);
    let instance = linker.instantiate_async(&mut store, &component).await?;

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

// --- Shared ---

fn make_writer(trace_path: Option<&Path>) -> Result<Box<dyn Write + Send>> {
    Ok(match trace_path {
        Some(path) => Box::new(BufWriter::new(File::create(path)?)),
        None => Box::new(std::io::sink()),
    })
}
