#!/usr/bin/env python3
"""Two-panel recording overhead plot (socat+variant / devnull time ratio).
Left panel:  X = devices,    dodge = benchmarks, 3 numbered dots per strip.
Right panel: X = benchmarks, dodge = devices,    3 numbered dots per strip.
Numbers 1/2/3 = wasmtime-native / wasmtime-wasm / wizeng.
Y axis shared; right panel suppresses Y tick labels."""

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

SOCAT_VARIANTS = ["socat+replay", "socat+decomposed-wt", "socat+wizeng"]
YMAX = 9.0
YMAX_PAD = 9.8   # ylim top (room for arrow + text)
def compute_overhead(data):
    """Returns overhead[bench][variant][device] = best(socat_time) / no-rec_time.
    best = min of threaded and non-threaded socat time."""
    nt = data["non_threaded"]
    th = data["threaded"]
    out = {}
    for bench in data["metadata"]["benchmarks"]:
        out[bench] = {}
        for var in SOCAT_VARIANTS:
            out[bench][var] = {
                d: min(nt[bench]["time_s"][var][d], th[bench]["time_s"][var][d])
                   / nt[bench]["time_s"]["no-rec"][d]
                for d in data["metadata"]["devices"]
            }
    return out


def draw_strip(ax, xpos, bench, device, overhead, color, marker):
    """Plot 3 dots connected by a vertical line; number only the worst (highest) one.
    Values above YMAX are clipped: upward arrow + annotated actual value."""
    ys    = [overhead[bench][var][device] for var in SOCAT_VARIANTS]
    worst = int(np.argmax(ys))
    raw_max = max(ys)

    ys_clip = [min(y, YMAX) for y in ys]
    disp_max = min(raw_max, YMAX)

    ax.plot([xpos, xpos], [min(ys), disp_max], color=color,
            linewidth=1.5, alpha=0.5, zorder=2)

    if raw_max > YMAX:
        ax.annotate("", xy=(xpos, YMAX + 0.55), xytext=(xpos, YMAX),
                    arrowprops=dict(arrowstyle="-|>", color=color,
                                   lw=1.5, mutation_scale=10), zorder=4)
        ax.text(xpos, YMAX + 0.45, f"{raw_max:.1f}×",
                ha="center", va="bottom", fontsize=9, color=color,
                rotation=0, fontweight="bold", zorder=5,
                bbox=dict(boxstyle="square,pad=0.05", fc="white", ec="none"))

    for k, (y, num) in enumerate(zip(ys_clip, ["1", "2", "3"])):
        ax.scatter(xpos, y, color=color, s=110, zorder=3, alpha=0.88,
                   marker=marker, edgecolors="none")
        if k == worst:
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
    ax.yaxis.set_major_locator(ticker.MultipleLocator(1.0))
    ax.yaxis.set_minor_locator(ticker.MultipleLocator(0.5))
    ax.yaxis.set_tick_params(labelsize=10)
    ax.grid(axis="y", which="major", linestyle="--", alpha=0.4, zorder=0)
    ax.grid(axis="y", which="minor", linestyle=":",  alpha=0.45, zorder=0)
    ax.set_axisbelow(True)
    ax.set_ylim(top=YMAX_PAD)
    # vertical separators between groups
    mids = (x_positions[:-1] + x_positions[1:]) / 2
    for xm in mids:
        ax.axvline(xm, color="#cccccc", linewidth=0.8, linestyle="-", zorder=0)


def main():
    data     = load_data()
    devices  = get_devices_ordered(data)
    dlabels  = [get_device_label(data, d) for d in devices]
    benchmarks = get_benchmarks(data)
    blabels  = [get_benchmark_label(data, b).replace("-", "-\n", 1) for b in benchmarks]

    overhead = compute_overhead(data)

    n_dev   = len(devices)
    n_bench = len(benchmarks)

    SEP = 1.4   # multiplier between group centres

    fig, (ax_l, ax_r) = plt.subplots(
        1, 2, sharey=True, figsize=(12, 4.3),
        gridspec_kw={"width_ratios": [1, 1], "wspace": 0.04},
    )

    # ── Left panel: X = devices, dodge = benchmarks ───────────────────────
    xl = np.arange(n_dev) * SEP
    dodge_b = np.linspace(-0.38, 0.38, n_bench)
    for i, (bench, color, marker) in enumerate(zip(benchmarks, BENCH_COLORS, BENCH_MARKERS)):
        for j, device in enumerate(devices):
            draw_strip(ax_l, xl[j] + dodge_b[i], bench, device, overhead, color, marker)
        ax_l.scatter([], [], color=color, s=55, marker=marker, label=get_benchmark_label(data, bench))

    style_ax(ax_l, dlabels, ylabel="Recording Overhead + Streaming", x_positions=xl, tick_fontsize=11)
    ax_l.legend(fontsize=10, framealpha=0.9, loc="upper right", ncol=2,
                handletextpad=0.4, columnspacing=0.8)

    # ── Right panel: X = benchmarks, dodge = devices ──────────────────────
    benchmarks_r = list(benchmarks)
    ri, bi = benchmarks_r.index("compress-l3"), benchmarks_r.index("blake3")
    benchmarks_r[ri], benchmarks_r[bi] = benchmarks_r[bi], benchmarks_r[ri]
    ci, regi = benchmarks_r.index("compress-l3"), benchmarks_r.index("regex-redux")
    benchmarks_r[ci], benchmarks_r[regi] = benchmarks_r[regi], benchmarks_r[ci]
    blabels_r = [get_benchmark_label(data, b).replace("-", "-\n", 1) for b in benchmarks_r]

    xr = np.arange(n_bench) * SEP
    dodge_d = np.linspace(-0.32, 0.32, n_dev)
    for i, (device, color, marker) in enumerate(zip(devices, DEVICE_COLORS, DEVICE_MARKERS)):
        for j, bench in enumerate(benchmarks_r):
            draw_strip(ax_r, xr[j] + dodge_d[i], bench, device, overhead, color, marker)
        ax_r.scatter([], [], color=color, s=55, marker=marker, label=get_device_label(data, device))

    style_ax(ax_r, blabels_r, tick_fontsize=10, x_positions=xr)
    ax_r.tick_params(axis="y", which="both", left=False)
    ax_r.legend(fontsize=10, framealpha=0.9, loc="upper right",
                handletextpad=0.4)

    # ── Shared variant key ─────────────────────────────────────────────────
    PLOTS_DIR.mkdir(parents=True, exist_ok=True)
    out = PLOTS_DIR / "record_streaming_overhead.pdf"
    fig.tight_layout()
    fig.savefig(out, bbox_inches="tight", dpi=150)
    print(f"Saved: {out}")


if __name__ == "__main__":
    main()
