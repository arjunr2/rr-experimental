# x86 Benchmark Results: Record/Replay Tool Comparison (tmpfs)

## Overview

We compare six record/replay and tracing tools on eight benchmarks, measuring
execution time, trace size, and trace generation rate. All traces are written to
a 50 GB tmpfs mount (`/tmp/traces/`) to eliminate disk I/O as a confounding variable
and measure pure recording overhead.

## Environment

### Hardware

| Component | Details |
|-----------|---------|
| **CPU** | AMD Ryzen 9 7950X (16 cores, 32 threads, Zen 4, AVX-512), 4.5 GHz base / 5.7 GHz boost |
| **RAM** | 64 GB DDR5 (62 GiB usable) |
| **Storage** | Sabrent Rocket 4 Plus 2 TB NVMe SSD (not used for traces — see tmpfs below) |
| **Motherboard** | Gigabyte B650M AORUS ELITE AX |
| **Trace storage** | 50 GB tmpfs at `/tmp/traces/` (`mount -t tmpfs -o size=50G tmpfs /tmp/traces/`) |

### Software

| Component | Version |
|-----------|---------|
| **OS** | Ubuntu 24.04.3 LTS (Noble Numbat) |
| **Kernel** | 6.14.0-33-generic |
| **Rust** | rustc 1.95.0-nightly (366a1b93e 2026-02-03) |
| **Cargo** | 1.95.0-nightly (fe2f314ae 2026-01-30) |
| **GCC (x86)** | 13.3.0 (Ubuntu 13.3.0-6ubuntu2~24.04) |
| **GCC (aarch64 cross)** | aarch64-linux-gnu-gcc 13.3.0 |
| **Python** | 3.12.3 |
| **wasi-sdk** | 27.0 (LLVM 20.1.8) |
| **wasm-tools** | 1.239.0 |
| **ftzz** | 4.0.0 |
| **zstd CLI** | 1.5.5 |

### Recording Tools

| Tool | Version / Commit |
|------|-----------------|
| **wasmtime** (with rr support) | 40.0.0 (commit 17956d0d5, 2026-02-04), branch `crimp-dev` from `wasmtime-rr-prototyping` repo |
| **rr** | 5.7.0 |
| **r3** (r3-record, r3-instrument) | Commit b0d1df4 from `rr-experimental` repo |
| **Intel SDE / PinPlay** | 10.7.0 external |
| **QEMU** | 8.2.2 (Debian 1:8.2.2+ds-0ubuntu1.13) |
| **SQLite** (speedtest1) | 3.47.2 (2024-12-07) |

### Compilation Flags

- **Native x86 binaries (Rust)**: `RUSTFLAGS="-C target-cpu=native"` (Zen 4, AVX-512)
- **WASM binaries (Rust)**: `RUSTFLAGS="-C target-feature=+simd128"` targeting `wasm32-wasip2`
- **WASM binaries (C, speedtest1)**: wasi-sdk with `-O2 -msimd128 -DSQLITE_THREADSAFE=0`
- **WASM binaries (C, blake3)**: wasi-sdk with `-O2` (no WASM SIMD — no intrinsics available)
- **aarch64 binaries (Rust, for QEMU)**: `cargo zigbuild --target aarch64-unknown-linux-musl --release` (static musl)
- **aarch64 binaries (C, speedtest1)**: `aarch64-linux-gnu-gcc -static -O2 -DSQLITE_THREADSAFE=0`
- **QEMU VM kernel**: Alpine Linux aarch64 kernel (EFI stub), busybox-static initramfs, 16 GB guest RAM

## Tools

| Tool | Records | Target | Mechanism |
|------|---------|--------|-----------|
| **wasmtime-rr** | WASI component events | WASM binary via wasmtime | Userspace hooks in wasmtime runtime; serializes each WASI call to a trace stream |
| **wasmtime-rr-threaded** | Same as wasmtime-rr | Same | Same, but with a background writer thread (`buffer-size=8192,threaded,channels=128`) |
| **rr** | Syscalls + signals | Native x86 binary | Linux `ptrace`; intercepts every syscall with a kernel context switch |
| **r3** | WASI events via instrumented WASM | WASM binary via r3-record | Static binary rewriting adds shadow memory instrumentation before execution |
| **pinplay** | Every x86 instruction | Native x86 binary | Intel SDE (Software Development Emulator); logs full instruction stream |
| **qemu** | Deterministic replay events | Native aarch64 binary in QEMU VM | Records only non-deterministic inputs (I/O, interrupts, timers); replays by re-executing from initial state |

