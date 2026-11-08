"""Generate deterministic synthetic archives for the portable-reader benchmark."""

from __future__ import annotations

import hashlib
import json
import random
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tests" / "conformance"))
from build_fixtures import assemble, png_pixel  # noqa: E402

TOTAL_BYTES = 16 * 1024 * 1024
BLOCK_KIB = (64, 128, 256, 512, 1024)


def data_for(profile: str, block_size: int) -> bytes:
    if profile == "incompressible":
        return random.Random(0x4E58464F524D4154 ^ block_size).randbytes(TOTAL_BYTES)
    if profile == "compressible":
        chunks = []
        for block_index in range(TOTAL_BYTES // block_size):
            chunks.append(bytes(((block_index + 1) & 0xFF,)) * block_size)
        return b"".join(chunks)
    raise ValueError(profile)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: python benchmarks/generate_reader_archives.py <output-directory>", file=sys.stderr)
        return 2
    destination = Path(sys.argv[1])
    destination.mkdir(parents=True, exist_ok=True)
    rows = []
    icon = png_pixel((70, 120, 180, 255))
    for profile in ("compressible", "incompressible"):
        for block_kib in BLOCK_KIB:
            block_size = block_kib * 1024
            content = data_for(profile, block_size)
            loose_name = f"{profile}_{block_kib}k.bin"
            (destination / loose_name).write_bytes(content)
            for codec in ("raw", "zstd"):
                name = f"{profile}_{codec}_{block_kib}k.nx"
                archive = assemble(
                    title="NX synthetic benchmark",
                    publisher="NX benchmark generator",
                    localized=[{"language": "en", "title": "NX benchmark", "icon": 0}],
                    media_payloads=[(1, icon)],
                    directories=[[('payload.bin', 2, 0)]],
                    file_bytes=[content],
                    snapshots=[{"program": 0, "type": 0, "version": 0, "root": 0}],
                    chunk_log2=block_kib.bit_length() + 9,
                    compress=codec == "zstd",
                )
                path = destination / name
                path.write_bytes(archive)
                rows.append({
                    "file": name,
                    "profile": profile,
                    "codec": codec,
                    "block_kib": block_kib,
                    "logical_bytes": len(content),
                    "archive_bytes": len(archive),
                    "sha256": hashlib.sha256(archive).hexdigest(),
                    "loose_file": loose_name,
                    "loose_sha256": hashlib.sha256(content).hexdigest(),
                })
    (destination / "archive_manifest.json").write_text(
        json.dumps({"seed": "0x4e58464f524d4154", "logical_bytes": TOTAL_BYTES, "archives": rows}, sort_keys=True, indent=2) + "\n",
        encoding="utf-8",
    )
    print(f"generated {len(rows)} deterministic 16 MiB logical archives in {destination}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
