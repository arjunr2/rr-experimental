# Offloading Benchmarks Results

5 devices x 6 benchmarks x 6 variants.
Non-threaded: 5 runs each (5 for milkv-duo). Threaded: 5 runs (3 for milkv-duo).
Values are **median** across runs.

Recording parameters:
- Non-threaded: `buffer-size=8192`
- Threaded: `buffer-size=8192,threaded,channels=128`

Devices: mac-mini (M2 Pro), nuc11 (i7-1165G7), aplos (Ryzen 5 4500U), pi0 (Cortex-A53), milkv-duo (C906 RISC-V)

---

## Non-threaded

### compress-l3

| variant | mac-mini | nuc11 | aplos | pi0 | milkv-duo |
|---------|---------|---------|---------|---------|---------|
| **native** | 0.152 | 0.517 | 0.132 | 13.401 | 12.501 |
| **no-rec** | 0.667 | 2.099 | 0.320 | 18.337 | 15.032 |
| **devnull** | 1.182 | 2.765 | 0.979 | 21.914 | 40.707 |
| **socat+wizeng** | 3.146 | 5.311 | 3.154 | 28.857 | 85.437 |
| **socat+replay** | 1.235 | 5.282 | 1.115 | 29.216 | 85.042 |
| **socat+decomposed-wt** | 1.535 | 5.278 | 1.305 | 29.055 | 85.306 |
| **trace size (MB)** | 96.9 | 96.9 | 96.9 | 96.9 | 96.9 |
| **trace peak (MB/s)** | 182.2 | 65.2 | 125.4 | 8.1 | 3.3 |
| **trace avg (MB/s)** | 80.4 | 34.9 | 91.1 | 4.3 | 2.4 |

### sort-10k

| variant | mac-mini | nuc11 | aplos | pi0 | milkv-duo |
|---------|---------|---------|---------|---------|---------|
| **native** | 0.425 | 0.614 | 0.555 | 20.153 | 26.697 |
| **no-rec** | 0.952 | 3.744 | 1.572 | 29.183 | 57.234 |
| **devnull** | 3.056 | 8.266 | 4.116 | 69.797 | 225.658 |
| **socat+wizeng** | 8.022 | 28.583 | 8.006 | 82.709 | 354.169 |
| **socat+replay** | 2.756 | 27.604 | 3.981 | 81.971 | 350.460 |
| **socat+decomposed-wt** | 3.429 | 27.646 | 4.174 | 82.914 | 353.640 |
| **trace size (MB)** | 218.2 | 218.2 | 218.2 | 218.2 | 218.2 |
| **trace peak (MB/s)** | 129.1 | 42.7 | 89.0 | 6.1 | 2.5 |
| **trace avg (MB/s)** | 71.5 | 26.4 | 51.9 | 3.1 | 1.0 |

### regex-redux

| variant | mac-mini | nuc11 | aplos | pi0 | milkv-duo |
|---------|---------|---------|---------|---------|---------|
| **native** | 0.090 | 0.101 | 0.131 | 1.873 | 5.723 |
| **no-rec** | 0.364 | 0.304 | 0.365 | 5.527 | 10.480 |
| **devnull** | 0.391 | 0.336 | 0.418 | 6.265 | 12.062 |
| **socat+wizeng** | 0.699 | 0.601 | 0.508 | 7.803 | 12.853 |
| **socat+replay** | 0.448 | 0.379 | 0.443 | 7.793 | 12.915 |
| **socat+decomposed-wt** | 0.491 | 0.432 | 0.440 | 7.819 | 12.874 |
| **trace size (MB)** | 9.7 | 9.7 | 9.7 | 9.7 | 9.7 |
| **trace peak (MB/s)** | 370.2 | 310.7 | 203.3 | 17.1 | 7.5 |
| **trace avg (MB/s)** | 23.1 | 28.7 | 23.1 | 1.5 | 0.8 |

### json-process