**Key distinction**: wasmtime-rr, rr, and r3 record at the **event level** (WASI calls
or syscalls). PinPlay records at the **instruction level**. QEMU records only
**non-deterministic inputs** — the smallest trace by far, but requires full
re-execution for replay.

## Benchmarks

| Benchmark | Description | Input | Iterations | Native time |
|-----------|-------------|-------|------------|-------------|
| compress-l3 | zstd compression, level 3 (I/O-bound) | 1 GB ftzz (1000 files) | 12 | 4.0s |
| compress-l9 | zstd compression, level 9 (mixed) | 1 GB ftzz (1000 files) | 9 | 3.7s |
| compress-l19 | zstd compression, level 19 (CPU-bound) | 62 MB ftzz (620 files) | 1 | 8.7s |
| sort | External merge sort (I/O-heavy) | 1.7 GB text (18M lines) | 1 | 9.1s |
| regex-redux | Regex matching on DNA (CPU-bound) | 1019 MB FASTA | 1 | 10.7s |
| json-process | Python JSON processing (interpreter) | 285 MB JSON (2M records) | 1 | 12.3s |
| speedtest1 | SQLite benchmark (I/O-heavy) | --size 300 | 1 | 9.1s |
| blake3 | BLAKE3 cryptographic hash (CPU-bound) | 100 MB random | 150 | 3.2s |

The compressor uses a single zstd encoder streaming all files into one compressed
output. The `-n` flag loops the full cycle (re-create encoder, re-read all files).
Blake3 also uses an iterations parameter to re-read and re-hash the input file.

Native Python (json-process) uses C extensions (`_json`) which are unavailable in
the WASM CPython build. This contributes to the wasmtime overhead for json-process.

WASM binaries are compiled with `RUSTFLAGS="-C target-feature=+simd128"` (128-bit
WASM SIMD) for Rust benchmarks. C WASM binaries (speedtest1) use `-msimd128`.
Blake3 WASM uses portable C only (no WASM SIMD intrinsics available).
Native x86 binaries are compiled with `-C target-cpu=native` (Zen 4, AVX-512).

## Results

### compress-l3 (1 GB input, 12 iterations)

| Metric | Native | Wasmtime | wasmtime-rr | wtrr-threaded | rr | r3 | pinplay | qemu |
|--------|--------|----------|-------------|---------------|------|------|---------|------|
| **Time (s)** | 4.0 | 18.1 | 70.1 | 97.2 | 93.6 | 345.8 | 2095.9 | 46.1 |
| **Trace size** | — | — | 12G | 12G | 12G | 4.7G | 27G | 15M |
| **Peak rate (MB/s)** | — | — | 203.1 | 146.0 | 459.0 | 19.1 | 28.3 | 5.5 |
| **Avg rate (MB/s)** | — | — | 164.3 | 118.6 | 122.7 | 13.8 | 13.1 | 0.3 |

### compress-l9 (1 GB input, 9 iterations)

| Metric | Native | Wasmtime | wasmtime-rr | wtrr-threaded | rr | r3 | pinplay | qemu |
|--------|--------|----------|-------------|---------------|------|------|---------|------|
| **Time (s)** | 3.7 | 16.6 | 56.8 | 75.9 | 76.1 | 401.7 | 1729.1 | 50.1 |
| **Trace size** | — | — | 8.5G | 8.5G | 8.5G | 4.8G | 21G | 11M |
| **Peak rate (MB/s)** | — | — | 185.8 | 135.9 | 425.3 | 18.7 | 29.7 | 5.2 |
| **Avg rate (MB/s)** | — | — | 152.0 | 113.8 | 113.1 | 12.1 | 11.9 | 0.2 |

### compress-l19 (62 MB input, 1 iteration)

