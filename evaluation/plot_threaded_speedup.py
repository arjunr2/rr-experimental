#!/usr/bin/env python3
"""Two-panel threaded vs non-threaded time ratio plot.
Left panel:  X = devices,    dodge = benchmarks, 3 numbered dots per strip.
Right panel: X = benchmarks, dodge = devices,    3 numbered dots per strip.
Numbers 1/2/3 = wasmtime-native / wasmtime-wasm / wizeng.
Y axis shared; right panel suppresses Y tick labels."""

import numpy as np
import matplotlib.pyplot as plt
import matplotlib.ticker as ticker
from matplotlib.ticker import FixedLocator, FuncFormatter

from data_utils import (
    load_data, get_devices_ordered, get_device_label,
    get_benchmarks, get_benchmark_label, PLOTS_DIR,
)

BENCH_COLORS   = ["#4C72B0", "#DD8452", "#55A868", "#C44E52", "#8172B2", "#937860"]
BENCH_MARKERS  = ["o", "D", "p", "s", "v", "h"]
DEVICE_COLORS  = ["#4C72B0", "#DD8452", "#55A868", "#C44E52", "#8172B2"]
DEVICE_MARKERS = ["o", "D", "p", "s", "v"]

SOCAT_VARIANTS = ["socat+replay", "socat+decomposed-wt", "socat+wizeng"]
YMIN_CLIP    = 0.5    # clip threshold at bottom
YMIN_DISPLAY = 0.28   # actual ylim bottom (space for "0" label + cut lines)
YMAX_PAD     = 1.85   # ylim top


def compute_ratios(data):
    """Returns ratios[bench][variant][device] = non-threaded / threaded time (speedup)."""
    raw = data["derived"]["threaded_vs_non_threaded_ratio"]
    out = {}
    for bench in data["metadata"]["benchmarks"]:
        out[bench] = {}
        for var in SOCAT_VARIANTS:
            out[bench][var] = {d: 1.0 / raw[bench][var][d] for d in data["metadata"]["devices"]}
    return out


def draw_strip(ax, xpos, bench, device, ratios, color, marker):
    """Plot 3 dots connected by a vertical line; number only the top-most (highest) one.
    Values below YMIN_CLIP are clipped: downward arrow + annotated actual value."""
    ys      = [ratios[bench][var][device] for var in SOCAT_VARIANTS]
    top     = int(np.argmax(ys))
    raw_min = min(ys)

    ys_clip  = [max(y, YMIN_CLIP) for y in ys]
    disp_min = max(raw_min, YMIN_CLIP)

    ax.plot([xpos, xpos], [disp_min, max(ys_clip)], color=color,
            linewidth=1.5, alpha=0.5, zorder=2)

    if raw_min < YMIN_CLIP:
        ax.annotate("", xy=(xpos, YMIN_CLIP - 0.14), xytext=(xpos, YMIN_CLIP),
                    arrowprops=dict(arrowstyle="-|>", color=color,
                                   lw=1.5, mutation_scale=10), zorder=4)
        ax.text(xpos, YMIN_CLIP - 0.15, f"{raw_min:.1f}×",
                ha="center", va="top", fontsize=9, color=color,
                rotation=0, fontweight="bold", zorder=5,
                bbox=dict(boxstyle="square,pad=0.05", fc="white", ec="none"))

    for k, (y, num) in enumerate(zip(ys_clip, ["1", "2", "3"])):
        ax.scatter(xpos, y, color=color, s=110, zorder=3, alpha=0.88,
                   marker=marker, edgecolors="none")
        if k == top:
            ax.text(xpos, y, num, ha="center", va="center",
                    fontsize=8, color="white", fontweight="bold", zorder=4)