| variant | mac-mini | nuc11 | aplos | pi0 | milkv-duo |
|---------|---------|---------|---------|---------|---------|
| **native** | 1.319 | 2.499 | 2.031 | 27.288 | 117.249 |
| **no-rec** | 5.530 | 3.334 | 3.747 | 49.721 | 233.563 |
| **devnull** | 5.725 | 3.616 | 3.937 | 53.718 | 228.169 |
| **socat+wizeng** | 6.988 | 5.081 | 5.030 | 61.745 | 239.278 |
| **socat+replay** | 5.906 | 4.147 | 4.146 | 61.837 | 240.945 |
| **socat+decomposed-wt** | 6.248 | 4.432 | 4.307 | 61.868 | 240.782 |
| **trace size (MB)** | 45.0 | 45.0 | 45.0 | 45.0 | 45.0 |
| **trace peak (MB/s)** | 332.1 | 311.5 | 227.8 | 17.1 | 7.7 |
| **trace avg (MB/s)** | 7.4 | 11.9 | 10.9 | 0.8 | 0.2 |

### speedtest1

| variant | mac-mini | nuc11 | aplos | pi0 | milkv-duo |
|---------|---------|---------|---------|---------|---------|
| **native** | 0.179 | 0.302 | 0.213 | 3.184 | 6.862 |
| **no-rec** | 1.109 | 4.689 | 0.796 | 13.403 | 36.544 |
| **devnull** | 1.614 | 5.527 | 1.158 | 22.199 | 61.937 |
| **socat+wizeng** | 1.671 | 10.451 | 1.240 | 25.581 | 129.970 |
| **socat+replay** | 1.590 | 10.380 | 1.152 | 25.489 | 131.322 |
| **socat+decomposed-wt** | 1.587 | 10.398 | 1.160 | 25.593 | 133.093 |
| **trace size (MB)** | 10.6 | 10.6 | 10.6 | 10.6 | 10.3 |
| **trace peak (MB/s)** | 138.0 | 58.9 | 111.2 | 7.9 | 2.5 |
| **trace avg (MB/s)** | 6.5 | 1.9 | 9.2 | 0.5 | 0.2 |

### blake3

| variant | mac-mini | nuc11 | aplos | pi0 | milkv-duo |
|---------|---------|---------|---------|---------|---------|
| **native** | 0.078 | 0.032 | 0.052 | 4.716 | 4.889 |
| **no-rec** | 0.267 | 0.436 | 0.444 | 5.248 | 14.116 |
| **devnull** | 0.587 | 0.828 | 0.962 | 11.499 | 29.163 |
| **socat+wizeng** | 3.107 | 3.158 | 3.129 | 24.395 | 40.387 |
| **socat+replay** | 0.693 | 1.066 | 1.010 | 24.473 | 40.447 |
| **socat+decomposed-wt** | 1.466 | 1.540 | 1.476 | 24.069 | 40.375 |
| **trace size (MB)** | 100.1 | 100.1 | 100.1 | 100.1 | 100.1 |
| **trace peak (MB/s)** | 175.6 | 124.5 | 105.0 | 8.8 | 3.6 |
| **trace avg (MB/s)** | 168.6 | 120.6 | 103.9 | 8.7 | 3.4 |

### Replay CPU utilization (non-threaded)

| benchmark | variant | mac-mini | nuc11 | aplos | pi0 | milkv-duo |
|-----------|---------|---------|---------|---------|---------|---------|
| compress-l3 | wizeng | 83.6% | 42.2% | 85.4% | 11.9% | 5.9% |
| compress-l3 | wt-replay | 25.2% | 7.7% | 25.6% | 1.9% | 1.5% |
| compress-l3 | decomposed-wt | 62.1% | 19.9% | 69.7% | 4.5% | 3.0% |
| sort-10k | wizeng | 93.2% | 29.7% | 93.5% | 11.5% | 3.8% |
| sort-10k | wt-replay | 43.9% | 7.7% | 34.6% | 2.4% | 1.3% |
| sort-10k | decomposed-wt | 84.9% | 15.0% | 72.2% | 5.0% | 2.1% |
| regex-redux | wizeng | 63.0% | 24.3% | 65.0% | 9.4% | 8.9% |
| regex-redux | wt-replay | 29.1% | 9.4% | 28.9% | 2.9% | 2.5% |
| regex-redux | decomposed-wt | 37.7% | 12.4% | 40.7% | 3.9% | 3.4% |
| json-process | wizeng | 79.3% | 68.6% | 91.2% | 10.9% | 3.2% |
| json-process | wt-replay | 38.0% | 32.2% | 47.2% | 3.7% | 1.2% |
| json-process | decomposed-wt | 42.2% | 36.7% | 54.2% | 4.4% | 1.4% |
| speedtest1 | wizeng | 45.1% | 11.9% | 54.5% | 5.7% | 1.7% |
| speedtest1 | wt-replay | 14.4% | 5.4% | 19.9% | 1.9% | 0.9% |
| speedtest1 | decomposed-wt | 19.9% | 6.8% | 27.1% | 2.6% | 1.2% |
| blake3 | wizeng | 85.0% | 53.5% | 85.8% | 13.7% | 9.7% |
| blake3 | wt-replay | 46.6% | 16.3% | 37.5% | 2.4% | 2.3% |
| blake3 | decomposed-wt | 72.6% | 35.4% | 74.6% | 6.8% | 5.1% |

