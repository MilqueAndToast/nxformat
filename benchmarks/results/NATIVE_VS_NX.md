# Decrypted package vs NX: level, threshold and chunk size

Games (Switch 1, base only):

- **30XX**: decrypted DNSP of 1,149,115,716 bytes. About half the bytes
  compress.
- **Yoshi's Crafted World**: decrypted DXCI of 7,985,954,816 bytes, about
  6.2 GB in the secure partition, mostly RAW.

Windows 11, warm OS cache, 2026-11-07. Each variant was run once with
`nxconvert pack SRC OUT --level L --min-gain-bp G --compressed-chunk-kib C`.
Every number below comes from
[`native_settings_raw.txt`](native_settings_raw.txt).

Tools: [`native_vs_nx.py`](../native_vs_nx.py) and
[`nx_read_list.cpp`](../nx_read_list.cpp) (shipped C++ reader, MSVC /O2).
Each run makes 300 random 4 KiB reads (seed 1234), weighted by member size,
and keeps the second pass. Reading the same bytes straight from the package
takes 1.7 to 2.6 µs median.

## 30XX

| Chunk | Level | Threshold | Size | Pack time | Median | p99 |
|---|---:|---:|---:|---:|---:|---:|
| 64 KiB | 3 | 25 % | 0.606 GB | 1.4 s | 39.8 µs | 85.8 µs |
| 64 KiB | 3 | 20 % | 0.599 GB | 1.4 s | 40.7 µs | 92.5 µs |
| 64 KiB | 9 | 25 % | 0.598 GB | 3.0 s | 34.5 µs | 84.9 µs |
| 64 KiB | 9 | 20 % | 0.589 GB | 3.0 s | 35.0 µs | 92.2 µs |
| 64 KiB | 13 | 25 % | 0.604 GB | 5.9 s | 34.2 µs | 84.1 µs |
| 64 KiB | 13 | 20 % | 0.595 GB | 6.1 s | 34.9 µs | 92.4 µs |
| 128 KiB | 3 | 25 % | 0.604 GB | 1.4 s | 88.1 µs | 191.8 µs |
| 128 KiB | 3 | 20 % | 0.597 GB | 1.3 s | 90.3 µs | 162.6 µs |
| 128 KiB | 9 | 25 % | 0.595 GB | 2.9 s | 77.1 µs | 155.8 µs |
| 128 KiB | 9 | 20 % | 0.584 GB | 3.0 s | 79.8 µs | 159.5 µs |
| 128 KiB | 13 | 25 % | 0.599 GB | 6.7 s | 87.0 µs | 163.3 µs |
| 128 KiB | 13 | 20 % | 0.588 GB | 7.0 s | 90.0 µs | 168.0 µs |
| off | 3 | 25 % | 0.598 GB | 1.3 s | 166.5 µs | 319.0 µs |
| off | 3 | 20 % | 0.588 GB | 1.3 s | 169.4 µs | 320.8 µs |
| off | 9 | 25 % | 0.591 GB | 2.9 s | 146.9 µs | 286.1 µs |
| off | 9 | 20 % | 0.579 GB | 3.0 s | 149.0 µs | 307.5 µs |
| off | 13 | 25 % | 0.594 GB | 8.8 s | 153.6 µs | 322.1 µs |
| off | 13 | 20 % | 0.582 GB | 9.1 s | 157.1 µs | 311.1 µs |

## Yoshi's Crafted World

| Chunk | Level | Threshold | Size | Pack time | Median | p99 |
|---|---:|---:|---:|---:|---:|---:|
| 64 KiB | 3 | 25 % | 5.669 GB | 6.9 s | 3.8 µs | 10.9 µs |
| 64 KiB | 3 | 20 % | 5.667 GB | 7.4 s | 4.0 µs | 11.2 µs |
| 64 KiB | 9 | 25 % | 5.668 GB | 7.4 s | 3.7 µs | 12.4 µs |
| 64 KiB | 9 | 20 % | 5.665 GB | 7.3 s | 3.9 µs | 9.4 µs |
| 64 KiB | 13 | 25 % | 5.668 GB | 7.7 s | 4.0 µs | 14.2 µs |
| 64 KiB | 13 | 20 % | 5.665 GB | 7.7 s | 4.2 µs | 12.6 µs |
| 128 KiB | 3 | 25 % | 5.667 GB | 7.2 s | 4.0 µs | 10.1 µs |
| 128 KiB | 3 | 20 % | 5.665 GB | 6.5 s | 3.9 µs | 11.8 µs |
| 128 KiB | 9 | 25 % | 5.666 GB | 7.2 s | 3.7 µs | 9.7 µs |
| 128 KiB | 9 | 20 % | 5.662 GB | 7.0 s | 3.8 µs | 14.9 µs |
| 128 KiB | 13 | 25 % | 5.665 GB | 7.4 s | 3.9 µs | 11.3 µs |
| 128 KiB | 13 | 20 % | 5.660 GB | 7.4 s | 3.7 µs | 17.1 µs |
| off | 3 | 25 % | 5.664 GB | 6.6 s | 3.8 µs | 9.2 µs |
| off | 3 | 20 % | 5.660 GB | 6.7 s | 3.9 µs | 94.4 µs |
| off | 9 | 25 % | 5.663 GB | 6.8 s | 3.8 µs | 10.4 µs |
| off | 9 | 20 % | 5.657 GB | 7.8 s | 3.9 µs | 70.0 µs |
| off | 13 | 25 % | 5.662 GB | 7.4 s | 3.9 µs | 10.8 µs |
| off | 13 | 20 % | 5.656 GB | 6.9 s | 3.7 µs | 70.8 µs |

## What the numbers say

- **Chunk size is the lever.** On 30XX, 64 KiB chunks make reads 4x faster
  than no re-chunking (median 40 vs 167 µs, p99 86 vs 319 µs) for a 1.3 %
  larger archive. 128 KiB is halfway. On Yoshi, chunk size changes almost
  nothing. 64 KiB stays the default (ADR-053).
- **Level barely matters.** Level 9 saves about 1 % on 30XX for 2.2x the
  pack time. Level 13 saves less and is slower still. Level 3 stays the
  default.
- **Keep the 25 % threshold.** 20 % saves 1-1.5 % on 30XX. Without
  re-chunking it raised Yoshi's p99 from about 10 to 70-94 µs.
