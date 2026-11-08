"""Measure verified random-read misses and hits in the optional decoded LRU."""

from __future__ import annotations

import argparse
import csv
import json
import platform
import subprocess
import sys
import tempfile
from pathlib import Path

from run_reader_benchmarks import CSV_HEADER, SAMPLE_HEADER, build_benchmark

ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "benchmarks" / "results" / "reader_cache_increment4.csv",
    )
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    rows: list[list[str]] = []
    samples: list[list[str]] = []
    diagnostics: list[str] = []
    with tempfile.TemporaryDirectory(prefix="nx-reader-cache-study-") as temp_name:
        temp = Path(temp_name)
        executable = build_benchmark(temp)
        archives = temp / "archives"
        subprocess.run(
            [sys.executable, str(ROOT / "benchmarks" / "generate_reader_archives.py"), str(archives)],
            cwd=ROOT,
            check=True,
        )
        manifest = json.loads((archives / "archive_manifest.json").read_text(encoding="utf-8"))
        selected = [item for item in manifest["archives"] if item["block_kib"] == 256]
        for item in selected:
            result = subprocess.run(
                [
                    str(executable),
                    str(archives / item["file"]),
                    item["profile"],
                    item["codec"],
                    str(item["block_kib"]),
                    "cache-study",
                ],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            if result.returncode:
                raise RuntimeError(
                    f"cache study failed for {item['file']}:\n{result.stdout}\n{result.stderr}"
                )
            diagnostics.append(result.stderr.strip())
            for line in result.stdout.splitlines()[1:]:
                parts = next(csv.reader([line]))
                if parts[0] == "SAMPLE":
                    if len(parts) != len(SAMPLE_HEADER) + 1:
                        raise RuntimeError(f"unexpected raw cache sample row: {line}")
                    samples.append(parts[1:])
                else:
                    if len(parts) != len(CSV_HEADER):
                        raise RuntimeError(f"unexpected cache result row: {line}")
                    rows.append(parts)
    with args.output.open("w", encoding="utf-8", newline="") as output:
        writer = csv.writer(output, lineterminator="\n")
        writer.writerow(CSV_HEADER)
        writer.writerows(rows)
    samples_path = args.output.with_name(args.output.stem + "_samples.csv")
    with samples_path.open("w", encoding="utf-8", newline="") as output:
        writer = csv.writer(output, lineterminator="\n")
        writer.writerow(SAMPLE_HEADER)
        writer.writerows(samples)
    metadata = {
        "method": "C++17 reader; deterministic 16 MiB synthetic payload; read samples use FileId and CRC-32C slice verification; the source file is pre-read into the OS page cache before measurement",
        "reader_cache": "disabled has no decoded LRU; cold_miss constructs and mounts a new Archive object per sample so its verified decoded-block LRU is empty; warm_hit performs an untimed priming read on the same Archive then times an immediate repeat",
        "cache_capacity": {"payload_bytes": 8 * 1024 * 1024, "entries": 128},
        "cold_storage": "not measured; operating-system file cache is warm for every case",
        "profiles": ["compressible", "incompressible"],
        "codecs": ["raw", "zstd"],
        "block_kib": [256],
        "read_sizes_kib": [4, 64],
        "providers": ["runtime-selected CRC-32C"],
        "legacy_csv_fields": "verify_read_sha256 means verify_read_blocks (CRC-32C); sha_provider is crc32c; SHA callback timing columns are zero",
        "measurements": ["p50/p95/p99 latency", "Data-region bytes per request", "allocations per request", "process peak working set", "Zstandard callback time", "legacy SHA callback time (zero on current block reads)"],
        "compiler": __import__("os").environ.get("CXX") or "auto-detected C++17 compiler",
        "os": platform.platform(),
        "cpu": platform.processor() or platform.machine(),
        "result_rows": len(rows),
        "raw_sample_rows": len(samples),
        "diagnostics": diagnostics,
        "results_csv": str(args.output),
        "raw_samples_csv": str(samples_path),
    }
    metadata_path = args.output.with_suffix(".metadata.json")
    metadata_path.write_text(json.dumps(metadata, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {len(rows)} measurements and {len(samples)} raw samples")
    print(f"results: {args.output}")
    print(f"samples: {samples_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
