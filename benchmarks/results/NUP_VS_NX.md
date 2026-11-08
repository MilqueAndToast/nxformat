# NUP vs NX: measured on a real game

Game: Donkey Kong Bananza + update 3.2.0 + DLC (Switch 2), the real NUP
(`Donkey Kong Bananza + Update 3.2.0 + DLC.nup`, 8.65 GiB) and the NX
converted from it with `nxconvert pack-nup`. Windows 11, warm OS cache,
2026-11-07. Every number below comes from
[`dkb_settings_raw.txt`](dkb_settings_raw.txt).

Tools: [`nup_layout.py`](../nup_layout.py), [`nup_vs_nx.py`](../nup_vs_nx.py)
and [`nx_read_list.cpp`](../nx_read_list.cpp) (shipped C++ reader, MSVC /O2).

## Summary (level 3, 25 % threshold, no re-chunking)

| | NUP | NX |
|---|---:|---:|
| File size | 8.65 GiB | 9.00 GiB (+4.1 %) |
| Median random 4 KiB read | 579 µs | 5.1 µs (×114) |
| Mean random 4 KiB read | 2,895 µs | 19.1 µs (×152) |
| p99 random 4 KiB read | 29,404 µs | 264 µs (×111) |
| Median read inside a Zstd-stored NUP file | 1,973 µs | 5.3 µs (×372) |
| Open | 15 MiB TOC, 64 ms to parse (Python) | 49 ms open + mount (C++) |
| Conversion NUP → NX | | 233 s (76 s NUP decode, 137 s conversion) |

## Random reads: 300 reads of 4 KiB in the base files

Reads are drawn with a fixed seed, weighted by file size, at a uniform offset
inside the file. NUP stores each file as **one** raw or Zstandard extent, so a
read inside a Zstd file must decode from the start of the file. The NUP side
does exactly that, and no more (it stops once the bytes are decoded).

| Reads | n | NUP median | NUP mean | NUP p99 | NX median | NX mean | NX p99 |
|---|---:|---:|---:|---:|---:|---:|---:|
| All | 300 | 579 µs | 2,895 µs | 29,404 µs | 5.1 µs | 19.1 µs | 264 µs |
| In Zstd-stored NUP files | 209 | 1,973 µs | 4,149 µs | 29,404 µs | 5.3 µs | 24.9 µs | 264 µs |
| In raw NUP files | 91 | 11.6 µs | 13.6 µs | 31 µs | 5.1 µs | 5.9 µs | 9.6 µs |

Slowest single NUP read: 48 ms, at 24.6 MiB in
`romfs/Sound/Resource/Stream/Dm_DemoEnding_Explosion.bwav` (29.8 MiB file).

## Settings: what changes size, time and reads

Same NUP, one setting changed at a time. Size is the NX file; time is the
whole `pack-nup` run (NUP decode included).

| Level | Threshold | Zstd chunk | NX size | vs NUP | Time | Median read | Mean read | p99 read |
|---:|---:|---|---:|---:|---:|---:|---:|---:|
| 3 | 25 % | off (tested baseline) | 9.00 GiB | +4.1 % | 233 s | 5.1 µs | 19.1 µs | 264 µs |
| 3 | 25 % | 256 KiB | 9.00 GiB | +4.1 % | 237 s | 5.3 µs | 19.4 µs | 265 µs |
| 3 | 25 % | 64 KiB | 9.36 GiB | +8.2 % | 228 s ¹ | 5.2 µs | 13.2 µs | 292 µs |
| 3 | 20 % | 64 KiB | 9.38 GiB | +8.4 % | 538 s ² | 5.4 µs | 14.3 µs | 286 µs |
| 9 | 25 % | 64 KiB | 9.36 GiB | +8.2 % | 353 s ² | 5.1 µs | 12.5 µs | 269 µs |
| 9 | 20 % | 64 KiB | 9.36 GiB | +8.2 % | 345 s ² | 5.4 µs | 14.1 µs | 276 µs |
| 13 | 25 % | 64 KiB | 9.35 GiB | +8.1 % | 281 s ¹ | 5.4 µs | 14.0 µs | 314 µs |
| 13 | 20 % | 64 KiB | 9.37 GiB | +8.3 % | 299 s | 5.3 µs | 14.1 µs | 304 µs |
| 3 | 0 % | 256 KiB | 8.62 GiB | −0.3 % | 202 s | 139 µs | 132 µs | 294 µs |

¹ Second run. The first runs of the series were slower (L3: 353 s, L13:
293 s) while the OS cache warmed up, so only same-series runs compare.
² First runs of the series (cold cache); L3 20 % also overlapped a compile.

What the numbers say:

- **Chunk size is the lever.** Re-chunking compressible files at 64 KiB made
  the archive 3.9 % larger for the same median read (p99 292 vs 264 µs);
  256 KiB and off give the same bytes on this game. On compressible Switch 1
  games 64 KiB reads 4x faster, so it stays the default (see
  [`NATIVE_VS_NX.md`](NATIVE_VS_NX.md)).
- **Level barely matters.** Level 13 saves 0.09 % and costs about 23 % more
  conversion time. Level 3 stays the default.
- **A lower threshold does not pay.** 20 % made the archive slightly larger
  here. 0 % gets below the NUP size (8.62 GiB) only by keeping Zstd on most
  blocks: the median read becomes 139 µs, still 3.8× faster than NUP.

## Layout of the real NUP files

| File | Files | Logical | Zstd-stored share | Largest Zstd files |
|---|---:|---:|---:|---|
| Donkey Kong Bananza + 3.2.0 + DLC | 159,858 | 18.02 GiB | 65.9 % of bytes (76,654 files) | 53 MiB, 34 MiB, 30 MiB |
| Orbitals Digital Deluxe | 116 | 19.45 GiB | 0 % (all raw) | none |

On a NUP whose files are all raw (Orbitals), random reads cost the same in
both formats: the NX advantage comes from per-block compression, not from
raw I/O.

## Notes and limits

- The raw-file NUP times include Python call overhead (about 5 µs); a C++
  NUP reader would match NX there. Only the Zstd case is a format difference:
  it is dominated by libzstd decoding, the same library NX uses.
- A NUP reader can avoid the cost by keeping decoded files in memory; that
  costs up to the decoded size of every file in use (53 MiB for one file
  here). NX needs at most one decoded block (≤ 1 MiB) per read.
- NX is larger than NUP because it keeps a block compressed only when that
  saves at least 25 %: most blocks stay raw, so most reads are a direct copy.
- One game with Zstd files was measured. Results depend on how much a game's
  NUP stores as Zstd and how large those files are.
