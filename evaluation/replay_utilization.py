#!/usr/bin/env python3
"""Plot replay CPU utilization trend: devices ordered weak→strong on X,
one line per variant (median across benchmarks), shaded band for min/max."""

import numpy as np
import matplotlib.pyplot as plt
import matplotlib.ticker as ticker

from data_utils import (
    load_data, get_devices, get_devices_ordered, get_device_label,
    get_benchmarks, get_cpu_variants, get_replay_variant_labels, PLOTS_DIR,
)

VARIANT_COLORS  = ["#4C72B0", "#DD8452", "#55A868"]
VARIANT_MARKERS = ["o", "D", "^"]


def get_all_values(data):
    """Returns {variant: {device: [values across benchmarks]}}."""
    nt = data["non_threaded"]["cpu_utilization"]
    th = data["threaded"]["cpu_utilization"]
    devices = get_devices(data)
    benchmarks = get_benchmarks(data)
    variants = get_cpu_variants(data)

    return {
        var: {
            device: [max(nt[bench][var][device], th[bench][var][device]) for bench in benchmarks]
            for device in devices
        }
        for var in variants
    }



def main():
    data = load_data()
    variants = get_cpu_variants(data)
    variant_labels = get_replay_variant_labels(data)

    all_values = get_all_values(data)
    ordered_devices = get_devices_ordered(data)
    device_labels = [get_device_label(data, d) for d in ordered_devices]

    x = np.arange(len(ordered_devices))

    fig, ax = plt.subplots(figsize=(5.5, 3.2))

    n_variants = len(variants)
    dodge = np.linspace(-0.15, 0.15, n_variants)

    for i, (var, label, color, marker) in enumerate(zip(variants, variant_labels, VARIANT_COLORS, VARIANT_MARKERS)):
        xd = x + dodge[i]
        medians = [np.median(all_values[var][d]) for d in ordered_devices]
        mins    = [np.min(all_values[var][d])    for d in ordered_devices]
        maxs    = [np.max(all_values[var][d])    for d in ordered_devices]

        for j, d in enumerate(ordered_devices):
            ys = all_values[var][d]
            ax.plot([xd[j], xd[j]], [min(ys), max(ys)], color=color,
                    linewidth=1.5, zorder=2, alpha=0.5)
            ax.scatter([xd[j]] * len(ys), ys, color=color, s=30, zorder=3,
                       marker=marker, alpha=0.8, label=label if j == 0 else None)
            ax.plot([xd[j] - 0.09, xd[j] + 0.09], [np.median(ys)] * 2,
                    color="white", linewidth=6, zorder=5, solid_capstyle="butt")
            ax.plot([xd[j] - 0.09, xd[j] + 0.09], [np.median(ys)] * 2,
                    color=color, linewidth=4, zorder=6, solid_capstyle="butt")

    ax.set_xticks(x)
    ax.set_xticklabels(device_labels, fontsize=11)
    ax.set_ylabel("Replay CPU Utilization (%)", fontsize=11)
    ax.set_ylim(0, 100)
    ax.yaxis.set_major_locator(ticker.MultipleLocator(20))
    ax.yaxis.set_minor_locator(ticker.MultipleLocator(10))
    ax.yaxis.set_tick_params(labelsize=11)
    ax.grid(axis="y", which="major", linestyle="--", alpha=0.4, zorder=0)
    ax.grid(axis="y", which="minor", linestyle=":", alpha=0.45, zorder=0)
    ax.set_axisbelow(True)
    ax.legend(fontsize=10, framealpha=0.9, loc="upper right")

    PLOTS_DIR.mkdir(parents=True, exist_ok=True)
    out = PLOTS_DIR / "replay_utilization.pdf"
    fig.tight_layout()
    fig.savefig(out, bbox_inches="tight", dpi=150)
    print(f"Saved: {out}")


if __name__ == "__main__":
    main()
