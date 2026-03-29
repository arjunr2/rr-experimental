#!/usr/bin/env python3
"""Trace rate plot (avg MB/s with upward line to peak).
X = devices, dodge = benchmarks. Y axis log scale."""

import numpy as np
import matplotlib.pyplot as plt
import matplotlib.ticker as ticker
from matplotlib.patches import Patch

from data_utils import (
    load_data, get_devices_ordered, get_device_label,
    get_benchmarks, get_benchmark_label, PLOTS_DIR,
)

BENCH_COLORS  = ["#4C72B0", "#DD8452", "#55A868", "#C44E52", "#8172B2", "#937860"]
BENCH_MARKERS = ["o", "D", "p", "s", "v", "h"]


def get_trace_rates(data):
    """Returns avg[bench][device] and peak[bench][device] in MB/s."""
    nt = data["non_threaded"]
    avg, peak = {}, {}
    for bench in data["metadata"]["benchmarks"]:
        avg[bench]  = {d: nt[bench]["trace_avg_mb_s"][d]  for d in data["metadata"]["devices"]}
        peak[bench] = {d: nt[bench]["trace_peak_mb_s"][d] for d in data["metadata"]["devices"]}
    return avg, peak


CAP_WIDTH = 0.06  # half-width of horizontal cap at peak

def draw_strip(ax, xpos, bar_width, bench, device, avg, peak, color):
    """Bar from 0 to avg, vertical line from avg up to peak, with a horizontal cap."""
    a = avg[bench][device]
    p = peak[bench][device]
    ax.bar(xpos, a, width=bar_width, color=color, alpha=0.85, zorder=3)
    ax.plot([xpos, xpos], [a, p], color=color, linewidth=1.5, alpha=0.7, zorder=4)
    ax.plot([xpos - CAP_WIDTH, xpos + CAP_WIDTH], [p, p], color=color,
            linewidth=1.5, alpha=0.9, zorder=4)


def main():
    data       = load_data()
    devices    = get_devices_ordered(data)
    dlabels    = [get_device_label(data, d) for d in devices]
    benchmarks = get_benchmarks(data)

    avg, peak = get_trace_rates(data)

    n_dev   = len(devices)
    n_bench = len(benchmarks)
    SEP     = 1.1

    fig, ax = plt.subplots(figsize=(7, 5.2))

    xl      = np.arange(n_dev) * SEP
    dodge_b = np.linspace(-0.38, 0.38, n_bench)
    bar_width = dodge_b[1] - dodge_b[0]
    legend_handles = []
    for i, (bench, color, marker) in enumerate(zip(benchmarks, BENCH_COLORS, BENCH_MARKERS)):
        for j, device in enumerate(devices):
            draw_strip(ax, xl[j] + dodge_b[i], bar_width * 1.0, bench, device, avg, peak, color)
        legend_handles.append(Patch(facecolor=color, alpha=0.85,
                                    label=get_benchmark_label(data, bench)))

    ax.set_xlim(xl[0] - SEP * 0.5, xl[-1] + SEP * 0.5)
    ax.set_xticks(xl)
    ax.set_xticklabels(dlabels, fontsize=14)
    ax.set_ylabel("Streaming Trace Rate (MB/s)", fontsize=14, labelpad=1)
    ax.set_yscale("log")
    ax.yaxis.set_major_formatter(ticker.FuncFormatter(lambda y, _: f"{y:g}"))
    ax.yaxis.set_tick_params(labelsize=13, which="both")
    ax.grid(axis="y", which="major", linestyle="--", alpha=0.4, zorder=0)
    ax.grid(axis="y", which="minor", linestyle=":",  alpha=0.3,  zorder=0)
    ax.set_axisbelow(True)
    mids = (xl[:-1] + xl[1:]) / 2
    for xm in mids:
        ax.axvline(xm, color="#cccccc", linewidth=0.8, linestyle="-", zorder=0)
    ax.legend(handles=legend_handles, fontsize=14, framealpha=0.9, loc="upper right",
              ncol=1, handletextpad=0.4)

    PLOTS_DIR.mkdir(parents=True, exist_ok=True)
    out = PLOTS_DIR / "trace_rate.pdf"
    fig.tight_layout()
    fig.savefig(out, dpi=150)
    print(f"Saved: {out}")


if __name__ == "__main__":
    main()
