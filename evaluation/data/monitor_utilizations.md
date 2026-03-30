# Monitor Overhead: speedtest1 (crimp_driver_mono filtered)

## Overview

We measure the CPU overhead of Wizard Engine monitors during deterministic replay
of speedtest1 (SQLite benchmark) streamed from 5 remote devices. All monitors skip
functions starting with `crimp_driver_mono::` (replay driver functions).

speedtest1 was chosen because it has moderate replay CPU utilization (~48% on
mac-mini in the offloading benchmarks), providing headroom to observe monitor
overhead on fast devices — unlike sort-10k where even baseline replay saturated
the CPU on Gigabit-connected devices.

## Benchmark

**speedtest1**: SQLite 3.47.2 benchmark (`--size 10`). Self-contained — generates
its own data internally. Tests INSERTs, SELECTs (indexed, unindexed, LIKE, ORDER BY),
JOINs, REPLACE, DELETE, VACUUM, ANALYZE, and integrity_check. Trace size: ~10.6 MB.

## Environment

### Local machine (replay host)

| Component | Details |
|-----------|---------|
| **CPU** | AMD Ryzen 9 7950X, 16 cores / 32 threads, Zen 4 |
| **RAM** | 64 GB DDR5 |
| **OS** | Ubuntu 24.04.3 LTS, kernel 6.14.0-33-generic |

### Remote devices

| Device | CPU | Cores | Clock | Network | Link speed | Recording |
|--------|-----|-------|-------|---------|-----------|-----------|
| milkv-duo | T-Head C906 (RISC-V) | 1 | ~1 GHz | Fast Ethernet | 94 Mbps | non-threaded |
| pi0 | ARM Cortex-A53 | 4 | 1 GHz | WiFi | 35 Mbps | threaded |
| nuc11 | Intel i7-1165G7 | 4C/8T | 4.7 GHz | Gigabit Ethernet | 902 Mbps | threaded |
| aplos | AMD Ryzen 5 4500U | 6 | 2 GHz | Gigabit Ethernet | 941 Mbps | threaded |
| mac-mini | Apple M2 Pro | 10 (6P+4E) | 3.5 GHz | Gigabit Ethernet | 941 Mbps | threaded |

### Software

| Component | Version |
|-----------|---------|
| **wasmtime-rr** | 40.0.0 (commit 17956d0d5, branch `crimp-dev`) |
| **Wizard Engine** | 26.2944 (modified: crimp_driver_mono filter) |

## Results: Individual Monitors (filtered)

Each monitor runs alone. Values are medians across 5 runs.

### CPU utilization (%)

| Monitor | milkv-duo | pi0 | nuc11 | aplos | mac-mini |
|---------|-----------|-----|-------|-------|----------|
| no-monitors | 1.9 | 6.9 | 18.5 | 75.9 | 59.0 |
| icount | 3.0 | 13.6 | 37.9 | 99.6 | 99.0 |
| loops | 1.9 | 7.0 | 19.0 | 77.7 | 61.5 |
| hotness | 2.9 | 12.9 | 33.8 | 100.0 | 100.0 |
| profile{dot} | 7.3 | 37.7 | 91.1 | 99.6 | 99.4 |

### Wall time (seconds)

| Monitor | milkv-duo | pi0 | nuc11 | aplos | mac-mini |
|---------|-----------|-----|-------|-------|----------|
| no-monitors | 131.9 | 22.9 | 7.5 | 1.5 | 1.9 |
| icount | 131.9 | 23.0 | 7.7 | 2.7 | 2.7 |
| loops | 131.9 | 22.9 | 7.5 | 1.5 | 1.9 |
| hotness | 132.2 | 23.2 | 7.8 | 2.4 | 2.4 |
| profile{dot} | 131.9 | 23.0 | 8.8 | 7.8 | 7.9 |

Remote recording time (median): milkv-duo ~132s, pi0 ~21s, nuc11 ~6.3s, aplos ~1.2s, mac-mini ~1.5s.

## Results: Cumulative Monitors (filtered)

Monitors added incrementally. Values are medians across 5 runs.

### CPU utilization (%)

| Monitors | milkv-duo | pi0 | nuc11 | aplos | mac-mini |
|----------|-----------|-----|-------|-------|----------|
| none | 1.9 | 6.5 | 18.6 | 74.0 | 59.5 |
| +icount | 3.0 | 13.5 | 38.1 | 99.0 | 99.4 |
| +loops | 3.7 | 16.6 | 46.2 | 99.5 | 99.6 |
| +hotness | 23.7 | 95.2 | 98.0 | 100.0 | 100.0 |
| +profile{dot} | 29.1 | 96.1 | 98.4 | 100.0 | 100.0 |

### Wall time (seconds)

| Monitors | milkv-duo | pi0 | nuc11 | aplos | mac-mini |
|----------|-----------|-----|-------|-------|----------|
| none | 133.4 | 22.9 | 7.4 | 1.6 | 1.9 |
| +icount | 133.5 | 22.9 | 7.7 | 2.7 | 2.7 |
| +loops | 133.5 | 22.9 | 7.8 | 3.4 | 3.4 |
| +hotness | 134.2 | 31.6 | 30.5 | 29.8 | 30.4 |
| +profile{dot} | 134.3 | 38.9 | 37.7 | 37.4 | 37.2 |

Remote recording time (median): milkv-duo ~133s, pi0 ~21s, nuc11 ~6.4s, aplos ~1.2s, mac-mini ~1.5s.

## Key Findings

### 1. speedtest1 has much lower baseline replay overhead than sort-10k

| Device | sort-10k no-monitors CPU% | speedtest1 no-monitors CPU% |
|--------|--------------------------|----------------------------|
| milkv-duo | 3.9% | 1.9% |
| pi0 | 15.5% | 7.0% |
| nuc11 | — | 18.5% |
| aplos | — | 75.9% |
| mac-mini | 98.7% | 59.0% |

speedtest1's trace is much smaller (10.6 MB vs 218 MB) and arrives slower, giving
monitors more headroom.

### 2. Lightweight monitors (icount, loops) are nearly free on slow devices

On milkv-duo, icount adds 1.1% CPU and loops adds 0%. Even on pi0, they add
<7% individually. Only on Gigabit devices do they become visible.

### 3. hotness is the tipping point for cumulative overhead

Adding hotness to the cumulative stack pushes pi0 from 16.6% → 95.2% and nuc11
from 46.2% → 98.0%. However, individually hotness only uses 2.9% on milkv-duo
and 12.9% on pi0 — the compounding effect is significant.

### 4. profile{dot} absolute cost is ~37s regardless of device

When CPU-bottlenecked, all cumulative +profile{dot} runs converge to ~37-38s wall
time. This represents the fixed computational cost of profiling the 10.6 MB trace
through all 4 monitors.

### 5. nuc11 has ~1.2s overhead from Windows named pipe pipeline

nuc11 wall times are ~1.2s longer than the remote recording time due to PowerShell
startup and `pipe_forward.ps1` initialization (500ms sleep + startup overhead).

## Methodology

Same fan-out architecture as previous experiments (shared 256 MB buffer, spin-wait
consumers, per-process `wait4()` measurement). See RESULTS_cumulative.md for full
details. 5 runs per configuration, medians reported.

Note: stale `test.db` files must be cleaned between runs on each device to prevent
SQLite "database is locked" errors.
