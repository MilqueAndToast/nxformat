"""Summarize how a NUP v1 file stores its files: raw or whole-file Zstd.

Usage: python nup_layout.py FILE.nup [--json OUT]
Prints the byte share per codec and the largest Zstd files (whose random
reads must decode from the start of the file).
"""
import json
import struct
import sys


def main():
    path = sys.argv[1]
    with open(path, "rb") as f:
        sb = f.read(0x80)
        assert sb[:4] == b"NUP\x01", "not a NUP v1 file"
        toc_offset, toc_size = struct.unpack_from("<QQ", sb, 0x08)
        f.seek(toc_offset)
        toc = f.read(toc_size)
    packages = struct.unpack_from("<I", toc, 4)[0]
    dirs, files = struct.unpack_from("<QQ", toc, 0x08)
    pool_offset, pool_size = struct.unpack_from("<QQ", toc, 0x18)
    pool = toc[pool_offset:pool_offset + pool_size]
    at = 0x30 + packages * 0x40 + dirs * 0x18
    rows = []
    for _ in range(files):
        name_off, name_len, size, ext_off, ext_size = struct.unpack_from("<IIQQQ", toc, at)
        codec = toc[at + 0x40]
        rows.append((pool[name_off:name_off + name_len].decode("utf-8", "replace"), size, ext_off, ext_size, codec))
        at += 0x50
    total = sum(r[1] for r in rows)
    zstd = [r for r in rows if r[4] == 1]
    zbytes = sum(r[1] for r in zstd)
    zstored = sum(r[3] for r in zstd)
    print(f"{path}: {files} files, {total / 2**30:.2f} GiB logical")
    print(f"  zstd: {len(zstd)} files, {zbytes / 2**30:.2f} GiB logical "
          f"({100 * zbytes / max(total, 1):.1f} %), stored {zstored / 2**30:.2f} GiB")
    big = sorted(zstd, key=lambda r: -r[1])[:8]
    for name, size, _, stored, _ in big:
        print(f"  {size / 2**20:10.1f} MiB -> {stored / 2**20:9.1f} MiB  {name}")
    if "--json" in sys.argv:
        out = sys.argv[sys.argv.index("--json") + 1]
        with open(out, "w") as o:
            json.dump([{"name": r[0], "size": r[1], "offset": r[2], "stored": r[3], "codec": r[4]}
                       for r in rows], o)


if __name__ == "__main__":
    main()
