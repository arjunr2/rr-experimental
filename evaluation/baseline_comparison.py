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
LABELS  = ["CRIMP", "QEMU", "Wasm-R3", "rr", "PinPlay"]
COLORS  = ["#4C72B0", "#DD8452", "#55A868", "#C44E52", "#8172B2"]
MARKERS = ["o", "D", "p", "s", "v"]

YMAX     = 16.0
YMAX_PAD = 22.5


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

    fig, ax = plt.subplots(figsize=(6, 4.5))

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
        xs = xb + dodge[i]
        ys = [slowdowns[bench][tool] for bench in benchmarks]
        ys_clip = [min(y, YMAX) for y in ys]
        ax.bar(xs, ys_clip, width=bar_width * 1.0, color=color, alpha=0.85,
               zorder=3, label=label)
        if tool in ("wasmtime-rr", "r3", "qemu-rr"):
            emu_slow = qemu_emu_slowdowns if tool == "qemu-rr" else wt_slowdowns
            light = mcolors.to_rgba(color, alpha=0.85)
            light = (*[min(c + 0.35, 1.0) for c in light[:3]], 0.85)
            hw = bar_width * 0.5
            for x, bench, y_clip in zip(xs, benchmarks, ys_clip):
                emu = min(emu_slow[bench], y_clip)
                rec = y_clip - emu
                # redraw: emulation portion in light shade, recording on top
                ax.bar(x, emu, width=bar_width, color=light, alpha=1.0, zorder=3,
                       hatch="xx", edgecolor="white", linewidth=0.3)
                ax.bar(x, rec, width=bar_width, bottom=emu, color=color, alpha=0.85, zorder=3)
        for x, y_raw, y_clip in zip(xs, ys, ys_clip):
            if y_raw > YMAX:
                ax.annotate("", xy=(x, YMAX + 1.2), xytext=(x, YMAX),
                            arrowprops=dict(arrowstyle="-|>", color=color,
                                           lw=1.5, mutation_scale=8), zorder=4)
                ax.text(x, YMAX + 1.4, f"{y_raw:.0f}×",
                        ha="center", va="bottom", fontsize=11.5, color=color,
                        fontweight="bold", zorder=5, rotation=90,
                        bbox=dict(boxstyle="square,pad=0.05", fc="white", ec="none"))

    ax.set_xlim(xb[0] - SEP * 0.5, xb[-1] + SEP * 0.5)
    ax.axhline(1.0, color="black", linewidth=0.9, linestyle="--", zorder=1)
    ax.set_xticks(xb)
    ax.set_xticklabels(blabels, fontsize=12)
    ax.set_ylabel("Slowdown vs Native (×)", fontsize=12)
    ax.yaxis.set_major_locator(ticker.MultipleLocator(2))
    ax.yaxis.set_minor_locator(ticker.MultipleLocator(1))
    ax.yaxis.set_major_formatter(ticker.FuncFormatter(lambda y, _: f"{y:.0f}"))
    ax.yaxis.set_tick_params(labelsize=10, which="both")
    ax.set_ylim(bottom=0, top=YMAX_PAD)
    ax.grid(axis="y", which="major", linestyle="--", alpha=0.4, zorder=0)
    ax.grid(axis="y", which="minor", linestyle=":",  alpha=0.3,  zorder=0)
    ax.set_axisbelow(True)

    # vertical separators between benchmarks
    mids = (xb[:-1] + xb[1:]) / 2
    for xm in mids:
        ax.axvline(xm, color="#cccccc", linewidth=0.8, linestyle="-", zorder=0)

    ax.legend(fontsize=11, framealpha=0.9, loc="upper left",
              ncol=5, handletextpad=0.4, columnspacing=0.8,
              bbox_to_anchor=(0.0, 1.01))

    PLOTS_DIR.mkdir(parents=True, exist_ok=True)
    out = PLOTS_DIR / "baseline_comparison.pdf"
    fig.tight_layout()
    fig.savefig(out, bbox_inches="tight", dpi=150)
    print(f"Saved: {out}")


if __name__ == "__main__":
    main()
