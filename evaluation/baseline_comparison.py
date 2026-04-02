#!/usr/bin/env python3
"""Baseline comparison: slowdown vs native for record/replay tools on x86.
X = benchmarks, dodge = tools. Y = slowdown (log scale)."""

import json
import numpy as np
import matplotlib.pyplot as plt
import matplotlib.ticker as ticker
import matplotlib.colors as mcolors
from pathlib import Path

DATA_FILE = Path(__file__).parent / "data" / "baseline_comparison.json"
PLOTS_DIR = Path(__file__).parent / "plots"

TOOLS   = ["wasmtime-rr", "qemu-rr", "r3", "rr", "pinplay"]
LABELS  = ["RDT", "QEMU", "Wasm-R3", "rr", "PinPlay"]
COLORS  = ["#4C72B0", "#DD8452", "#55A868", "#C44E52", "#8172B2"]
MARKERS = ["o", "D", "p", "s", "v"]

YMIN = 0.9
YMAX = 1000.0


def load_data():
    with open(DATA_FILE) as f:
        return json.load(f)


def compute_slowdowns(data):
    """slowdown[bench][tool] = best_time / native_time.
    wasmtime-rr uses min(wasmtime-rr, wtrr-threaded)."""
    out = {}
    for bench, res in data["results"].items():
        nat = res["time_s"]["native"]
        out[bench] = {}
        for tool in TOOLS:
            if tool == "wasmtime-rr":
                t = min(res["time_s"]["wasmtime-rr"], res["time_s"]["wtrr-threaded"])
            else:
                t = res["time_s"][tool]
            out[bench][tool] = t / nat
    return out


def get_bench_label(data, bench):
    return data["metadata"]["benchmarks"][bench]


def main():
    data = load_data()
    benchmarks = [b for b in data["results"].keys()
                  if b not in ("compress-l9", "compress-l19")]
    # same order as threaded_speedup / record_streaming_overhead right panel:
    # swap compress-l3 ↔ blake3, then compress-l3 ↔ regex-redux
    ri, bi = benchmarks.index("compress-l3"), benchmarks.index("blake3")
    benchmarks[ri], benchmarks[bi] = benchmarks[bi], benchmarks[ri]
    ci, regi = benchmarks.index("compress-l3"), benchmarks.index("regex-redux")
    benchmarks[ci], benchmarks[regi] = benchmarks[regi], benchmarks[ci]
    blabels = [get_bench_label(data, b).replace("-", "-\n", 1) for b in benchmarks]

    slowdowns = compute_slowdowns(data)

    n_bench = len(benchmarks)
    n_tools = len(TOOLS)
    SEP     = 1.1

    fig, ax = plt.subplots(figsize=(6.5, 4))

    xb      = np.arange(n_bench) * SEP
    dodge   = np.linspace(-0.36, 0.36, n_tools)

    # emulation overhead = wasmtime / native (for CRIMP bars)
    wt_slowdowns = {bench: data["results"][bench]["time_s"]["wasmtime"]
                    / data["results"][bench]["time_s"]["native"]
                    for bench in benchmarks}
    # qemu emulation overhead = qemu-norr-internal / native (for QEMU-rr bars)
    qemu_emu_slowdowns = {bench: data["results"][bench]["time_s"]["qemu-norr-internal"]
                          / data["results"][bench]["time_s"]["native"]
                          for bench in benchmarks}

    bar_width = dodge[1] - dodge[0]
    for i, (tool, label, color) in enumerate(zip(TOOLS, LABELS, COLORS)):
        xs  = xb + dodge[i]
        ys  = [slowdowns[bench][tool] for bench in benchmarks]
        for j, (x, bench, total) in enumerate(zip(xs, benchmarks, ys)):
            ax.bar(x, total - 1.0, bottom=1.0, width=bar_width,
                   color=color, alpha=0.85, zorder=3,
                   label=label if j == 0 else None)
            # mark emulation/recording split with an X at the emulation height
            if tool in ("wasmtime-rr", "r3", "qemu-rr"):
                emu_slow = qemu_emu_slowdowns if tool == "qemu-rr" else wt_slowdowns
                emu = emu_slow[bench]
                ax.scatter(x, emu, marker="x", color="white", s=60,
                           linewidths=2.0, zorder=5)

    ax.set_yscale("log")
    ax.set_xlim(xb[0] - SEP * 0.5, xb[-1] + SEP * 0.5)
    ax.set_ylim(bottom=YMIN, top=YMAX)
    ax.axhline(1.0, color="black", linewidth=0.9, linestyle="--", zorder=1)
    ax.set_xticks(xb)
    ax.set_xticklabels(blabels, fontsize=12)
    ax.set_ylabel("Slowdown vs Native (×)", fontsize=14, labelpad=-3)
    ax.yaxis.set_major_locator(ticker.LogLocator(base=10, subs=[1.0]))
    ax.yaxis.set_minor_locator(ticker.LogLocator(base=10, subs=[2, 3, 5]))
    ax.yaxis.set_major_formatter(ticker.FuncFormatter(lambda y, _: f"{y:.0f}×"))
    ax.yaxis.set_minor_formatter(ticker.FuncFormatter(lambda y, _: f"{y:.0f}"))
    ax.yaxis.set_tick_params(labelsize=11, which="both")
    ax.grid(axis="y", which="major", linestyle="--", alpha=0.4, zorder=0)
    ax.grid(axis="y", which="minor", linestyle=":",  alpha=0.2,  zorder=0)
    ax.set_axisbelow(True)

    # vertical separators between benchmarks
    mids = (xb[:-1] + xb[1:]) / 2
    for xm in mids:
        ax.axvline(xm, color="#cccccc", linewidth=0.8, linestyle="-", zorder=0)

    ax.legend(fontsize=12, framealpha=0.9, loc="upper right",
              ncol=2, handletextpad=0.4, columnspacing=0.8,
              bbox_to_anchor=(1.0, 1.01))

    PLOTS_DIR.mkdir(parents=True, exist_ok=True)
    out = PLOTS_DIR / "baseline_comparison.pdf"
    fig.tight_layout()
    fig.savefig(out, bbox_inches="tight", dpi=150)
    print(f"Saved: {out}")


if __name__ == "__main__":
    main()
