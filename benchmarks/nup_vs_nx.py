"""Same random reads in a NUP and in the NX converted from it.

Usage: python nup_vs_nx.py GAME.nup GAME.nx nx_read_list.exe [--reads N] [--size BYTES]

Reads are drawn from the base package's files, weighted by file size (as a
game reads its RomFS), with a fixed seed. NUP stores each file as one raw or
one Zstandard extent, so a read inside a Zstd file decodes from the start of
the file up to the requested bytes: the fastest a NUP reader can do without
keeping decoded files in memory. NX reads go through the shipped C++ reader.
Both files are read twice; the second (warm OS cache) pass is reported.
"""
import random
import statistics
import struct
import subprocess
import sys
import tempfile
import time
from compression import zstd
from pathlib import Path


def parse(path):
    with open(path, "rb") as f:
        sb = f.read(0x80)
        assert sb[:4] == b"NUP\x01"
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
        tid, root, exefs, romfs, version = struct.unpack_from("<QIIII", toc, at)
        packages.append(dict(title=tid, root=root, exefs=exefs, romfs=romfs, version=version, kind=toc[at + 0x18]))
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
    return packages, dirs, files, children, toc_size


def walk(dirs, files, children, node, prefix, out):
    _, first, count = dirs[node]
    for child in children[first:first + count]:
        if child < len(dirs):
            walk(dirs, files, children, child, f"{prefix}/{dirs[child][0]}", out)
        else:
            f = files[child - len(dirs)]
            out.append((f"{prefix}/{f['name']}", f))


def nup_read(handle, f, offset, length):
    if f["codec"] == 0:
        handle.seek(f["offset"] + offset)
        return handle.read(length)
    handle.seek(f["offset"])
    left = f["stored"]
    decoder = zstd.ZstdDecompressor()
    produced = bytearray()
    while len(produced) < offset + length and left:
        chunk = handle.read(min(left, 1 << 20))
        left -= len(chunk)
        produced += decoder.decompress(chunk)
    return bytes(produced[offset:offset + length])


def stats(ns):
    ns = sorted(ns)
    return dict(median=statistics.median(ns) / 1e3, p99=ns[int(len(ns) * 0.99) - 1] / 1e3, mean=statistics.fmean(ns) / 1e3)


def main():
    nup, nx, tool = sys.argv[1:4]
    count = int(sys.argv[sys.argv.index("--reads") + 1]) if "--reads" in sys.argv else 400
    size = int(sys.argv[sys.argv.index("--size") + 1]) if "--size" in sys.argv else 4096
    packages, dirs, files, children, toc_size = parse(nup)
    base = packages[0]
    tree = []
    for node, name in ((base["exefs"], "exefs"), (base["romfs"], "romfs")):
        if node != 0xFFFFFFFF:
            walk(dirs, files, children, node, name, tree)
    tree = [(p, f) for p, f in tree if f["size"] >= size]
    rng = random.Random(0x4E58)
    weights = [f["size"] for _, f in tree]
    picks = rng.choices(tree, weights=weights, k=count)
    reads = [(p, f, rng.randrange(0, f["size"] - size + 1)) for p, f in picks]

    # NUP: open (superblock + TOC + its SHA-256), then the reads.
    t = time.perf_counter_ns()
    parse(nup)
    open_ns = time.perf_counter_ns() - t
    nup_ns = []
    with open(nup, "rb") as h:
        for warm in (False, True):
            nup_ns.clear()
            for _, f, off in reads:
                t = time.perf_counter_ns()
                nup_read(h, f, off, size)
                nup_ns.append(time.perf_counter_ns() - t)
    with tempfile.TemporaryDirectory() as tmp:
        listing = Path(tmp) / "reads.tsv"
        listing.write_text("".join(f"{p}\t{off}\t{size}\n" for p, _, off in reads))
        result = Path(tmp) / "out.tsv"
        for _ in range(2):
            run = subprocess.run([tool, nx, "0", str(listing), str(result)], capture_output=True, text=True)
        if run.returncode:
            sys.exit("NX tool failed:\n" + run.stdout + run.stderr)
        nx_open = int(next(l.split("\t")[1] for l in run.stdout.splitlines() if l.startswith("open_mount_ns")))
        nx_ns = [int(l.split("\t")[3]) for l in result.read_text().splitlines()]

    def report(label, picked):
        a = stats([nup_ns[i] for i in picked])
        b = stats([nx_ns[i] for i in picked])
        print(f"{label:<28} n={len(picked):4}  NUP median {a['median']:10.1f} us p99 {a['p99']:10.1f} mean {a['mean']:10.1f}"
              f" | NX median {b['median']:7.1f} us p99 {b['p99']:7.1f} mean {b['mean']:7.1f}"
              f" | x{a['median'] / b['median']:.1f} median, x{a['mean'] / b['mean']:.1f} mean")
    print(f"{Path(nup).name}: {count} random reads of {size} bytes in the base files")
    report("all reads", range(len(reads)))
    report("in Zstd-stored NUP files", [i for i, r in enumerate(reads) if r[1]["codec"] == 1])
    report("in raw NUP files", [i for i, r in enumerate(reads) if r[1]["codec"] == 0])
    worst = max(range(len(reads)), key=lambda i: nup_ns[i])
    print(f"slowest NUP read: {nup_ns[worst] / 1e3:.0f} us at {reads[worst][2] / 2**20:.1f} MiB in {reads[worst][0]}"
          f" ({reads[worst][1]['size'] / 2**20:.1f} MiB file); NX {nx_ns[worst] / 1e3:.1f} us")
    print(f"open: NUP TOC {toc_size / 2**20:.1f} MiB parse {open_ns / 1e6:.1f} ms (Python) | NX open+mount {nx_open / 1e6:.1f} ms (C++)")


if __name__ == "__main__":
    main()