---

## Threaded

### compress-l3

| variant | mac-mini | nuc11 | aplos | pi0 | milkv-duo |
|---------|---------|---------|---------|---------|---------|
| **native** | 0.154 | 0.521 | 0.135 | 13.040 | 12.498 |
| **no-rec** | 0.667 | 2.094 | 0.324 | 19.183 | 14.347 |
| **devnull** | 1.518 | 3.008 | 1.468 | 22.071 | 79.588 |
| **socat+wizeng** | 3.145 | 3.846 | 3.104 | 30.307 | 148.342 |
| **socat+replay** | 1.529 | 3.184 | 1.398 | 29.763 | 147.210 |
| **socat+decomposed-wt** | 1.647 | 3.301 | 1.434 | 30.182 | 148.906 |
| **trace size (MB)** | 96.9 | 96.9 | 96.9 | 96.9 | 96.9 |
| **trace peak (MB/s)** | 111.4 | 53.6 | 79.2 | 6.1 | 1.6 |
| **trace avg (MB/s)** | 62.9 | 32.1 | 62.3 | 4.1 | 1.2 |

### sort-10k

| variant | mac-mini | nuc11 | aplos | pi0 | milkv-duo |
|---------|---------|---------|---------|---------|---------|
| **native** | 0.427 | 0.616 | 0.560 | 20.102 | 27.133 |
| **no-rec** | 0.947 | 3.785 | 1.548 | 29.452 | 60.118 |
| **devnull** | 3.022 | 6.682 | 4.506 | 58.877 | 488.739 |
| **socat+wizeng** | 8.064 | 24.565 | 7.971 | 73.028 | 1108.791 |
| **socat+replay** | 3.022 | 23.614 | 4.511 | 72.452 | 1211.648 |
| **socat+decomposed-wt** | 3.375 | 23.602 | 4.683 | 72.407 | 1155.712 |
| **trace size (MB)** | 218.2 | 218.2 | 218.2 | 218.2 | 218.2 |
| **trace peak (MB/s)** | 90.5 | 35.8 | 64.5 | 4.8 | 1.4 |
| **trace avg (MB/s)** | 71.8 | 32.6 | 47.5 | 3.7 | 0.5 |

### regex-redux

| variant | mac-mini | nuc11 | aplos | pi0 | milkv-duo |
|---------|---------|---------|---------|---------|---------|
| **native** | 0.091 | 0.102 | 0.125 | 1.893 | 5.709 |
| **no-rec** | 0.367 | 0.303 | 0.365 | 5.526 | 10.620 |
| **devnull** | 0.430 | 0.380 | 0.467 | 6.796 | 13.853 |
| **socat+wizeng** | 0.648 | 0.567 | 0.482 | 7.627 | 14.665 |
| **socat+replay** | 0.442 | 0.398 | 0.491 | 7.736 | 14.650 |
| **socat+decomposed-wt** | 0.449 | 0.431 | 0.485 | 7.575 | 14.457 |
| **trace size (MB)** | 9.7 | 9.7 | 9.7 | 9.7 | 9.7 |
| **trace peak (MB/s)** | 148.3 | 130.4 | 102.8 | 8.6 | 3.3 |
| **trace avg (MB/s)** | 21.1 | 25.4 | 20.7 | 1.4 | 0.7 |

### json-process

