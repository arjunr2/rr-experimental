use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::error::Error;

use clap::Parser;
use walkdir::WalkDir;

/// Compress all files in a directory using Zstandard
#[derive(Parser, Debug)]
#[command(version, about = "Compresses all files in a directory using zstd", long_about = None)]
struct Args {
    /// Path to the input directory
    #[arg(short = 'i', long = "input", value_name = "DIR")]
    input_dir: PathBuf,

    /// Path to the output file
    #[arg(short = 'o', long = "output", value_name = "FILE")]
    output_file: PathBuf,

    /// Compression level (1–21)
    #[arg(short, long, default_value_t = 3)]
    level: i32,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    let mut f = BufWriter::new(File::create(args.output_file)?);
    let mut encoder = zstd::Encoder::new(&mut f, args.level)?;
    for entry in WalkDir::new(&args.input_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let input_file = File::open(entry.path())?;
        std::io::copy(&mut BufReader::new(input_file), &mut encoder)?;
    }
    encoder.finish()?;
    Ok(())
}