| Metric | Native | Wasmtime | wasmtime-rr | wtrr-threaded | rr | r3 | pinplay | qemu |
|--------|--------|----------|-------------|---------------|------|------|---------|------|
| **Time (s)** | 8.7 | 10.7 | 11.5 | 11.7 | 9.9 | 54.7 | 186.7 | 37.3 |
| **Trace size** | — | — | 60M | 60M | 60M | 409M | 149M | 1.1M |
| **Peak rate (MB/s)** | — | — | 31.0 | 21.0 | 416.7 | 13.1 | 27.3 | 5.5 |
| **Avg rate (MB/s)** | — | — | 5.2 | 5.1 | 6.0 | 7.5 | 0.8 | 0.03 |

### sort (1.7 GB input, 18M lines)

| Metric | Native | Wasmtime | wasmtime-rr | wtrr-threaded | rr | r3 | pinplay | qemu |
|--------|--------|----------|-------------|---------------|------|------|---------|------|
| **Time (s)** | 9.1 | 20.3 | 60.5 | 52.1 | 1404.5 | 362.3 | 749.7 | 221.5 |
| **Trace size** | — | — | 3.9G | 3.9G | 3.5G | 4.8G | 12G | 225M |
| **Peak rate (MB/s)** | — | — | 197.9 | 123.9 | 286.5 | 17.7 | 117.0 | 5.2 |
| **Avg rate (MB/s)** | — | — | 65.5 | 76.0 | 2.5 | 13.4 | 16.3 | 1.0 |

### regex-redux (1019 MB FASTA input)

| Metric | Native | Wasmtime | wasmtime-rr | wtrr-threaded | rr | r3 | pinplay | qemu |
|--------|--------|----------|-------------|---------------|------|------|---------|------|
| **Time (s)** | 10.7 | 24.2 | 28.6 | 30.6 | 16.1 | 194.1 | 423.2 | 162.1 |
| **Trace size** | — | — | 1019M | 1019M | 444M | 2.7G | 2.5G | 2.3M |
| **Peak rate (MB/s)** | — | — | 309.2 | 192.7 | 290.7 | 22.3 | 175.6 | 5.5 |
| **Avg rate (MB/s)** | — | — | 35.3 | 33.1 | 27.3 | 14.1 | 5.0 | 0.01 |

### json-process (285 MB JSON, 2M records, Python)

| Metric | Native | Wasmtime | wasmtime-rr | wtrr-threaded | rr | r3 | pinplay | qemu |
|--------|--------|----------|-------------|---------------|------|------|---------|------|
| **Time (s)** | 12.3 | 20.3 | 21.8 | 22.3 | 16.3 | 80.8 | 1242.5 | 225.6 |
| **Trace size** | — | — | 286M | 286M | 38M | 731M | 732M | 9.7M |
| **Peak rate (MB/s)** | — | — | 322.3 | 179.0 | 155.9 | 21.5 | 180.5 | 6.3 |
| **Avg rate (MB/s)** | — | — | 12.6 | 12.3 | 2.2 | 8.7 | 0.6 | 0.04 |

### speedtest1 (SQLite, --size 300)

| Metric | Native | Wasmtime | wasmtime-rr | wtrr-threaded | rr | r3 | pinplay | qemu |
|--------|--------|----------|-------------|---------------|------|------|---------|------|
| **Time (s)** | 9.1 | 32.7 | 141.0 | 160.5 | 395.0 | 742.0 | 962.4 | 119.4 |
| **Trace size** | — | — | 19G | 19G | 4.3G | 7.6G | 24G | 60M |
| **Peak rate (MB/s)** | — | — | 199.2 | 169.8 | 153.6 | 20.1 | 113.1 | 5.2 |
| **Avg rate (MB/s)** | — | — | 133.3 | 117.0 | 10.9 | 10.4 | 25.3 | 0.5 |

### blake3 (100 MB input, 150 iterations)

| Metric | Native | Wasmtime | wasmtime-rr | wtrr-threaded | rr | r3 | pinplay | qemu |
|--------|--------|----------|-------------|---------------|------|------|---------|------|
| **Time (s)** | 3.2 | 34.9 | 91.1 | 132.5 | 19.9 | 236.3 | 187.9 | 80.4 |
| **Trace size** | — | — | 15G | 15G | 15G | 2.1G | 35G | 2.8M |
| **Peak rate (MB/s)** | — | — | 192.2 | 131.6 | 1076.8 | 10.3 | 214.3 | 5.5 |
| **Avg rate (MB/s)** | — | — | 164.7 | 113.3 | 753.4 | 8.9 | 188.7 | 0.03 |

