//! Runner binary that executes an instrumented wasm module with wasmtime,
//! implements the r3 host functions, and writes a postcard-encoded trace.

use anyhow::Result;
use clap::Parser;
use r3_baseline::{R3Event, SHADOW_MEMORY_EXPORT};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use wasmtime::{Caller, Engine, Linker, Module, Store};
use wasmtime_wasi::p1::{self, WasiP1Ctx};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtxBuilder};

#[derive(Parser)]
#[command(name = "r3-record")]
#[command(about = "Run an instrumented wasm module and record an r3 trace")]
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

    /// Input instrumented core wasm module, followed by its arguments
    #[arg(required = true, allow_hyphen_values = true)]
    module_and_args: Vec<String>,
}

struct RecordState {
    wasi: WasiP1Ctx,
    last_import_idx: u32,
    writer: Box<dyn Write + Send>,
}

fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();

    let module_path = args
        .module_and_args
        .first()
        .ok_or_else(|| anyhow::anyhow!("missing module path"))?;
    let wasm_args: Vec<&str> = args.module_and_args[1..]
        .iter()
        .map(|s| s.as_str())
        .collect();

    let engine = Engine::default();
    let module = Module::from_file(&engine, module_path)?;

    let mut linker = Linker::<RecordState>::new(&engine);

    // Add WASI p1 host functions
    p1::add_to_linker_sync(&mut linker, |s| &mut s.wasi)?;

    // r3::record_import_call(func_idx: i32)
    linker.func_wrap(
        "r3",
        "record_import_call",
        |mut caller: Caller<'_, RecordState>, func_idx: i32| -> Result<()> {
            let state = caller.data_mut();
            state.last_import_idx = func_idx as u32;
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
        |mut caller: Caller<'_, RecordState>, addr: i32, size: i32| -> Result<()> {
            let addr = addr as u32;
            let size = size as u32;

            let mem0 = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .ok_or_else(|| anyhow::anyhow!("missing memory export"))?;
            let mem1 = caller
                .get_export(SHADOW_MEMORY_EXPORT)
                .and_then(|e| e.into_memory())
                .ok_or_else(|| anyhow::anyhow!("missing shadow memory export"))?;

            let data0 = mem0.data(&caller);
            let data1 = mem1.data(&caller);
            let start = addr as usize;
            let end = start + size as usize;

            let differing: Vec<(u8, u8)> = data0[start..end]
                .iter()
                .zip(&data1[start..end])
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .map(|(i, (a, _))| (i as u8, *a))
                .collect();

            if !differing.is_empty() {
                let state = caller.data_mut();
                let event = R3Event::MemoryWrite {
                    func_idx: state.last_import_idx,
                    addr,
                    data: differing,
                };
                postcard::to_io(&event, &mut state.writer)
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
            }
            Ok(())
        },
    )?;

    let mut wasi_builder = WasiCtxBuilder::new();
    wasi_builder.inherit_stdio();

    // argv[0] = module path, followed by the wasm arguments
    let mut full_args = vec![module_path.as_str()];
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

    let wasi = wasi_builder.build_p1();

    let state = RecordState {
        wasi,
        last_import_idx: 0,
        writer: match &args.trace {
            Some(path) => Box::new(BufWriter::new(File::create(path)?)),
            None => Box::new(std::io::sink()),
        },
    };

    let mut store = Store::new(&engine, state);
    let instance = linker.instantiate(&mut store, &module)?;

    let start = instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .or_else(|_| instance.get_typed_func::<(), ()>(&mut store, "main"))?;
    start.call(&mut store, ())?;

    Ok(())
}
