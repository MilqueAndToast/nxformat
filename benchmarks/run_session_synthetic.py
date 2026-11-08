"""Run repeatable p99/p99.9 traces against the production NX persistent reader.

Requires a C++17 compiler and Python 3.14 for the existing archive generator.
Only synthetic, redistributable data is used.
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import io
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CASES = (
    ("uniform4k", 0, 0, 1),
    ("uniform4k", 8, 0, 1),
    ("uniform4k", 64, 0, 1),
    ("hot4k", 8, 0, 1),
    ("hot4k", 64, 0, 1),
    ("cross4k", 8, 0, 1),
    ("sequential64k", 8, 0, 1),
    ("sequential64k", 8, 1, 1),
    ("sequential1m", 8, 1, 1),
    ("mixed", 8, 0, 4),
    ("mixed", 8, 1, 4),
)


def compile_benchmark(build: Path) -> Path:
    compiler_env = os.environ.get("CXX")
    compiler = shutil.which(compiler_env) if compiler_env else (
        shutil.which("c++") or shutil.which("g++") or shutil.which("cl")
    )
    if not compiler:
        raise RuntimeError("no C++17 compiler available")
    msvc = Path(compiler).name.lower() in {"cl", "cl.exe"}
    executable = build / ("session_trace_benchmark.exe" if msvc else "session_trace_benchmark")
    source = ROOT / "benchmarks" / "session_trace_benchmark.cpp"
    if msvc:
        command = [
            compiler, "/nologo", "/std:c++17", "/EHsc", "/W4", "/WX", "/O2",
            f"/I{ROOT / 'dist'}", f"/I{ROOT / 'tests' / 'support'}",
            str(source), f"/Fo:{build / 'session_trace_benchmark.obj'}",
            f"/Fe:{executable}",
        ]
    else:
        crypto = []
        if sys.platform != "darwin" and shutil.which("pkg-config"):
            import shlex
            found = subprocess.run(["pkg-config", "--cflags", "--libs", "libcrypto"], capture_output=True, text=True)
            if not found.returncode:
                crypto = ["-DNX_USE_OPENSSL=1", *shlex.split(found.stdout)]
        command = [
            compiler, "-std=c++17", "-O2", "-Wall", "-Wextra", "-Werror", "-pedantic",
            f"-I{ROOT / 'dist'}", f"-I{ROOT / 'tests' / 'support'}", str(source),
            "-pthread", *([] if sys.platform in ("darwin", "win32") else ["-ldl"]),
            "-o", str(executable), *crypto,
        ]
    result = subprocess.run(
        command, cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        timeout=180, check=False,
    )
    if result.returncode:
        raise RuntimeError("benchmark build failed:\n" + result.stdout)
    return executable


def main() -> int:
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("--archives", type=Path, help="existing generate_reader_archives.py output")
    parser.add_argument("--output", type=Path, required=True, help="summary CSV path")
    parser.add_argument("--quick", action="store_true", help="256 KiB smoke matrix")
    parser.add_argument("--samples", action="store_true", help="write every read's offset and latency")
    parser.add_argument("--seed", type=lambda v: int(v, 0), default=0x4E5853455353494F)
    parser.add_argument("--repeats", type=int, default=0, help="default: 1 quick, 5 full")
    parser.add_argument("--reads", type=int, default=0, help="per-thread, default: 1000 quick, 3000 full")
    args = parser.parse_args()
    repeats = args.repeats or (1 if args.quick else 5)
    reads = args.reads or (1000 if args.quick else 3000)
    if not 1 <= repeats <= 50 or not 1000 <= reads <= 1_000_000:
        parser.error("repeats must be 1..50 and reads must be 1000..1000000")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    rows: list[dict[str, str]] = []
    comparisons: dict[tuple[str, str, str, str], str] = {}
    with tempfile.TemporaryDirectory(prefix="nx-session-synthetic-") as temp_name:
        temp = Path(temp_name)
        executable = compile_benchmark(temp)
        archives = args.archives or temp / "archives"
        if not args.archives:
            subprocess.run(
                [sys.executable, str(ROOT / "benchmarks" / "generate_reader_archives.py"), str(archives)],
                cwd=ROOT, check=True, timeout=180,
            )
        manifest = json.loads((archives / "archive_manifest.json").read_text(encoding="utf-8"))
        sample_dir = args.output.parent / (args.output.stem + "_samples")
        if args.samples:
            sample_dir.mkdir(parents=True, exist_ok=True)
        for archive in manifest["archives"]:
            if args.quick and (archive["block_kib"] != 256):
                continue
            path = archives / archive["file"]
            with path.open("rb") as stream:
                if hashlib.file_digest(stream, "sha256").hexdigest() != archive["sha256"]:
                    raise RuntimeError("archive manifest digest mismatch: " + archive["file"])
            # A genuine loose-file baseline, read through the SAME pinned
            # positional DiskSource, reveals the NX verified-cache overhead.
            # The same loose file is reused for the RAW and Zstd variants.
            inputs = [(path, archive["codec"], CASES)]
            if archive["codec"] == "raw":
                loose_path = archives / archive["loose_file"]
                with loose_path.open("rb") as stream:
                    if hashlib.file_digest(stream, "sha256").hexdigest() != archive["loose_sha256"]:
                        raise RuntimeError("loose source digest mismatch: " + archive["loose_file"])
                # Include each trace once, with the cache/prefetch both off.
                loose_cases = tuple(dict.fromkeys(
                    (mode, 0, 0, threads) for mode, _, _, threads in CASES
                ))
                inputs.append((loose_path, "loose", loose_cases))
            for input_path, codec, cases in inputs:
                for mode, cache, prefetch, threads in cases:
                    for repeat in range(repeats):
                        sample_path = (
                            sample_dir / (
                                f"{input_path.stem}_{codec}_{mode}_c{cache}_p{prefetch}_t{threads}_r{repeat}.csv"
                            )
                            if args.samples else None
                        )
                        command = [
                            str(executable), str(input_path), archive["profile"], codec,
                            str(archive["block_kib"]), mode, str(cache), str(prefetch),
                            str(reads), str(threads), str(args.seed),
                            str(sample_path) if sample_path else "-",
                        ]
                        result = subprocess.run(
                            command, cwd=ROOT, text=True, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, timeout=180, check=False,
                        )
                        if result.returncode:
                            raise RuntimeError(
                                f"failed {input_path.name} {mode} cache={cache}:\n"
                                + result.stdout + result.stderr
                            )
                        parsed = list(csv.DictReader(io.StringIO(result.stdout)))
                        if len(parsed) != 1:
                            raise RuntimeError("unexpected benchmark CSV: " + result.stdout)
                        row = parsed[0]
                        # Same source bytes and access offsets across loose,
                        # RAW, Zstandard and cache must remain identical.
                        key = (row["profile"], row["block_kib"], row["trace"], row["threads"])
                        expected = comparisons.setdefault(key, row["checksum"])
                        if row["checksum"] != expected:
                            raise RuntimeError(f"decoded content mismatch for {key}")
                        row["repeat"] = str(repeat)
                        rows.append(row)
        if not rows:
            raise RuntimeError("archive selection produced no samples")
        with args.output.open("w", newline="", encoding="utf-8") as stream:
            writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
            writer.writeheader()
            writer.writerows(rows)
    metadata = {
        "benchmark": "verified NX C++17 persistent Session with pinned positional DiskSource",
        "profiles": ["compressible", "incompressible"],
        "codecs": ["loose", "raw", "zstd"],
        "source_generator": "benchmarks/generate_reader_archives.py, synthetic 16 MiB files",
        "cpu": platform.processor() or platform.machine(),
        "os": platform.platform(),
        "python": platform.python_version(),
        "compiler": os.environ.get("CXX") or "default C++17 compiler",
        "seed": hex(args.seed),
        "quick": args.quick,
        "repeats": repeats,
        "reads_per_worker": reads,
        "cache": "decoded verified blocks; cold at first access, warm OS page cache after mount",
        "verification": "NX CRC-32C slices on cache misses; manifest SHA-256 for loose; cross-variant full-byte rolling checksum",
        "scope": "synthetic CPU/VFS microbench; NOT emulator FPS or real 1% lows",
        "rows": len(rows),
        "samples_written": args.samples,
    }
    args.output.with_suffix(".metadata.json").write_text(
        json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(f"{len(rows)} complete verified benchmark scenarios: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
