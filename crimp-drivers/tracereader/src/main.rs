use std::fs::File;
use std::io::BufReader;

use anyhow::Result;
use clap::Parser;
use wasm_crimp::{RREvent, ReplaySettings, from_replay_reader};

#[derive(Parser)]
#[command(about = "Read and print all events from a wasm-crimp trace file")]
struct Args {
    /// Path to the trace file
    trace: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let file = File::open(&args.trace)?;
    let mut reader = BufReader::new(file);
    let settings = ReplaySettings::default();
    let mut scratch = vec![0u8; settings.deserialize_buffer_size];

    let mut count = 0u64;
    loop {
        let event = from_replay_reader(&mut reader, &mut scratch)?;
        println!("[{count}] {event}");
        if matches!(event, RREvent::Eof) {
            break;
        }
        count += 1;
    }

    println!("\nTotal events: {count}");
    Ok(())
}
