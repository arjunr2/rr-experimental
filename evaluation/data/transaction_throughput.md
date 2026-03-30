# Sysbench OLTP Rate-Limited Benchmark Results

## Overview

We measure the maximum sustained transaction rate for a sysbench `oltp_read_write`
workload running against SQLite, comparing native execution, wasmtime (WASM), and
wasmtime-rr (WASM with deterministic recording). At rates below the saturation
point, recording overhead is **zero** — all three variants achieve identical throughput.

## Benchmark

**Workload**: sysbench `oltp_read_write` — 10,000-row table, 20 queries per
transaction (10 point SELECTs, range SELECT, SUM, ORDER BY, DISTINCT, UPDATE index,
UPDATE non-index, DELETE + INSERT). Queries captured from actual sysbench 1.0.20
run against PostgreSQL 16.13.

**Rate limiting**: `clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME)` with absolute
time targets. Transactions executed in batches of 100, one sleep per batch. Each
run is 10 seconds.

**Recording**: wasmtime-rr writes trace to `/dev/null` (measures pure recording
overhead without I/O).

## Maximum Sustained Throughput

"Sustained" = achieves 100% of target rate over 10 seconds with no degradation.

| Device | CPU | Native (txn/s) | Wasmtime (txn/s) | Wasmtime-rr (txn/s) |
|--------|-----|---------------|-----------------|---------------------|
| local | AMD Ryzen 9 7950X (Zen 4) | 8400 | 3600 | 3000 |
| milkv-duo | T-Head C906 (RISC-V, 1 GHz) | 130 | 50 | 45 |
| pi0 | ARM Cortex-A53 (4 cores, 1 GHz) | 290 | 140 | 120 |
| aplos | AMD Ryzen 5 4500U (6 cores, Zen 2) | 5100 | 2400 | 1950 |
| mac-mini | Apple M2 Pro (10 cores) | 5100 | 2300 | 1900 |
| nuc11 | Intel i7-1165G7 (4C/8T, Windows) | 1100 | 310 | 280 |

## Detailed Saturation Curves

### local (AMD Ryzen 9 7950X)

**Native**:
| Target | Actual | % |
|--------|--------|---|
| 7000 | 7000 | 100% |
| 7500 | 7500 | 100% |
| 8000 | 8000 | 100% |
| 8200 | 8200 | 100% |
| 8400 | 8400 | 100% |
| 8500 | 8453 | 99% |
| 8600 | 8446 | 98% |
| 9000 | 8443 | 93% |

**Wasmtime**:
| Target | Actual | % |
|--------|--------|---|
| 3000 | 3000 | 100% |
| 3200 | 3200 | 100% |
| 3400 | 3400 | 100% |
| 3600 | 3600 | 100% |
| 3700 | 3698 | 99% |
| 3800 | 3788 | 99% |
| 4000 | 3811 | 95% |

**Wasmtime-rr**:
| Target | Actual | % |
|--------|--------|---|
| 2800 | 2800 | 100% |
| 2900 | 2900 | 100% |
| 3000 | 3000 | 100% |
| 3050 | 3042 | 99% |
| 3100 | 3023 | 97% |
| 3200 | 3020 | 94% |

## Key Finding

At any transaction rate ≤3000 txn/s on this machine, native, wasmtime, and
wasmtime-rr all achieve **identical throughput**. Recording overhead is absorbed
by the idle time between transaction batches via absolute-time sleep targets.

The recording overhead (17% reduction in max throughput: 3600 → 3000) only
manifests when the system is fully CPU-saturated.

## Reproducing

### 1. Generate queries with sysbench

The query file was captured from an actual sysbench run against PostgreSQL.

```bash
# Install sysbench and PostgreSQL
sudo apt install sysbench postgresql

# Create database
sudo -u postgres createuser --superuser $(whoami)
sudo -u postgres createdb sbtest

# Enable query logging
psql sbtest -c "ALTER SYSTEM SET log_statement = 'all';"
psql sbtest -c "ALTER SYSTEM SET log_destination = 'csvlog';"
psql sbtest -c "ALTER SYSTEM SET logging_collector = 'on';"
psql sbtest -c "ALTER SYSTEM SET log_directory = '/tmp/pglog';"
psql sbtest -c "ALTER SYSTEM SET log_filename = 'sysbench.csv';"
sudo mkdir -p /tmp/pglog && sudo chown postgres:postgres /tmp/pglog
sudo systemctl restart postgresql

# Prepare table (10,000 rows)
sysbench /usr/share/sysbench/oltp_read_write.lua \
  --db-driver=pgsql --pgsql-db=sbtest --pgsql-user=$(whoami) \
  --pgsql-host=/var/run/postgresql \
  --tables=1 --table_size=10000 prepare

# Run 10,000 transactions (200K queries)
sudo truncate -s 0 /tmp/pglog/sysbench.csv.csv
sysbench /usr/share/sysbench/oltp_read_write.lua \
  --db-driver=pgsql --pgsql-db=sbtest --pgsql-user=$(whoami) \
  --pgsql-host=/var/run/postgresql \
  --tables=1 --table_size=10000 \
  --threads=1 --events=10000 --time=0 run

# Copy log (owned by postgres)
sudo cp /tmp/pglog/sysbench.csv.csv /tmp/sysbench_log.csv
sudo chmod 644 /tmp/sysbench_log.csv
```

### 2. Extract SQL from PostgreSQL log

