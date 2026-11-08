"""Random reads in a decrypted package (DNSP or DXCI) and in the NX made from it.

Usage: python native_vs_nx.py GAME.dnsp|dxci GAME.nx nx_read_list.exe [--reads N] [--size BYTES]

The package members (the PFS0 of a DNSP, the secure partition of a DXCI) are
listed from the package header. Reads are drawn with a fixed seed, weighted by
member size, at a uniform offset inside the member. Each read is timed in the
NX (snapshot 0, shipped C++ reader, second pass) and at the same bytes of the
package file (Python, includes call overhead). The package must hold one
snapshot only (a base without update or DLC), so snapshot 0 has every member.
"""
import argparse
import random
import struct
import subprocess
import tempfile
import time
from pathlib import Path


def pfs0(handle, base):
    handle.seek(base)
    magic, count, strings, _ = struct.unpack("<4sIII", handle.read(16))
    if magic != b"PFS0":
        raise SystemExit("not a PFS0 package")
    entries = [struct.unpack("<QQII", handle.read(0x18)) for _ in range(count)]
    table = handle.read(strings)
    data = base + 16 + count * 0x18 + strings
    return [(table[n:table.index(b"\0", n)].decode(), data + o, s) for o, s, n, _ in entries]


def hfs0(handle, base):
    handle.seek(base)
    magic, count, strings, _ = struct.unpack("<4sIII", handle.read(16))
    if magic != b"HFS0":
        raise SystemExit("not an HFS0 partition")
    entries = [struct.unpack("<QQIIQ32s", handle.read(0x40)) for _ in range(count)]
    table = handle.read(strings)
    data = base + 16 + count * 0x40 + strings
    return [(table[e[2]:table.index(b"\0", e[2])].decode(), data + e[0], e[1]) for e in entries]


def members(path):
    with open(path, "rb") as handle:
        if handle.read(4) == b"PFS0":
            return pfs0(handle, 0)
        for start in (0, 0x1000):
            handle.seek(start + 0x100)
            if handle.read(4) in (b"HEAD", b"DXCI"):
                handle.seek(start + 0x130)
                root = struct.unpack("<Q", handle.read(8))[0] + start
                for name, offset, _ in hfs0(handle, root):
                    if name == "secure":
                        return hfs0(handle, offset)
        raise SystemExit("not a DNSP or DXCI")


def stats(ns):
    ns = sorted(ns)
    us = [n / 1000 for n in ns]
    return us[len(us) // 2], sum(us) / len(us), us[min(len(us) - 1, int(len(us) * 0.99))]


def main():
    p = argparse.ArgumentParser()
    p.add_argument("package")
    p.add_argument("nx")
    p.add_argument("tool")
    p.add_argument("--reads", type=int, default=300)
    p.add_argument("--size", type=int, default=4096)
    a = p.parse_args()
    files = [m for m in members(a.package) if m[2] >= a.size]
    rng = random.Random(1234)
    picks = []
    for _ in range(a.reads):
        name, base, size = rng.choices(files, weights=[f[2] for f in files])[0]
        picks.append((name, base, rng.randrange(0, size - a.size + 1)))
    with tempfile.TemporaryDirectory() as tmp:
        listing = Path(tmp) / "reads.tsv"
        result = Path(tmp) / "out.tsv"
        listing.write_text("".join(f"{n}\t{o}\t{a.size}\n" for n, _, o in picks))
        for _ in range(2):  # second pass: warm cache, as for the package
            run = subprocess.run([a.tool, a.nx, "0", str(listing), str(result)], capture_output=True, text=True)
            if run.returncode != 0:
                raise SystemExit(run.stdout + run.stderr)
        nx_ns = [int(line.split("\t")[3]) for line in result.read_text().splitlines()]
        open_ms = int(run.stdout.split("open_mount_ns\t")[1].split()[0]) / 1e6
    raw_ns = []
    with open(a.package, "rb", buffering=0) as handle:
        for _ in range(2):
            raw_ns = []
            for _, base, offset in picks:
                started = time.perf_counter_ns()
                handle.seek(base + offset)
                handle.read(a.size)
                raw_ns.append(time.perf_counter_ns() - started)
    r, n = stats(raw_ns), stats(nx_ns)
    print(f"{Path(a.package).name}: {a.reads} random reads of {a.size} bytes, {len(files)} members")
    print(f"package median {r[0]:9.1f} us mean {r[1]:9.1f} p99 {r[2]:9.1f} | "
          f"NX median {n[0]:7.1f} us mean {n[1]:7.1f} p99 {n[2]:7.1f} | NX open+mount {open_ms:.1f} ms")


if __name__ == "__main__":
    main()
