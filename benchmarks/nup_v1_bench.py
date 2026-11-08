"""Reproducible NX vs NUP v1 benchmark on synthetic data (no Nintendo content).

Usage:
  python nup_v1_bench.py WORK_DIR --nup NUP_EXE --nxconvert NXCONVERT_EXE
         --reader NX_READ_LIST_EXE --seed SEED.nup [--scale 1.0] [--reads 400]

Inputs:
- NUP_EXE: the reference `nup` CLI (a55uka/nup, tools/nup, `pip install .`).
- SEED.nup: a synthetic NUP whose ExeFS passes NXFormat's NPDM/NSO checks,
  written by `NX_NUP_DUMP=SEED.nup cargo test --release --lib dump_synthetic_nup`.
  Only its exefs/ trees are reused; all RomFS data is generated here.
- NX_READ_LIST_EXE: benchmarks/nx_read_list.cpp built with the shipped reader.

Steps, all timed, peak working set sampled with psutil:
1. dump/: base (exefs + generated RomFS) and updates/65536 (10 % of the base
   files changed, 20 added). Data is seeded: same scale, same bytes.
2. `nup pack` (full and base-only) and `nxconvert pack-nup` of both.
3. Random reads of 4 KiB, 64 KiB and 256 KiB in the base files, weighted by
   file size, same offsets on both sides. NUP is read two ways:
   - stream: decode the one Zstd frame from its start up to the bytes asked
     (no cache, what a reader without memory or temp files can do);
   - whole: decode the whole file on first touch and keep it (what Eglantine
     and Lichii do; its memory is reported).
   NX goes through the shipped C++ reader (nx_read_list). NUP is read from
   Python, so its per-read overhead is a few us higher than C++ would be.
4. Update: NX `add-tree` into the base NX in place, `remove` it, `compact`;
   NUP v1 has no in-place add, so the update costs a full `nup pack`, or a
   separate file with `nup pack --split`.
Results are printed as one table and written to WORK_DIR/results.json.
"""
from __future__ import annotations

import argparse
import json
import os
import random
import shutil
import statistics
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import psutil
import zstandard

MIB = 1 << 20


def run(cmd: list[str], cwd: Path | None = None) -> dict:
    """Run cmd; return wall time, peak working set and output.

    The peak is the largest peak working set seen in the process or any of
    its children (`nup.exe` is a launcher; the work runs in its Python
    child), sampled every 20 ms: a process shorter than that may be missed.
    """
    start = time.perf_counter()
    env = dict(os.environ, PYTHONIOENCODING="utf-8")
    proc = psutil.Popen(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env)
    peak = 0
    out = []
    import threading

    reader = threading.Thread(target=lambda: out.append(proc.stdout.read()))
    reader.start()
    while proc.poll() is None:
        try:
            for p in [proc, *proc.children(recursive=True)]:
                try:
                    info = p.memory_info()
                    peak = max(peak, getattr(info, "peak_wset", info.rss))
                except psutil.Error:
                    pass
        except psutil.Error:
            pass
        time.sleep(0.02)
    reader.join()
    text = out[0].decode("utf-8", "replace") if out else ""
    if proc.returncode:
        sys.exit(f"failed ({proc.returncode}): {' '.join(cmd)}\n{text}")
    return {"seconds": time.perf_counter() - start, "peak_mib": peak / MIB, "output": text}


def payload(seed: int, size: int, compressible: bool) -> bytes:
    rng = random.Random(seed)
    data = rng.randbytes(size)
    if compressible:
        # 16 symbols: about 2:1 with Zstd level 3, like packed game assets.
        data = data.translate(bytes(b % 16 + 0x41 for b in range(256)))
    return data


