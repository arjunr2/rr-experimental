#!/usr/bin/env python3
"""Monitor CPU utilization during individual replay runs.
X = platforms, dodge = monitors. Y = CPU utilization (%).
If wall time overhead > 0.1s AND cpu > 95%, cap to 100% (replay-bottlenecked)."""

import json
import numpy as np
import matplotlib.pyplot as plt
import matplotlib.ticker as ticker
from matplotlib.patches import Patch
from pathlib import Path

DATA_FILE = Path(__file__).parent / "data" / "monitor_utilizations.json"
PLOTS_DIR = Path(__file__).parent / "plots"

MONITORS = ["no-monitors", "loops", "hotness", "icount", "profile"]
LABELS   = ["none", "loops", "hotness", "icount", "profile"]
COLORS   = ["#aaaaaa", "#55A868", "#DD8452", "#4C72B0", "#C44E52"]

DEVICE_ORDER = ["mac-mini", "aplos", "nuc11", "pi0", "milkv-duo"]


def load_data():
    with open(DATA_FILE) as f:
        return json.load(f)


def adjusted_cpu(cpu_pct, wall_time, baseline_wall):
    """Cap to 100% if monitor causes >5% relative wall overhead and cpu >90%.
    Returns (value, clipped)."""
    if (wall_time - baseline_wall) / baseline_wall > 0.05 and cpu_pct > 90.0:
        return 100.0, True
    return min(cpu_pct, 100.0), False


def main():
    data = load_data()
    device_labels = data["metadata"]["device_labels"]
    cpu  = data["individual"]["cpu_pct"]
    wall = data["individual"]["wall_time_s"]

    devices    = DEVICE_ORDER
    n_devices  = len(devices)
    n_monitors = len(MONITORS)
    SEP        = 1.12

    fig, ax = plt.subplots(figsize=(8, 4.5))

    xb        = np.arange(n_devices) * SEP
    dodge     = np.linspace(-0.38, 0.38, n_monitors)
    bar_width = dodge[1] - dodge[0]

    legend_handles = []
    for i, (monitor, label, color) in enumerate(zip(MONITORS, LABELS, COLORS)):
        xs = xb + dodge[i]
        vals = [adjusted_cpu(cpu[monitor][d], wall[monitor][d], wall["no-monitors"][d])
                for d in devices]
        ys      = [v for v, _ in vals]
        clipped = [c for _, c in vals]
        ax.bar(xs, ys, width=bar_width * 1.0, color=color, alpha=0.85, zorder=3)
        for x, y, is_clipped in zip(xs, ys, clipped):
            if is_clipped:
                ax.bar(x, y, width=bar_width * 1.0, color=color, alpha=0.85,
                       hatch="xx", edgecolor="white", linewidth=0.4, zorder=4)
        legend_handles.append(Patch(facecolor=color, alpha=0.85, label=label))

    ax.axhline(100.0, color="black", linewidth=0.8, linestyle="--", zorder=1)
    ax.set_xlim(xb[0] - SEP * 0.5, xb[-1] + SEP * 0.5)
    ax.set_xticks(xb)
    ax.set_xticklabels([device_labels[d] for d in devices], fontsize=15)
    ax.set_ylabel("Replay CPU Utilization (%)", fontsize=15)
    ax.yaxis.set_major_locator(ticker.FixedLocator(range(0, 101, 10)))
    ax.yaxis.set_major_formatter(ticker.FuncFormatter(lambda y, _: f"{y:.0f}" if y <= 100 else ""))
    ax.yaxis.set_tick_params(labelsize=14, which="both")
    ax.set_ylim(bottom=0, top=115)
    ax.grid(axis="y", which="major", linestyle="--", alpha=0.4, zorder=0)
    ax.grid(axis="y", which="minor", linestyle=":",  alpha=0.3,  zorder=0)
    ax.set_axisbelow(True)

    mids = (xb[:-1] + xb[1:]) / 2
    for xm in mids:
        ax.axvline(xm, color="#cccccc", linewidth=0.8, linestyle="-", zorder=0)

    ax.legend(handles=legend_handles, fontsize=14, framealpha=0.9,
              loc="upper right", ncol=n_monitors, handletextpad=0.4,
              columnspacing=0.8)

    PLOTS_DIR.mkdir(parents=True, exist_ok=True)
    out = PLOTS_DIR / "monitor_utilizations.pdf"
    fig.tight_layout()
    fig.savefig(out, dpi=150)
    print(f"Saved: {out}")


if __name__ == "__main__":
    main()