## Summary Tables

### Recording overhead (time relative to native)

| Benchmark | Wasmtime | wasmtime-rr | wtrr-threaded | rr | r3 | pinplay | qemu |
|-----------|----------|-------------|---------------|------|------|---------|------|
| compress-l3 | 4.6x | 17.7x | 24.5x | 23.6x | 87x | 529x | 11.6x |
| compress-l9 | 4.4x | 15.2x | 20.3x | 20.4x | 107x | 463x | 13.5x |
| compress-l19 | 1.2x | 1.3x | 1.3x | 1.1x | 6.3x | 21.5x | 4.3x |
| sort | 2.2x | 6.6x | 5.7x | 154x | 40x | 82x | 24.3x |
| regex-redux | 2.3x | 2.7x | 2.9x | 1.5x | 18.2x | 40x | 15.2x |
| json-process | 1.7x | 1.8x | 1.8x | 1.3x | 6.6x | 101x | 18.3x |
| speedtest1 | 3.6x | 15.5x | 17.6x | 43.4x | 81.5x | 105.8x | 13.1x |
| blake3 | 10.9x | 28.5x | 41.4x | 6.2x | 73.8x | 58.7x | 25.1x |

### wasmtime-rr recording overhead (time relative to wasmtime, no recording)

| Benchmark | wasmtime-rr / wasmtime | wtrr-threaded / wasmtime |
|-----------|----------------------|--------------------------|
| compress-l3 | 3.9x | 5.4x |
| compress-l9 | 3.4x | 4.6x |
| compress-l19 | 1.07x | 1.09x |
| sort | 3.0x | 2.6x |
| regex-redux | 1.2x | 1.3x |
| json-process | 1.1x | 1.1x |
| speedtest1 | 4.3x | 4.9x |
| blake3 | 2.6x | 3.8x |

### wasmtime-rr threaded vs non-threaded

Values < 1.0 = threaded is faster. Values > 1.0 = threaded is slower.

| Benchmark | threaded / non-threaded |
|-----------|------------------------|
| compress-l3 | 1.39x |
| compress-l9 | 1.34x |
| compress-l19 | 1.01x |
| sort | 0.86x |
| regex-redux | 1.07x |
| json-process | 1.02x |
| speedtest1 | 1.14x |
| blake3 | 1.45x |

Threading helps only for sort (I/O-heavy, 14% faster). For all other benchmarks,
thread synchronization overhead exceeds any benefit, since tmpfs writes are already
near-instant. This contrasts with remote/network recording where the threaded writer
provides significant benefits by decoupling serialization from I/O latency.

### Trace sizes

| Benchmark | wasmtime-rr | rr | r3 | pinplay | qemu |
|-----------|------------|------|------|---------|------|
| compress-l3 | 12G | 12G | 4.7G | 27G | 15M |
| compress-l9 | 8.5G | 8.5G | 4.8G | 21G | 11M |
| compress-l19 | 60M | 60M | 409M | 149M | 1.1M |
| sort | 3.9G | 3.5G | 4.8G | 12G | 225M |
| regex-redux | 1019M | 444M | 2.7G | 2.5G | 2.3M |
| json-process | 286M | 38M | 731M | 732M | 9.7M |
| speedtest1 | 19G | 4.3G | 7.6G | 24G | 60M |
| blake3 | 15G | 15G | 2.1G | 35G | 2.8M |

Note: wasmtime-rr threaded produces identical trace sizes to non-threaded (same data, different write path).

### QEMU pure emulation overhead (no icount recording)

QEMU aarch64 TCG emulation without `-icount` recording. Measures the baseline
emulation cost before adding deterministic replay overhead.

