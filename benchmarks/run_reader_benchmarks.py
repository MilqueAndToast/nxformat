"""Build and run the complete synthetic C++ reader benchmark matrix."""

from __future__ import annotations

import argparse
import csv
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from compression import zstd

ROOT = Path(__file__).resolve().parents[1]
# Historical column names are retained for CSV consumers. Current reads use
# CRC-32C; sha_provider is "none" or "crc32c", and SHA timing columns are zero.
CSV_HEADER = (
    "profile", "codec", "block_kib", "verify_read_sha256", "sha_provider", "read_mode", "operation", "samples",
    "p50_us", "p95_us", "p99_us", "throughput_mib_s", "allocations_per_sample",
    "peak_working_set_bytes", "archive_bytes", "cache_state", "data_region_bytes_per_sample",
    "decode_p50_us", "sha_p50_us",
)
SAMPLE_HEADER = (
    "profile", "codec", "block_kib", "verify_read_sha256", "sha_provider", "read_mode", "operation",
    "sample_index", "offset_bytes", "bytes", "latency_us", "allocations", "data_region_bytes",
    "decode_us", "sha_us",
)


def build_benchmark(temp: Path) -> Path:
    source = ROOT / "benchmarks" / "reader_benchmark.cpp"
    compiler_name = os.environ.get("CXX")
    compiler = shutil.which(compiler_name) if compiler_name else (shutil.which("c++") or shutil.which("g++") or shutil.which("cl"))
    if not compiler:
        raise RuntimeError("no C++17 compiler found")
    executable = temp / ("reader_benchmark.exe" if Path(compiler).name.lower() in {"cl", "cl.exe"} else "reader_benchmark")
    if executable.suffix == ".exe":
        command = [compiler, "/nologo", "/std:c++17", "/EHsc", "/W4", "/WX", "/O2",
                   f"/I{ROOT / 'dist'}", f"/I{ROOT / 'tests' / 'support'}", str(source),
                   f"/Fo:{temp / 'reader_benchmark.obj'}", f"/Fe:{executable}"]
    else:
        dynamic_library = [] if sys.platform in ("darwin", "win32") else ["-ldl"]
        command = [compiler, "-std=c++17", "-O2", "-Wall", "-Wextra", "-Werror", "-pedantic",
                   f"-I{ROOT / 'dist'}", f"-I{ROOT / 'tests' / 'support'}", str(source),
                   "-pthread", *dynamic_library, "-o", str(executable)]
    built = subprocess.run(command, cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    if built.returncode:
        raise RuntimeError("benchmark compile failed:\n" + built.stdout)
    return executable


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, default=ROOT / "benchmarks" / "results" / "reader_warm_cache.csv")
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    all_rows = []
    all_samples = []
    diagnostics = ""
    with tempfile.TemporaryDirectory(prefix="nx-reader-bench-") as temp_name:
        temp = Path(temp_name)
        executable = build_benchmark(temp)
        archives = temp / "archives"
        subprocess.run([sys.executable, str(ROOT / "benchmarks" / "generate_reader_archives.py"), str(archives)],
                       cwd=ROOT, check=True)
        manifest = json.loads((archives / "archive_manifest.json").read_text(encoding="utf-8"))
        for item in manifest["archives"]:
            result = subprocess.run(
                [str(executable), str(archives / item["file"]), item["profile"], item["codec"], str(item["block_kib"])],
                cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            )
            if result.returncode:
                raise RuntimeError(f"benchmark failed for {item['file']} with exit {result.returncode}:\n{result.stdout}\n{result.stderr}")
            if not diagnostics:
                diagnostics = result.stderr.strip()
            for line in result.stdout.splitlines():
                parts = next(csv.reader([line]))
                if parts[0] == "SAMPLE":
                    if len(parts) != len(SAMPLE_HEADER) + 1:
                        raise RuntimeError(f"unexpected raw benchmark sample row: {line}")
                    all_samples.append(parts[1:])
                    continue
                if len(parts) != len(CSV_HEADER):
                    raise RuntimeError(f"unexpected benchmark result row: {line}")
                all_rows.append(parts)
        with args.output.open("w", encoding="utf-8", newline="") as output:
            writer = csv.writer(output, lineterminator="\n")
            writer.writerow(CSV_HEADER)
            writer.writerows(all_rows)
        samples_path = args.output.with_name(args.output.stem + "_samples.csv")
        with samples_path.open("w", encoding="utf-8", newline="") as output:
            writer = csv.writer(output, lineterminator="\n")
            writer.writerow(SAMPLE_HEADER)
            writer.writerows(all_samples)
    metadata = {
        "method": "C++17 reader; deterministic 16 MiB synthetic file; file source pre-read once per process; warm OS file cache only; Data bytes count requested source ranges intersecting the Data region; decode times are callback wall times; legacy SHA callback timings are zero on current block reads",
        "cold_cache": "not measured; reliable system cache clearing was unavailable",
        "storage": "not classified; results are not labeled as SSD performance",
        "profiles": ["compressible", "incompressible"],
        "codecs": ["raw", "zstd"],
        "block_kib": [64, 128, 256, 512, 1024],
        "random_reads": {"4 KiB": 200, "64 KiB": 140, "1 MiB": 100},
        "random_offset_seed": "0x4e58524541444552 + read_size_bytes + block_kib",
        "sequential_reads": 192,
        "sequential_trace": "12 passes over the 16 MiB file, each as sixteen consecutive 1 MiB requests",
        "zstd_encoder": "Python 3.14 compression.zstd default compression level",
        "open_samples": 60,
        "mount_samples": 30,
        "icon_samples": 40,
        "verification": ["disabled", "CRC-32C slices (runtime-selected implementation)"],
        "legacy_csv_fields": "verify_read_sha256 means verify_read_blocks (CRC-32C); sha_provider reports none/crc32c; SHA callback timing columns are zero on block reads",
        "measurements": ["p50/p95/p99 latency", "throughput", "C++ allocations per sample", "process peak working set", "Data-region bytes read per sample", "p50 time inside Zstandard callback; legacy SHA timing is zero on block reads"],
        "compiler": os.environ.get("CXX") or (shutil.which("c++") or shutil.which("g++") or shutil.which("cl")),
        "os": platform.platform(),
        "cpu": platform.processor() or platform.machine(),
        "python": platform.python_version(),
        "python_zstandard": list(zstd.zstd_version_info),
        "benchmark_diagnostic": diagnostics,
        "archive_manifest_sha256": __import__("hashlib").sha256(json.dumps(manifest, sort_keys=True).encode()).hexdigest(),
        "results_csv": str(args.output),
        "result_rows": len(all_rows),
        "raw_sample_rows": len(all_samples),
        "raw_samples_csv": str(samples_path),
    }
    metadata_path = args.output.with_suffix(".metadata.json")
    metadata_path.write_text(json.dumps(metadata, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {len(all_rows)} measurements to {args.output}")
    print(f"wrote {len(all_samples)} raw samples and read offsets to {samples_path}")
    print(f"metadata: {metadata_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
