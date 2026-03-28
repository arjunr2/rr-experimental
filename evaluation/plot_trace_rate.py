#!/usr/bin/env python3
"""Two-panel trace rate plot (avg MB/s with upward line to peak).
Left panel:  X = devices,    dodge = benchmarks.
Right panel: X = benchmarks, dodge = devices.
Y axis log scale; right panel suppresses Y tick labels."""

import numpy as np
import matplotlib.pyplot as plt
import matplotlib.ticker as ticker

from data_utils import (
    load_data, get_devices_ordered, get_device_label,
    get_benchmarks, get_benchmark_label, PLOTS_DIR,
)

BENCH_COLORS   = ["#4C72B0", "#DD8452", "#55A868", "#C44E52", "#8172B2", "#937860"]
BENCH_MARKERS  = ["o", "D", "p", "s", "v", "h"]
DEVICE_COLORS  = ["#4C72B0", "#DD8452", "#55A868", "#C44E52", "#8172B2"]
DEVICE_MARKERS = ["o", "D", "p", "s", "v"]


def get_trace_rates(data):
    """Returns avg[bench][device] and peak[bench][device] in MB/s."""
    nt = data["non_threaded"]
    avg, peak = {}, {}
    for bench in data["metadata"]["benchmarks"]:
        avg[bench]  = {d: nt[bench]["trace_avg_mb_s"][d]  for d in data["metadata"]["devices"]}
        peak[bench] = {d: nt[bench]["trace_peak_mb_s"][d] for d in data["metadata"]["devices"]}
    return avg, peak


CAP_WIDTH = 0.06  # half-width of horizontal cap at peak

def draw_strip(ax, xpos, bench, device, avg, peak, color, marker):
    """Dot at avg, vertical line from avg up to peak, with a horizontal cap at peak."""
    a = avg[bench][device]
    p = peak[bench][device]
    ax.plot([xpos, xpos], [a, p], color=color, linewidth=1.5, alpha=0.5, zorder=2)
    ax.plot([xpos - CAP_WIDTH, xpos + CAP_WIDTH], [p, p], color=color,
            linewidth=1.5, alpha=0.7, zorder=2)
    ax.scatter(xpos, a, color=color, s=70, zorder=3, alpha=0.88,
               marker=marker, edgecolors="none")


def style_ax(ax, xtick_labels, ylabel=None, tick_fontsize=10, x_positions=None):
    if x_positions is None:
        x_positions = np.arange(len(xtick_labels))
    ax.set_xticks(x_positions)
    ax.set_xticklabels(xtick_labels, fontsize=tick_fontsize)
    if ylabel:
        ax.set_ylabel(ylabel, fontsize=10)
    ax.set_yscale("log")
    ax.yaxis.set_major_formatter(ticker.FuncFormatter(lambda y, _: f"{y:g}"))
    ax.yaxis.set_tick_params(labelsize=10, which="both")
    ax.grid(axis="y", which="major", linestyle="--", alpha=0.4, zorder=0)
    ax.grid(axis="y", which="minor", linestyle=":",  alpha=0.3,  zorder=0)
    ax.set_axisbelow(True)
    mids = (x_positions[:-1] + x_positions[1:]) / 2
    for xm in mids:
        ax.axvline(xm, color="#cccccc", linewidth=0.8, linestyle="-", zorder=0)


def main():
    data       = load_data()
    devices    = get_devices_ordered(data)
    dlabels    = [get_device_label(data, d) for d in devices]
    benchmarks = get_benchmarks(data)

    avg, peak = get_trace_rates(data)

    n_dev   = len(devices)
    n_bench = len(benchmarks)
    SEP     = 1.4

    fig, (ax_l, ax_r) = plt.subplots(
        1, 2, sharey=True, figsize=(12, 3.8),
        gridspec_kw={"width_ratios": [1, 1], "wspace": 0.04},
    )

    # ── Left panel: X = devices, dodge = benchmarks ───────────────────────
    xl      = np.arange(n_dev) * SEP
    dodge_b = np.linspace(-0.38, 0.38, n_bench)
    for i, (bench, color, marker) in enumerate(zip(benchmarks, BENCH_COLORS, BENCH_MARKERS)):
        for j, device in enumerate(devices):
            draw_strip(ax_l, xl[j] + dodge_b[i], bench, device, avg, peak, color, marker)
        ax_l.scatter([], [], color=color, s=55, marker=marker, label=get_benchmark_label(data, bench))

    style_ax(ax_l, dlabels, ylabel="Trace Rate (MB/s)", x_positions=xl, tick_fontsize=11)
    ax_l.legend(fontsize=10, framealpha=0.9, loc="lower left", ncol=2,
                handletextpad=0.4, columnspacing=0.8)

    # ── Right panel: X = benchmarks, dodge = devices ──────────────────────
    benchmarks_r = list(benchmarks)
    ri, bi = benchmarks_r.index("compress-l3"), benchmarks_r.index("blake3")
    benchmarks_r[ri], benchmarks_r[bi] = benchmarks_r[bi], benchmarks_r[ri]
    ci, regi = benchmarks_r.index("compress-l3"), benchmarks_r.index("regex-redux")
    benchmarks_r[ci], benchmarks_r[regi] = benchmarks_r[regi], benchmarks_r[ci]
    blabels_r = [get_benchmark_label(data, b).replace("-", "-\n", 1) for b in benchmarks_r]

    xr      = np.arange(n_bench) * SEP
    dodge_d = np.linspace(-0.32, 0.32, n_dev)
    for i, (device, color, marker) in enumerate(zip(devices, DEVICE_COLORS, DEVICE_MARKERS)):
        for j, bench in enumerate(benchmarks_r):
            draw_strip(ax_r, xr[j] + dodge_d[i], bench, device, avg, peak, color, marker)
        ax_r.scatter([], [], color=color, s=55, marker=marker, label=get_device_label(data, device))

    style_ax(ax_r, blabels_r, tick_fontsize=10, x_positions=xr)
    ax_r.tick_params(axis="y", which="both", left=False)
    ax_r.legend(fontsize=10, framealpha=0.9, loc="lower left", ncol=1,
                handletextpad=0.4, labelspacing=0.2)

    PLOTS_DIR.mkdir(parents=True, exist_ok=True)
    out = PLOTS_DIR / "trace_rate.pdf"
    fig.tight_layout()
    fig.savefig(out, bbox_inches="tight", dpi=150)
    print(f"Saved: {out}")


if __name__ == "__main__":
    main()