| variant | mac-mini | nuc11 | aplos | pi0 | milkv-duo |
|---------|---------|---------|---------|---------|---------|
| **native** | 1.331 | 2.484 | 2.046 | 27.022 | 116.749 |
| **no-rec** | 5.534 | 3.340 | 3.764 | 49.897 | 232.823 |
| **devnull** | 5.889 | 3.833 | 4.156 | 55.935 | 239.371 |
| **socat+wizeng** | 6.980 | 4.824 | 4.986 | 61.761 | 249.919 |
| **socat+replay** | 5.918 | 3.882 | 4.151 | 61.923 | 250.843 |
| **socat+decomposed-wt** | 6.219 | 4.117 | 4.313 | 61.726 | 249.393 |
| **trace size (MB)** | 45.0 | 45.0 | 45.0 | 45.0 | 45.0 |
| **trace peak (MB/s)** | 147.3 | 130.5 | 116.8 | 8.6 | 3.3 |
| **trace avg (MB/s)** | 7.2 | 11.2 | 10.4 | 0.8 | 0.2 |

### speedtest1

| variant | mac-mini | nuc11 | aplos | pi0 | milkv-duo |
|---------|---------|---------|---------|---------|---------|
| **native** | 0.180 | 0.301 | 0.216 | 3.185 | 6.864 |
| **no-rec** | 1.107 | 4.704 | 0.796 | 13.422 | 36.513 |
| **devnull** | 1.490 | 4.955 | 1.160 | 18.179 | 110.760 |
| **socat+wizeng** | 1.563 | 6.330 | 1.220 | 20.870 | 225.571 |
| **socat+replay** | 1.497 | 6.232 | 1.125 | 20.768 | 223.393 |
| **socat+decomposed-wt** | 1.502 | 6.223 | 1.135 | 20.822 | 225.959 |
| **trace size (MB)** | 10.6 | 10.6 | 10.6 | 10.6 | 10.3 |
| **trace peak (MB/s)** | 94.2 | 49.3 | 70.4 | 6.4 | 1.3 |
| **trace avg (MB/s)** | 7.0 | 2.1 | 9.2 | 0.6 | 0.1 |

### blake3

| variant | mac-mini | nuc11 | aplos | pi0 | milkv-duo |
|---------|---------|---------|---------|---------|---------|
| **native** | 0.078 | 0.033 | 0.053 | 4.664 | 4.892 |
| **no-rec** | 0.267 | 0.436 | 0.450 | 5.336 | 14.110 |
| **devnull** | 0.957 | 1.331 | 1.571 | 17.302 | 47.687 |
| **socat+wizeng** | 3.213 | 3.158 | 3.087 | 24.644 | 59.942 |
| **socat+replay** | 0.959 | 1.255 | 1.448 | 25.107 | 59.810 |
| **socat+decomposed-wt** | 1.487 | 1.545 | 1.492 | 25.059 | 59.918 |
| **trace size (MB)** | 100.1 | 100.1 | 100.1 | 100.1 | 100.1 |
| **trace peak (MB/s)** | 106.2 | 76.6 | 68.7 | 5.8 | 2.2 |
| **trace avg (MB/s)** | 103.4 | 75.1 | 63.7 | 5.8 | 2.1 |

### Replay CPU utilization (threaded)

| benchmark | variant | mac-mini | nuc11 | aplos | pi0 | milkv-duo |
|-----------|---------|---------|---------|---------|---------|---------|
| compress-l3 | wizeng | 83.7% | 49.2% | 85.6% | 11.7% | 3.7% |
| compress-l3 | wt-replay | 22.5% | 8.8% | 23.3% | 1.9% | 1.1% |
| compress-l3 | decomposed-wt | 59.5% | 23.9% | 66.9% | 4.6% | 2.0% |
| sort-10k | wizeng | 93.2% | 33.3% | 93.7% | 13.1% | 1.7% |
| sort-10k | wt-replay | 40.9% | 8.3% | 32.6% | 2.6% | 0.8% |
| sort-10k | decomposed-wt | 84.9% | 16.4% | 66.1% | 5.6% | 1.2% |
| regex-redux | wizeng | 62.9% | 24.3% | 64.6% | 9.5% | 8.4% |
| regex-redux | wt-replay | 29.8% | 9.7% | 29.3% | 2.9% | 2.3% |
| regex-redux | decomposed-wt | 39.0% | 12.5% | 39.1% | 3.9% | 3.3% |
| json-process | wizeng | 80.0% | 69.1% | 91.1% | 11.1% | 3.2% |
| json-process | wt-replay | 38.0% | 33.1% | 48.2% | 3.8% | 1.1% |
| json-process | decomposed-wt | 42.5% | 37.9% | 55.1% | 4.5% | 1.4% |
| speedtest1 | wizeng | 47.8% | 14.4% | 55.6% | 6.5% | 1.2% |
| speedtest1 | wt-replay | 15.0% | 5.1% | 20.5% | 2.1% | 0.7% |
| speedtest1 | decomposed-wt | 20.8% | 6.6% | 27.4% | 2.9% | 0.9% |
| blake3 | wizeng | 85.1% | 53.4% | 85.9% | 13.5% | 7.8% |
| blake3 | wt-replay | 38.2% | 15.8% | 29.5% | 2.4% | 1.8% |
| blake3 | decomposed-wt | 73.1% | 35.5% | 74.3% | 6.6% | 4.1% |