def style_ax(ax, xtick_labels, ylabel=None, tick_fontsize=10, x_positions=None):
    if x_positions is None:
        x_positions = np.arange(len(xtick_labels))
    ax.axhline(1.0, color="black", linewidth=0.9, linestyle="--", zorder=1)
    ax.set_xticks(x_positions)
    ax.set_xticklabels(xtick_labels, fontsize=tick_fontsize)
    if ylabel:
        ax.set_ylabel(ylabel, fontsize=10)
    major_ticks = [YMIN_DISPLAY] + list(np.arange(YMIN_CLIP, YMAX_PAD + 0.01, 0.2))
    ax.yaxis.set_major_locator(FixedLocator(major_ticks))
    ax.yaxis.set_major_formatter(FuncFormatter(
        lambda y, _: "0" if abs(y - YMIN_DISPLAY) < 0.001 else f"{y:.1f}"
    ))
    ax.yaxis.set_minor_locator(ticker.MultipleLocator(0.1))
    ax.yaxis.set_tick_params(labelsize=10)
    ax.grid(axis="y", which="major", linestyle="--", alpha=0.4, zorder=0)
    ax.grid(axis="y", which="minor", linestyle=":",  alpha=0.45, zorder=0)
    ax.set_axisbelow(True)
    ax.set_ylim(bottom=YMIN_DISPLAY, top=YMAX_PAD)
    mids = (x_positions[:-1] + x_positions[1:]) / 2
    for xm in mids:
        ax.axvline(xm, color="#cccccc", linewidth=0.8, linestyle="-", zorder=0)
    # double diagonal cut lines to indicate broken axis
    cut_mid = YMIN_DISPLAY + 0.5 * (YMIN_CLIP - YMIN_DISPLAY)
    cut_y = (cut_mid - YMIN_DISPLAY) / (YMAX_PAD - YMIN_DISPLAY)  # in axes coords
    d, gap = 0.020, 0.018
    lkw = dict(transform=ax.transAxes, color="k", lw=1.5, clip_on=False, zorder=21)
    ekw = dict(transform=ax.transAxes, color="white", lw=6,  clip_on=False, zorder=20)
    for dy in (-gap / 2, gap / 2):
        y0, y1 = cut_y + dy - d, cut_y + dy + d
        ax.plot([-d, d], [y0, y1], **ekw)
        ax.plot([-d, d], [y0, y1], **lkw)


def main():
    data       = load_data()
    devices    = get_devices_ordered(data)
    dlabels    = [get_device_label(data, d) for d in devices]
    benchmarks = get_benchmarks(data)

    ratios  = compute_ratios(data)
    n_dev   = len(devices)
    n_bench = len(benchmarks)
    SEP     = 1.4

    fig, (ax_l, ax_r) = plt.subplots(
        1, 2, sharey=True, figsize=(12, 4.3),
        gridspec_kw={"width_ratios": [1, 1], "wspace": 0.04},
    )

    # ── Left panel: X = devices, dodge = benchmarks ───────────────────────
    xl      = np.arange(n_dev) * SEP
    dodge_b = np.linspace(-0.38, 0.38, n_bench)
    for i, (bench, color, marker) in enumerate(zip(benchmarks, BENCH_COLORS, BENCH_MARKERS)):
        for j, device in enumerate(devices):
            draw_strip(ax_l, xl[j] + dodge_b[i], bench, device, ratios, color, marker)
        ax_l.scatter([], [], color=color, s=55, marker=marker, label=get_benchmark_label(data, bench))

    style_ax(ax_l, dlabels, ylabel="Threaded Speedup (non-threaded / threaded)", x_positions=xl, tick_fontsize=11)
    ax_l.legend(fontsize=10, framealpha=0.9, loc="upper left", ncol=1,
                handletextpad=0.4)

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
            draw_strip(ax_r, xr[j] + dodge_d[i], bench, device, ratios, color, marker)
        ax_r.scatter([], [], color=color, s=55, marker=marker, label=get_device_label(data, device))

    style_ax(ax_r, blabels_r, tick_fontsize=10, x_positions=xr)
    ax_r.tick_params(axis="y", which="both", left=False)
    ax_r.legend(fontsize=10, framealpha=0.9, loc="upper left", handletextpad=0.4)

    PLOTS_DIR.mkdir(parents=True, exist_ok=True)
    out = PLOTS_DIR / "threaded_speedup.pdf"
    fig.tight_layout()
    fig.savefig(out, bbox_inches="tight", dpi=150)
    print(f"Saved: {out}")


if __name__ == "__main__":
    main()
