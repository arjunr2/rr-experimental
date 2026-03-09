import subprocess
import argparse

parser = argparse.ArgumentParser(description="Run benchmark for wasmtime with RR support.")
parser.add_argument("--rr", type=str, help="Path to RR-supported Wasmtime", required=True)
parser.add_argument("--upstream", type=str, help="Path to upstream Wasmtime", required=True)

args = parser.parse_args()

hyperfine = "hyperfine --warmup 3 --shell=none".split(' ')

program_common = "--dir=data test-modules/components/wasi/compressor.wasm --input data/uncompressed-10M --output data/compressed.bin"

subprocess.run(
        hyperfine 
        + [f"-n \"{x}\"" for x in ["wasmtime-rr-record-enabled", "wasmtime-rr-record-disabled", "wasmtime-without-rr"]]
        + [' '.join([args.rr, "-R path=\"\"", program_common])]
        + [' '.join([args.rr, program_common])]
        + [' '.join([args.upstream, program_common])]
)