```python
import csv, re

out = open('sysbench_queries.sql', 'w')
with open('/tmp/sysbench_log.csv') as f:
    reader = csv.reader(f)
    for row in reader:
        if len(row) < 14: continue
        msg = row[13]
        params = row[14] if len(row) > 14 else ''
        if not msg.startswith('execute '): continue
        m = re.match(r'execute \S+: (.+)', msg)
        if not m: continue
        sql = m.group(1).strip()
        if params.startswith('parameters: '):
            param_str = params[len('parameters: '):]
            for pm in re.finditer(r"\$(\d+) = '([^']*)'", param_str):
                sql = sql.replace(f'${pm.group(1)}', f"'{pm.group(2)}'", 1)
        out.write(sql + ';\n')
out.close()
```

### 3. Dump table as SQLite-compatible SQL

```bash
psql sbtest -t -A -c "SELECT 'INSERT INTO sbtest1 VALUES (' || id || ',' || k || ',''' \
  || replace(c, '''', '''''') || ''',''' || replace(pad, '''', '''''') || ''');' \
  FROM sbtest1 ORDER BY id" > sysbench_data.sql

cat > sysbench_prepare.sql << 'EOF'
CREATE TABLE sbtest1(
  id INTEGER NOT NULL PRIMARY KEY,
  k INTEGER DEFAULT 0 NOT NULL,
  c CHAR(120) DEFAULT '' NOT NULL,
  pad CHAR(60) DEFAULT '' NOT NULL
);
CREATE INDEX k_1 ON sbtest1(k);
EOF

cat sysbench_data.sql >> sysbench_prepare.sql
rm sysbench_data.sql
```

### 4. Build sysbench_runner

The runner uses SQLite 3.47.2 amalgamation (`sqlite3.c`, `sqlite3.h`).

```bash
# Common build flags
CFLAGS="-O2 -DSQLITE_THREADSAFE=0 -DSQLITE_OMIT_WAL=1 \
  -DSQLITE_OMIT_LOAD_EXTENSION=1 -DSQLITE_NO_SYNC=1 -DSQLITE_TEMP_STORE=3"

# Native Linux
gcc $CFLAGS -o sysbench_runner sysbench_runner.c sqlite3.c -lpthread -ldl -lm

# Native macOS (on device)
clang $CFLAGS -o sysbench_runner sysbench_runner.c sqlite3.c -lm

# Native Windows (MinGW, on device)
gcc $CFLAGS -o sysbench_runner.exe sysbench_runner.c sqlite3.c -lm

# Native aarch64 (cross-compile for pi0)
aarch64-linux-gnu-gcc $CFLAGS -static -o sysbench_runner-aarch64 \
  sysbench_runner.c sqlite3.c -lpthread -lm

# Native riscv64 (cross-compile for milkv-duo)
riscv64-linux-gnu-gcc $CFLAGS -static -o sysbench_runner-riscv64 \
  sysbench_runner.c sqlite3.c -lpthread -lm

# WASM (wasi-sdk, same binary for all devices)
/opt/wasi-sdk/bin/clang $CFLAGS -o sysbench_runner.wasm \
  sysbench_runner.c sqlite3.c -lm
```

### 5. Run the benchmark

```bash
# Phase 1: Prepare (loads 10K rows into SQLite)
# Phase 2: Run transactions at target rate for 10 seconds

# Native
./sysbench_runner test.db sysbench_prepare.sql sysbench_queries.sql --rate 500 --time 10

# Wasmtime (no recording)
wasmtime --dir=.::/work sysbench_runner.wasm \
  /work/test.db /work/sysbench_prepare.sql /work/sysbench_queries.sql --rate 500 --time 10

# Wasmtime-rr (recording to /dev/null)
wasmtime -R path=/dev/null,buffer-size=8192 --dir=.::/work sysbench_runner.wasm \
  /work/test.db /work/sysbench_prepare.sql /work/sysbench_queries.sql --rate 500 --time 10

# Clean database between runs
rm -rf test*.db*
```

Output: `METRICS <actual_rate> <actual_rate> <elapsed_seconds>`

### 6. Find the saturation point

Sweep target rates upward until `actual_rate < target_rate`. The highest rate
where `actual == target` (100%) is the maximum sustained throughput.

### Rate limiting mechanism

The runner executes transactions in **batches of 100**. Between batches, it calls
`clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, ...)` (Linux/WASI),
`mach_wait_until()` (macOS), or `SetWaitableTimer` + `WaitForSingleObject`
(Windows) to sleep until the next absolute time target.

Absolute time targets are computed as `start + batch_num * (100 / rate)` seconds.
This self-corrects for per-call overhead: if a batch runs late, the next sleep
is shorter. The total number of transactions over the 10-second window reflects
whether the system kept up with the target rate.

For WASM, wasi-sdk's libc translates `clock_nanosleep` to WASI `poll_oneoff`
with a clock subscription. This works correctly with absolute time targets in
wasmtime (verified: 100 ticks at 100/s = 1.001s in WASM, identical with and
without recording).

### Notes

- **Clean database between runs**: SQLite creates `.db-wal`, `.db-shm`, and
  `.db.lock` files. Use `rm -rf test*.db*` between runs.
- **milkv-duo**: requires `-C cranelift-has_v=false` for wasmtime. Non-threaded
  recording (`buffer-size=8192` only).
- **nuc11 (Windows)**: uses `nul` instead of `/dev/null` for trace discard.
  Threaded recording. WASI path mapping uses `--dir=.` with relative paths.
- **mac-mini**: occasional outliers from macOS background tasks (Spotlight,
  Gatekeeper). Rerun affected data points.
- **Compile natively on-device** for best results (aplos, nuc11, mac-mini).
  Cross-compile only for pi0 and milkv-duo.