| Benchmark | QEMU wall (s) | QEMU internal (s) | icount wall (s) | icount overhead |
|-----------|--------------|-------------------|-----------------|-----------------|
| compress-l3 | 32.0 | 26.9 | 46.1 | 1.44x |
| compress-l9 | 34.3 | 29.1 | 50.1 | 1.46x |
| compress-l19 | 30.1 | 28.3 | 37.3 | 1.24x |
| sort | 157.7 | 138.3 | 221.5 | 1.40x |
| regex-redux | 136.1 | 125.1 | 162.1 | 1.19x |
| json-process | 174.8 | 166.0 | 225.6 | 1.29x |
| speedtest1 | 98.3 | 96.9 | 119.4 | 1.21x |
| blake3 | 72.4 | 70.7 | 80.4 | 1.11x |

QEMU wall time includes VM boot/shutdown overhead (~1-5s). QEMU internal time is
from the benchmark's own METRICS output (excludes boot).

icount overhead ranges from 1.11x (blake3, CPU-bound) to 1.46x (compress-l9, I/O-heavy).
The previous sort QEMU result (68.4s, 8.3M trace) was incorrect — the sort binary
crashed with EMFILE (too many open files) inside the VM. Fixed with `ulimit -n 65536`.

## Methodology

### Trace storage

All traces are written to a 50 GB tmpfs mount at `/tmp/traces/`. This eliminates
disk I/O latency and bandwidth as confounding variables, allowing us to measure the
pure computational overhead of each recording tool. Previous results with traces on
NVMe SSD are preserved in `RESULTS_disk.md`.

### Trace rate measurement

Trace generation rates (peak and average, in MB/s) are measured by polling the
trace file or directory size using `du -sb` every 10 milliseconds from a background
shell loop while the recording tool runs. Peak rate is the maximum instantaneous
rate observed across all 10ms windows. Average rate is total trace bytes divided
by total elapsed time. This approach works for all tools since they all write
traces to tmpfs (file or directory).

### Tool execution

- **Native**: the benchmark binary compiled for x86-64 with `-C target-cpu=native`
  (Zen 4, AVX-512 where applicable). Used as the baseline.
- **Wasmtime**: the benchmark compiled to `wasm32-wasip2`, run via wasmtime with
  `--dir` filesystem mapping. No recording. Measures the WASM execution overhead.
- **wasmtime-rr**: same as wasmtime but with `-R path=<trace>,buffer-size=8192`.
  Recording is synchronous (non-threaded) with an 8 KB serialization buffer.
- **wasmtime-rr-threaded**: same as wasmtime-rr but with
  `-R path=<trace>,buffer-size=8192,threaded,channels=128`. A background thread
  handles trace writes, using 128 channels of 8 KB each (1 MB total buffer).
- **rr**: `rr record -n -o <trace_dir> <native_binary> <args>`. Records the native
  x86 process. The `-n` flag disables syscall buffering for deterministic traces.
- **r3**: `r3-record --trace <file> --dir=<workdir>::/work <instrumented.wasm> <args>`.
  The WASM binary is pre-instrumented with `r3-instrument` which adds shadow memory
  tracking. Uses `r3-instrument` and `r3-record` (not the `-component` variants).
- **pinplay**: `sde64 -log -log:basename <trace_dir>/pp -- <native_binary> <args>`.
  Intel SDE records every x86 instruction executed.
- **qemu**: the benchmark cross-compiled to aarch64 (static musl), embedded in a
  minimal Linux initramfs (Alpine kernel + busybox), run in QEMU with
  `-icount shift=auto,rr=record,rrfile=<trace>`. Guest RAM: 16 GB.
  Output written to `/dev/null` inside the guest (requires `mount -t devtmpfs`).
  QEMU traces are also written to tmpfs for consistency.

### Input data generation

- **ftzz data** (compressor l3/l9): `ftzz -n 1000 -b 1000000000 -d 10 --seed 15213 --exact <dir>` (1 GB, 1000 files)
- **ftzz data** (compressor l19): `ftzz -n 620 -b 62000000 -d 8 --seed 15213 --exact <dir>` (62 MB, 620 files — smaller input because l19 is ~16× slower per byte)
- **Sort data**: Python script generating random alphanumeric lines (seed 42)
- **FASTA data** (regex-redux): Python script generating random DNA bases (seed 42)
- **JSON data** (json-process): Python script generating records with id, name, email, score, tags, active fields (seed 42)
- **Blake3 input**: `dd if=/dev/urandom of=blake3_input.bin bs=1M count=100`
- **Speedtest1**: self-contained (SQLite generates its own data internally)

## Reproducing

