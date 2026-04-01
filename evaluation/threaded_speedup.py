#!/usr/bin/env python3
"""Two-panel threaded vs non-threaded speedup bar plot (wasmtime-native replayer only).
Left panel:  X = devices,    dodge = benchmarks.
Right panel: X = benchmarks, dodge = devices.
Y axis shared; right panel suppresses Y tick labels."""

import numpy as np
import matplotlib.pyplot as plt
import matplotlib.ticker as ticker
from matplotlib.ticker import FuncFormatter
from matplotlib.patches import Patch

from data_utils import (
    load_data, get_devices_ordered, get_device_label,
    get_benchmarks, get_benchmark_label, PLOTS_DIR,
)

BENCH_COLORS  = ["#4C72B0", "#DD8452", "#55A868", "#C44E52", "#8172B2", "#937860"]
DEVICE_COLORS = ["#4C72B0", "#DD8452", "#55A868", "#C44E52", "#8172B2"]

VARIANT      = "socat+replay"   # wasmtime-native only
YMIN_DISPLAY = 0.0
YMAX_PAD     = 1.85


def compute_ratios(data):
    """ratios[bench][device] = non-threaded / threaded (speedup) for wasmtime-native."""
    raw = data["derived"]["threaded_vs_non_threaded_ratio"]
    out = {}
    for bench in data["metadata"]["benchmarks"]:
        out[bench] = {d: 1.0 / raw[bench][VARIANT][d]
                      for d in data["metadata"]["devices"]}
    return out


def draw_bar(ax, xpos, bar_width, y_raw, color):
    ax.bar(xpos, y_raw - YMIN_DISPLAY, bottom=YMIN_DISPLAY,
           width=bar_width, color=color, alpha=0.85, zorder=3)


def style_ax(ax, xtick_labels, ylabel=None, tick_fontsize=10, x_positions=None):
    if x_positions is None:
        x_positions = np.arange(len(xtick_labels))
    ax.axhline(1.0, color="black", linewidth=0.9, linestyle="--", zorder=1)
    ax.set_xticks(x_positions)
    ax.set_xticklabels(xtick_labels, fontsize=tick_fontsize)
    if ylabel:
        ax.set_ylabel(ylabel, fontsize=10)
    ax.yaxis.set_major_locator(ticker.MultipleLocator(0.2))
    ax.yaxis.set_minor_locator(ticker.MultipleLocator(0.1))
    ax.yaxis.set_major_formatter(FuncFormatter(lambda y, _: f"{y:.1f}"))
    ax.yaxis.set_tick_params(labelsize=10)
    ax.grid(axis="y", which="major", linestyle="--", alpha=0.4, zorder=0)
    ax.grid(axis="y", which="minor", linestyle=":",  alpha=0.45, zorder=0)
    ax.set_axisbelow(True)
    ax.set_ylim(bottom=YMIN_DISPLAY, top=YMAX_PAD)
    ax.set_xlim(x_positions[0] - 0.7, x_positions[-1] + 0.7)
    mids = (x_positions[:-1] + x_positions[1:]) / 2
    for xm in mids:
        ax.axvline(xm, color="#cccccc", linewidth=0.8, linestyle="-", zorder=0)


def main():
    data       = load_data()
    devices    = get_devices_ordered(data)
    dlabels    = [get_device_label(data, d) for d in devices]
    benchmarks = get_benchmarks(data)
    ratios     = compute_ratios(data)

    n_dev   = len(devices)
    n_bench = len(benchmarks)
    SEP     = 1.1

    fig, (ax_l, ax_r) = plt.subplots(
        1, 2, sharey=True, figsize=(12, 3.8),
        gridspec_kw={"width_ratios": [1, 1], "wspace": 0.04},
    )

    # ── Left panel: X = devices, dodge = benchmarks ───────────────────────
    xl        = np.arange(n_dev) * SEP
    dodge_b   = np.linspace(-0.38, 0.38, n_bench)
    bar_width = dodge_b[1] - dodge_b[0]
    for i, (bench, color) in enumerate(zip(benchmarks, BENCH_COLORS)):
        for j, device in enumerate(devices):
            draw_bar(ax_l, xl[j] + dodge_b[i], bar_width * 0.9,
                     ratios[bench][device], color)

    legend_handles = [Patch(facecolor=c, alpha=0.85, label=get_benchmark_label(data, b))
                      for b, c in zip(benchmarks, BENCH_COLORS)]
    style_ax(ax_l, dlabels, ylabel="Threaded Speedup (×)", x_positions=xl, tick_fontsize=11)
    ax_l.legend(handles=legend_handles, fontsize=10, framealpha=0.9,
                loc="upper left", ncol=1, handletextpad=0.4, labelspacing=0.2)

    # ── Right panel: X = benchmarks, dodge = devices ──────────────────────
    benchmarks_r = list(benchmarks)
    ri, bi = benchmarks_r.index("compress-l3"), benchmarks_r.index("blake3")
    benchmarks_r[ri], benchmarks_r[bi] = benchmarks_r[bi], benchmarks_r[ri]
    ci, regi = benchmarks_r.index("compress-l3"), benchmarks_r.index("regex-redux")
    benchmarks_r[ci], benchmarks_r[regi] = benchmarks_r[regi], benchmarks_r[ci]
    blabels_r = [get_benchmark_label(data, b).replace("-", "-\n", 1) for b in benchmarks_r]

    xr        = np.arange(n_bench) * SEP
    dodge_d   = np.linspace(-0.32, 0.32, n_dev)
    bar_width_r = dodge_d[1] - dodge_d[0]
    for i, (device, color) in enumerate(zip(devices, DEVICE_COLORS)):
        for j, bench in enumerate(benchmarks_r):
            draw_bar(ax_r, xr[j] + dodge_d[i], bar_width_r * 0.9,
                     ratios[bench][device], color)

    legend_handles_r = [Patch(facecolor=c, alpha=0.85, label=get_device_label(data, d))
                        for d, c in zip(devices, DEVICE_COLORS)]
    style_ax(ax_r, blabels_r, tick_fontsize=10, x_positions=xr)
    ax_r.tick_params(axis="y", which="both", left=False)
    ax_r.legend(handles=legend_handles_r, fontsize=10, framealpha=0.9,
                loc="upper left", handletextpad=0.4)

    PLOTS_DIR.mkdir(parents=True, exist_ok=True)
    out = PLOTS_DIR / "threaded_speedup.pdf"
    fig.tight_layout()
    fig.savefig(out, bbox_inches="tight", dpi=150)
    print(f"Saved: {out}")


if __name__ == "__main__":
    main()