def make_dump(work: Path, seed_nup: Path, nup: str, scale: float) -> tuple[Path, Path]:
    seed_tree = work / "seed"
    for kind in ("base", "update"):
        run([nup, "extract", "--package", kind, str(seed_nup), str(seed_tree / kind)])
    dump = work / "dump"
    base = dump / "base"
    update = dump / "updates" / "65536"
    shutil.copytree(seed_tree / "base" / "exefs", base / "exefs")
    shutil.copytree(seed_tree / "update" / "exefs", update / "exefs")
    files = []
    for i in range(int(24 * scale)):
        files.append((f"data/pack{i:02}.bin", 8 * MIB, True))
    for i in range(int(16 * scale)):
        files.append((f"stream/audio{i:02}.bin", 8 * MIB, False))
    for i in range(max(1, int(2 * scale))):
        files.append((f"large/world{i}.bin", 48 * MIB, True))
    rng = random.Random(7)
    for i in range(int(2000 * scale)):
        files.append((f"small/{i // 100:02}/f{i:04}.bin", rng.randrange(4096, 65536), i % 3 != 0))
    for n, (path, size, comp) in enumerate(files):
        target = base / "romfs" / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(payload(n, size, comp))
    for n, (path, size, comp) in enumerate(files):
        if n % 10 == 0:
            target = update / "romfs" / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(payload(100_000 + n, size, comp))
    for i in range(20):
        target = update / "romfs" / "added" / f"new{i:02}.bin"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(payload(200_000 + i, 256 * 1024, True))
    base_only = work / "dump_base" / "base"
    shutil.copytree(base, base_only)
    return dump, base_only.parent


def parse_nup(path: Path):
    with open(path, "rb") as f:
        sb = f.read(0x80)
        toc_offset, toc_size = struct.unpack_from("<QQ", sb, 0x08)
        f.seek(toc_offset)
        toc = f.read(toc_size)
    npk = struct.unpack_from("<I", toc, 4)[0]
    ndir, nfile = struct.unpack_from("<QQ", toc, 0x08)
    pool_off, pool_size = struct.unpack_from("<QQ", toc, 0x18)
    pool = toc[pool_off:pool_off + pool_size]
    at = 0x30
    packages = []
    for _ in range(npk):
        _tid, _root, exefs, romfs = struct.unpack_from("<QIII", toc, at)
        packages.append((exefs, romfs, toc[at + 0x18]))
        at += 0x40
    dirs = []
    for _ in range(ndir):
        no, nl, first, count = struct.unpack_from("<IIII", toc, at)
        dirs.append((pool[no:no + nl].decode(), first, count))
        at += 0x18
    files = []
    for _ in range(nfile):
        no, nl, size, eo, es = struct.unpack_from("<IIQQQ", toc, at)
        files.append(dict(name=pool[no:no + nl].decode(), size=size, offset=eo, stored=es, codec=toc[at + 0x40]))
        at += 0x50
    children = struct.unpack_from(f"<{(pool_off - at) // 4}I", toc, at)
    base = next(p for p in packages if p[2] == 0)
    out: list[tuple[str, dict]] = []

    def walk(node: int, prefix: str) -> None:
        _, first, count = dirs[node]
        for child in children[first:first + count]:
            if child < len(dirs):
                walk(child, f"{prefix}/{dirs[child][0]}")
            else:
                f = files[child - len(dirs)]
                out.append((f"{prefix}/{f['name']}", f))

    walk(base[1], "romfs")
    return out


def pct(values: list[float], q: float) -> float:
    values = sorted(values)
    return values[min(len(values) - 1, int(round(q * (len(values) - 1))))]


def summary(ns: list[int]) -> dict:
    us = [n / 1e3 for n in ns]
    return {"p50_us": pct(us, 0.50), "p95_us": pct(us, 0.95), "p99_us": pct(us, 0.99), "mean_us": statistics.fmean(us)}


def read_bench(nup_path: Path, nx_path: Path, reader: str, size: int, count: int) -> dict:
    tree = [(p, f) for p, f in parse_nup(nup_path) if f["size"] >= size]
    rng = random.Random(0x4E58 + size)
    picks = rng.choices(tree, weights=[f["size"] for _, f in tree], k=count)
    reads = [(p, f, rng.randrange(0, f["size"] - size + 1)) for p, f in picks]
    result = {}
    with open(nup_path, "rb") as h:
        stream_ns: list[int] = []
        for _ in range(2):  # second pass: warm OS cache
            stream_ns = []
            for _, f, off in reads:
                t = time.perf_counter_ns()
                if f["codec"] == 0:
                    h.seek(f["offset"] + off)
                    h.read(size)
                else:
                    h.seek(f["offset"])
                    d = zstandard.ZstdDecompressor().decompressobj()
                    produced = bytearray()
                    left = f["stored"]
                    while len(produced) < off + size and left:
                        chunk = h.read(min(left, MIB))
                        left -= len(chunk)
                        produced += d.decompress(chunk)
                    bytes(produced[off:off + size])
                stream_ns.append(time.perf_counter_ns() - t)
        cache: dict[int, bytes] = {}
        whole_ns = []
        for _, f, off in reads:
            t = time.perf_counter_ns()
            if f["codec"] == 0:
                h.seek(f["offset"] + off)
                h.read(size)
            else:
                data = cache.get(f["offset"])
                if data is None:
                    h.seek(f["offset"])
                    data = zstandard.ZstdDecompressor().decompress(h.read(f["stored"]), max_output_size=f["size"])
                    cache[f["offset"]] = data
                data[off:off + size]
            whole_ns.append(time.perf_counter_ns() - t)
    result["nup_stream"] = summary(stream_ns)
    result["nup_whole_file_cache"] = summary(whole_ns) | {"cache_mib": sum(map(len, cache.values())) / MIB}
    whole_mib = result["nup_whole_file_cache"]["cache_mib"]
    nx_ns, peak = nx_reads(reader, nx_path, [(p, off, size) for p, _, off in reads])
    result["nx"] = summary(nx_ns) | {"reader_peak_mib": peak}
    # Same memory as the NUP whole-file cache, spent on verified 64 KiB blocks.
    budget = max(1, int(whole_mib + 0.5))
    nx_ns, peak = nx_reads(reader, nx_path, [(p, off, size) for p, _, off in reads], budget)
    result["nx_block_cache"] = summary(nx_ns) | {"cache_mib": budget, "reader_peak_mib": peak}
    result["zstd_share_of_reads"] = sum(f["codec"] == 1 for _, f, _ in reads) / len(reads)
    return result


