"""Common data loading and schema helpers for evaluation scripts.

All structural info (benchmarks, devices, variants) is derived from the JSON
rather than hardcoded, so adding new entries to the data file is enough.
"""

import json
from pathlib import Path

DATA_FILE = Path(__file__).parent / "data" / "offloading_results.json"
PLOTS_DIR = Path(__file__).parent / "plots"


def load_data(path=DATA_FILE):
    with open(path) as f:
        return json.load(f)


def get_devices(data):
    """Ordered list of device keys, as they appear in metadata."""
    return list(data["metadata"]["devices"].keys())


def get_devices_ordered(data):
    """Device keys in the explicit weak→strong order from metadata."""
    return data["metadata"]["device_order"]


def get_device_label(data, device):
    return data["metadata"]["devices"][device]


def get_device_labels(data):
    return [get_device_label(data, d) for d in get_devices(data)]


def get_benchmarks(data):
    """Ordered list of benchmark keys, as they appear in metadata."""
    return list(data["metadata"]["benchmarks"].keys())


def get_benchmark_label(data, benchmark):
    return data["metadata"]["benchmarks"][benchmark]


def get_benchmark_labels(data):
    return [get_benchmark_label(data, b) for b in get_benchmarks(data)]


def get_cpu_variants(data):
    """Replay variant names, ordered as declared in metadata."""
    return list(data["metadata"]["replay-variants"].keys())


def get_replay_variant_label(data, variant):
    return data["metadata"]["replay-variants"][variant]


def get_replay_variant_labels(data):
    return [get_replay_variant_label(data, v) for v in get_cpu_variants(data)]


def get_socat_variants(data):
    """Socat replay variant keys, ordered as declared in metadata."""
    return list(data["metadata"]["socat-replay-variants"].keys())


def get_socat_variant_label(data, variant):
    return data["metadata"]["socat-replay-variants"][variant]


def get_socat_variant_labels(data):
    return [get_socat_variant_label(data, v) for v in get_socat_variants(data)]


def get_time_variants(data):
    """All variant names for time_s measurements."""
    bench = get_benchmarks(data)[0]
    return list(data["non_threaded"][bench]["time_s"].keys())