---

## Recording overhead (non-threaded, median no-rec -> devnull ratio)

| benchmark | mac-mini | nuc11 | aplos | pi0 | milkv-duo |
|-----------|---------|---------|---------|---------|---------|
| compress-l3 | 1.77x | 1.32x | 3.06x | 1.20x | 2.71x |
| sort-10k | 3.21x | 2.21x | 2.62x | 2.39x | 3.94x |
| regex-redux | 1.07x | 1.11x | 1.15x | 1.13x | 1.15x |
| json-process | 1.04x | 1.08x | 1.05x | 1.08x | 0.98x |
| speedtest1 | 1.46x | 1.18x | 1.45x | 1.66x | 1.69x |
| blake3 | 2.20x | 1.90x | 2.17x | 2.19x | 2.07x |

## Threaded vs Non-threaded Comparison (median time ratio)

Values < 1.0 = threaded is faster. Values > 1.0 = threaded is slower.

| benchmark | variant | mac-mini | nuc11 | aplos | pi0 | milkv-duo |
|-----------|---------|---------|---------|---------|---------|---------|
| compress-l3 | devnull | 1.28x | 1.09x | 1.50x | 1.01x | 1.96x |
| compress-l3 | socat+wizeng | 1.00x | 0.72x | 0.98x | 1.05x | 1.74x |
| compress-l3 | socat+replay | 1.24x | 0.60x | 1.25x | 1.02x | 1.73x |
| compress-l3 | socat+decomposed-wt | 1.07x | 0.63x | 1.10x | 1.04x | 1.75x |
| sort-10k | devnull | 0.99x | 0.81x | 1.09x | 0.84x | 2.17x |
| sort-10k | socat+wizeng | 1.01x | 0.86x | 1.00x | 0.88x | 3.13x |
| sort-10k | socat+replay | 1.10x | 0.86x | 1.13x | 0.88x | 3.46x |
| sort-10k | socat+decomposed-wt | 0.98x | 0.85x | 1.12x | 0.87x | 3.27x |
| regex-redux | devnull | 1.10x | 1.13x | 1.12x | 1.08x | 1.15x |
| regex-redux | socat+wizeng | 0.93x | 0.94x | 0.95x | 0.98x | 1.14x |
| regex-redux | socat+replay | 0.99x | 1.05x | 1.11x | 0.99x | 1.13x |
| regex-redux | socat+decomposed-wt | 0.91x | 1.00x | 1.10x | 0.97x | 1.12x |
| json-process | devnull | 1.03x | 1.06x | 1.06x | 1.04x | 1.05x |
| json-process | socat+wizeng | 1.00x | 0.95x | 0.99x | 1.00x | 1.04x |
| json-process | socat+replay | 1.00x | 0.94x | 1.00x | 1.00x | 1.04x |
| json-process | socat+decomposed-wt | 1.00x | 0.93x | 1.00x | 1.00x | 1.04x |
| speedtest1 | devnull | 0.92x | 0.90x | 1.00x | 0.82x | 1.79x |
| speedtest1 | socat+wizeng | 0.94x | 0.61x | 0.98x | 0.82x | 1.74x |
| speedtest1 | socat+replay | 0.94x | 0.60x | 0.98x | 0.81x | 1.70x |
| speedtest1 | socat+decomposed-wt | 0.95x | 0.60x | 0.98x | 0.81x | 1.70x |
| blake3 | devnull | 1.63x | 1.61x | 1.63x | 1.50x | 1.64x |
| blake3 | socat+wizeng | 1.03x | 1.00x | 0.99x | 1.01x | 1.48x |
| blake3 | socat+replay | 1.38x | 1.18x | 1.43x | 1.03x | 1.48x |
| blake3 | socat+decomposed-wt | 1.01x | 1.00x | 1.01x | 1.04x | 1.48x |