def nx_reads(reader: str, nx_path: Path, reads: list[tuple[str, int, int]], cache_mib: int | None = None,
             passes: int = 2) -> tuple[list[int], float]:
    """Time each (path, offset, length) read with nx_read_list; last pass reported."""
    with tempfile.TemporaryDirectory() as tmp:
        listing = Path(tmp) / "reads.tsv"
        listing.write_text("".join(f"{p}\t{off}\t{n}\n" for p, off, n in reads))
        out = Path(tmp) / "out.tsv"
        extra = [] if cache_mib is None else [str(cache_mib)]
        for _ in range(passes):
            r = run([reader, str(nx_path), "0", str(listing), str(out), *extra])
        return [int(line.split("\t")[3]) for line in out.read_text().splitlines()], r["peak_mib"]


def asset_bench(nup_path: Path, nx_path: Path, reader: str, count: int, window: int = MIB,
                request: int = 64 * 1024) -> dict:
    """Asset loads: `window` bytes read in order, in `request` pieces, from a
    random offset of a file at least `window` long (a level pack, a sound
    stream). The time of one load is the sum of its pieces. Each load is
    the first touch of its range: caches start cold for every pattern."""
    tree = [(p, f) for p, f in parse_nup(nup_path) if f["size"] >= window]
    rng = random.Random(0xA55E7)
    loads = []
    for _ in range(count):
        p, f = rng.choice(tree)
        loads.append((p, f, rng.randrange(0, f["size"] - window + 1) // request * request))
    result = {}
    with open(nup_path, "rb") as h:
        stream_ns, whole_ns = [], []
        for _, f, off in loads:
            # Best a NUP v1 reader can do without memory: one decoder from the
            # start of the frame to the end of the window.
            t = time.perf_counter_ns()
            if f["codec"] == 0:
                for at in range(off, off + window, request):
                    h.seek(f["offset"] + at)
                    h.read(request)
            else:
                h.seek(f["offset"])
                d = zstandard.ZstdDecompressor().decompressobj()
                produced = bytearray()
                left = f["stored"]
                while len(produced) < off + window and left:
                    chunk = h.read(min(left, MIB))
                    left -= len(chunk)
                    produced += d.decompress(chunk)
                bytes(produced[off:off + window])
            stream_ns.append(time.perf_counter_ns() - t)
            # Whole-file reader, cold: decode the file, then copy the window.
            t = time.perf_counter_ns()
            if f["codec"] == 0:
                for at in range(off, off + window, request):
                    h.seek(f["offset"] + at)
                    h.read(request)
            else:
                h.seek(f["offset"])
                data = zstandard.ZstdDecompressor().decompress(h.read(f["stored"]), max_output_size=f["size"])
                for at in range(off, off + window, request):
                    data[at:at + request]
            whole_ns.append(time.perf_counter_ns() - t)
    pieces = [(p, at, request) for p, _, off in loads for at in range(off, off + window, request)]
    per = window // request
    nx_ns, peak = nx_reads(reader, nx_path, pieces, passes=2)
    nx_loads = [sum(nx_ns[i * per:(i + 1) * per]) for i in range(len(loads))]
    # With a small cache: a 64 KiB request inside a 256 KiB block decodes it once.
    cached_ns, cached_peak = nx_reads(reader, nx_path, pieces, 8, passes=1)
    cached_loads = [sum(cached_ns[i * per:(i + 1) * per]) for i in range(len(loads))]
    result["nup_stream"] = summary(stream_ns)
    result["nup_whole_file_cold"] = summary(whole_ns)
    result["nx"] = summary(nx_loads) | {"reader_peak_mib": peak}
    result["nx_8mib_block_cache"] = summary(cached_loads) | {"reader_peak_mib": cached_peak}
    result["zstd_share_of_loads"] = sum(f["codec"] == 1 for _, f, _ in loads) / len(loads)
    return result


def snapshot_ids(nxconvert: str, archive: Path) -> list[str]:
    out = run([nxconvert, "contents", str(archive)])["output"]
    return [line.split("\t") for line in out.splitlines() if line.startswith("@SNAPSHOT")]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("work", type=Path)
    ap.add_argument("--nup", required=True)
    ap.add_argument("--nxconvert", required=True)
    ap.add_argument("--reader", required=True)
    ap.add_argument("--seed", required=True, type=Path)
    ap.add_argument("--scale", type=float, default=1.0)
    ap.add_argument("--reads", type=int, default=400)
    a = ap.parse_args()
    work = a.work
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    dump, dump_base = make_dump(work, a.seed, a.nup, a.scale)
    logical = sum(p.stat().st_size for p in (dump / "base" / "romfs").rglob("*") if p.is_file())
    res: dict = {"scale": a.scale, "base_romfs_mib": logical / MIB}
    meta = ["--name", "Synthetic Game", "--publisher", "Synthetic Publisher", "--display-version", "1.0.0"]
    full_nup, base_nup = work / "game.nup", work / "base.nup"
    res["nup_pack_full"] = run([a.nup, "pack", "-q", *meta, str(dump), str(full_nup)])
    res["nup_pack_base"] = run([a.nup, "pack", "-q", *meta, str(dump_base), str(base_nup)])
    split = work / "split"
    split.mkdir()
    res["nup_pack_split"] = run([a.nup, "pack", "-q", "--split", *meta, str(dump), str(split / "game.nup")])
    full_nx, base_nx = work / "game.nx", work / "base.nx"
    res["nx_pack_nup_full"] = run([a.nxconvert, "pack-nup", str(full_nup), str(full_nx), "--platform", "switch2", "--no-trail"])
    res["nx_pack_nup_base"] = run([a.nxconvert, "pack-nup", str(base_nup), str(base_nx), "--platform", "switch2", "--no-trail"])
    res["nx_info_full"] = run([a.nxconvert, "info", str(full_nx)])["output"]
    res["sizes_mib"] = {str(p.relative_to(work)): p.stat().st_size / MIB for p in [full_nup, base_nup, full_nx, base_nx, *split.iterdir()]}
    res["reads"] = {str(size): read_bench(full_nup, full_nx, a.reader, size, a.reads) for size in (4096, 65536, 262144)}
    res["asset_loads_1mib"] = asset_bench(full_nup, full_nx, a.reader, max(20, a.reads // 4))
    # Update in place on the NX side.
    edit = work / "edit.nx"
    shutil.copy(base_nx, edit)
    res["nx_add_update"] = run([a.nxconvert, "add-tree", str(edit), str(dump / "updates" / "65536"),
                                "--platform", "switch2", "--update-version", "65536", "--no-trail"])
    res["nx_after_add_mib"] = edit.stat().st_size / MIB
    updates = [row for row in snapshot_ids(a.nxconvert, edit) if "update" in "\t".join(row).lower()]
    res["nx_snapshots_after_add"] = updates
    if updates:
        identity = next(field for field in updates[0] if len(field) == 64)
        res["nx_remove_update"] = run([a.nxconvert, "remove", str(edit), identity])
        res["nx_after_remove_mib"] = edit.stat().st_size / MIB
        res["nx_compact"] = run([a.nxconvert, "compact", str(edit)])
        res["nx_after_compact_mib"] = edit.stat().st_size / MIB
    for value in res.values():
        if isinstance(value, dict) and "output" in value and len(value["output"]) > 4000:
            value["output"] = value["output"][-4000:]
    (work / "results.json").write_text(json.dumps(res, indent=2), encoding="utf-8")
    print(json.dumps({k: v for k, v in res.items() if k not in ("nx_info_full",)}, indent=1, default=str)[:20000])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