### 1. Set up tmpfs

```bash
sudo mount -t tmpfs -o size=50G tmpfs /tmp/traces/
```

Requires ≥50 GB free RAM. PinPlay traces can exceed 30 GB for a single benchmark.

### 2. Generate input data

```bash
# Compressor l3/l9 (1 GB, 1000 files)
ftzz -n 1000 -b 1000000000 -d 10 --seed 15213 --exact rr/test_data/

# Compressor l19 (62 MB, 620 files)
ftzz -n 620 -b 62000000 -d 8 --seed 15213 --exact rr/test_data_l19/

# Sort (1.7 GB, 18M lines) — deterministic via seed
python3 -c "
import random, string
random.seed(42)
with open('rr/sort_data.txt','w') as f:
    for _ in range(18_000_000):
        f.write(''.join(random.choices(string.ascii_letters+string.digits,k=random.randint(20,100)))+'\n')
"

# FASTA (1019 MB) — deterministic via seed
python3 -c "
import random
random.seed(42)
bases='ACGT'
with open('rr/fasta_input.txt','w') as f:
    f.write('>seq\n')
    remaining=1019*1024*1024
    while remaining>0:
        line=''.join(random.choices(bases,k=min(60,remaining)))
        f.write(line+'\n')
        remaining-=len(line)
"

# JSON (285 MB, 2M records) — deterministic via seed
python3 -c "
import json,random,string
random.seed(42)
with open('rr/json_input.json','w') as f:
    f.write('[')
    for i in range(2_000_000):
        if i: f.write(',')
        r={'id':i,'name':''.join(random.choices(string.ascii_letters,k=10)),
           'email':''.join(random.choices(string.ascii_lowercase,k=8))+'@example.com',
           'score':round(random.uniform(0,100),2),
           'tags':[random.choice(['a','b','c','d','e']) for _ in range(3)],
           'active':random.choice([True,False])}
        json.dump(r,f)
    f.write(']')
"

# Blake3 (100 MB random)
dd if=/dev/urandom of=blake3-bench/blake3_input.bin bs=1M count=100
```

### 3. Build benchmarks

```bash
# Native x86 (Rust benchmarks)
cd compressor && RUSTFLAGS="-C target-cpu=native" cargo build --release && cd ..
cd external-sort && RUSTFLAGS="-C target-cpu=native" cargo build --release && cd ..
cd regex-redux && RUSTFLAGS="-C target-cpu=native" cargo build --release && cd ..

# WASM (Rust benchmarks)
cd compressor && RUSTFLAGS="-C target-feature=+simd128" cargo build --release --target wasm32-wasip2 && cd ..
cd external-sort && RUSTFLAGS="-C target-feature=+simd128" cargo build --release --target wasm32-wasip2 && cd ..
cd regex-redux && RUSTFLAGS="-C target-feature=+simd128" cargo build --release --target wasm32-wasip2 && cd ..

# speedtest1 (C, native x86)
gcc -O2 -march=native -DSQLITE_THREADSAFE=0 -o speedtest1/speedtest1-x86_64 \
    speedtest1/speedtest1.c speedtest1/sqlite3.c -lpthread -ldl -lm

# speedtest1 (C, WASM)
$WASI_SDK/bin/clang -O2 -msimd128 -DSQLITE_THREADSAFE=0 \
    -o speedtest1/speedtest1.wasm speedtest1/speedtest1.c speedtest1/sqlite3.c -lm

# blake3 (C, native x86)
gcc -O2 -march=native -o blake3-bench/blake3-bench-x86_64 \
    blake3-bench/bench.c blake3-bench/blake3.c blake3-bench/blake3_dispatch.c \
    blake3-bench/blake3_portable.c

# blake3 (C, WASM — no SIMD)
$WASI_SDK/bin/clang -O2 -o blake3-bench/blake3-bench.wasm \
    blake3-bench/bench.c blake3-bench/blake3.c blake3-bench/blake3_dispatch.c \
    blake3-bench/blake3_portable.c

# R3 instrumented WASM (for each benchmark)
r3-instrument <benchmark>.wasm -o r3/<benchmark>.instrumented.wasm

# aarch64 for QEMU (Rust, static musl)
cargo zigbuild --target aarch64-unknown-linux-musl --release

# aarch64 for QEMU (speedtest1, static)
aarch64-linux-gnu-gcc -static -O2 -DSQLITE_THREADSAFE=0 \
    -o speedtest1/speedtest1-aarch64 speedtest1/speedtest1.c speedtest1/sqlite3.c -lpthread -ldl -lm
```

