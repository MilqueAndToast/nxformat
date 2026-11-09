"""Generate the synthetic archives used by partial_read_benchmark.cpp.

64 MiB payload, about 2:1 compressible (each 4 KiB page is half random, half
a repeated pattern), every block unique. RAW and Zstandard, 64 and 256 KiB
blocks. Byte at offset i can be checked: page i // 4096 starts with its index.
"""
from __future__ import annotations

import random
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tests" / "conformance"))
from build_fixtures import assemble, png_pixel  # noqa: E402

TOTAL = 64 * 1024 * 1024


def payload() -> bytes:
    rng = random.Random(0x4E58_5041_5254)
    pages = []
    for page in range(TOTAL // 4096):
        noise = rng.randbytes(2048)
        pattern = (page.to_bytes(4, "little") + b"NXPARTIAL-BENCH!") * (2048 // 20) + b"\0" * (2048 % 20)
        pages.append(page.to_bytes(4, "little") + noise[4:] + pattern)
    return b"".join(pages)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: python benchmarks/partial_read_archives.py <output-directory>", file=sys.stderr)
        return 2
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    content = payload()
    icon = png_pixel((70, 120, 180, 255))
    for codec in ("raw", "zstd"):
        for block_kib in (64, 256):
            path = out / f"{codec}_{block_kib}k.nx"
            if path.exists():
                continue
            archive = assemble(
                title="NX partial read benchmark",
                publisher="NX benchmark generator",
                localized=[{"language": "en", "title": "NX bench", "icon": 0}],
                media_payloads=[(1, icon)],
                directories=[[("payload.bin", 2, 0)]],
                file_bytes=[content],
                snapshots=[{"program": 0, "type": 0, "version": 0, "root": 0}],
                chunk_log2=block_kib.bit_length() + 9,
                compress=codec == "zstd",
            )
            path.write_bytes(archive)
            print(f"{path.name} {len(archive)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