### 4. Run benchmarks

Each benchmark is run once per tool. Example commands for compress-l3:

```bash
TRACES=/tmp/traces
WT=path/to/wasmtime   # wasmtime with rr support (crimp-dev branch)

# Native
./compressor -i rr/test_data -o /dev/null -l 3 -n 12

# Wasmtime (no recording)
$WT --dir=rr/test_data::/work/input --dir=/tmp::/work/output \
    compressor.wasm -i /work/input -o /work/output/out.zst -l 3 -n 12

# wasmtime-rr (non-threaded)
$WT -R path=$TRACES/trace.bin,buffer-size=8192 \
    --dir=rr/test_data::/work/input --dir=/tmp::/work/output \
    compressor.wasm -i /work/input -o /work/output/out.zst -l 3 -n 12

# wasmtime-rr (threaded)
$WT -R path=$TRACES/trace.bin,buffer-size=8192,threaded,channels=128 \
    --dir=rr/test_data::/work/input --dir=/tmp::/work/output \
    compressor.wasm -i /work/input -o /work/output/out.zst -l 3 -n 12

# rr
rr record -n -o $TRACES/rr_trace ./compressor -i rr/test_data -o /dev/null -l 3 -n 12

# r3
r3-record --trace $TRACES/r3.bin \
    --dir=rr/test_data::/work/input --dir=/tmp::/work/output \
    r3/compressor.instrumented.wasm -i /work/input -o /work/output/out.zst -l 3 -n 12

# PinPlay
sde64 -log -log:basename $TRACES/pp/pp -- ./compressor -i rr/test_data -o /dev/null -l 3 -n 12

# QEMU (aarch64 in VM with icount recording)
qemu-system-aarch64 -M virt -cpu cortex-a57 -m 16384M -no-reboot \
    -kernel vmlinuz-aarch64 -initrd initrd.cpio.gz \
    -append "console=ttyAMA0 quiet loglevel=0 rdinit=/init" \
    -nographic -serial /dev/null -monitor none \
    -icount "shift=auto,rr=record,rrfile=$TRACES/qemu.bin"
```

### 5. Measure trace rate

Trace write rate is measured by polling `du -sb` on the trace path every 10 ms
from a background loop while the recording tool runs:

```bash
# run_bench.sh <trace_path> <command...>
TRACE_PATH=$1; shift
"$@" &
PID=$!
PREV_SIZE=0; PREV_T=$(date +%s%3N); PEAK=0
while kill -0 $PID 2>/dev/null; do
    sleep 0.01
    CUR_SIZE=$(du -sb "$TRACE_PATH" 2>/dev/null | cut -f1) || CUR_SIZE=$PREV_SIZE
    CUR_T=$(date +%s%3N)
    DS=$((CUR_SIZE - PREV_SIZE)); DT=$((CUR_T - PREV_T))
    if [ "$DT" -gt 0 ] && [ "$DS" -gt 0 ]; then
        BPS=$((DS * 1000 / DT))
        [ "$BPS" -gt "$PEAK" ] && PEAK=$BPS
    fi
    PREV_SIZE=$CUR_SIZE; PREV_T=$CUR_T
done
wait $PID
FINAL=$(du -sb "$TRACE_PATH" 2>/dev/null | cut -f1)
ELAPSED=$(( $(date +%s%3N) - START ))
# peak = PEAK bytes/s, avg = FINAL / ELAPSED
```

### Notes

- Clean `$TRACES` between runs (`rm -rf /tmp/traces/*`) to ensure accurate trace sizes.
- QEMU initramfs must include `mount -t devtmpfs none /dev` for `/dev/null` access.
- The QEMU VM kernel is an Alpine Linux aarch64 kernel with a minimal busybox-static initramfs.
- json-process uses CPython 3.12 compiled for WASM (via wasi-sdk) and for aarch64 (musl static).
  Native x86 json-process runs the same script under the system Python 3.12.
