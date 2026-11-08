"""Measure reproducible RAW/Zstandard writer workloads and phase timings."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCENARIOS = (
    "unique-random",
    "repetitive",
    "mixed-64m",
    "snapshots-sharing-most",
    "limited-modifications",
    "many-small-files",
)
WRITER_COLUMNS = (
    "scenario", "codec", "level", "chunk_kib", "verify", "input_bytes", "archive_bytes",
    "file_entries", "file_records", "blocks", "raw_blocks", "zstd_blocks", "deduplicated_blocks",
    "logical_stage_bytes", "stored_stage_bytes", "block_spool_bytes", "elapsed_us",
    "input_read_hash_us", "dedup_us", "compression_us", "archive_write_us", "verification_us",
    "input_bytes_per_second", "accounted_memory_bytes", "peak_temporary_disk_bytes",
)
SAMPLE_COLUMNS = (
    "scenario", "codec", "level", "chunk_kib", "verify", "repetition", "input_bytes", "archive_bytes",
    "data_region_bytes", "file_entries", "file_records", "blocks", "raw_blocks", "zstd_blocks",
    "deduplicated_blocks", "logical_stage_bytes", "stored_stage_bytes", "block_spool_bytes", "elapsed_us",
    "wall_us", "input_read_hash_us", "dedup_us", "compression_us", "archive_write_us", "verification_us",
    "input_bytes_per_second", "accounted_memory_bytes", "peak_temporary_disk_bytes", "peak_rss_bytes",
    "archive_sha256",
)


def peak_rss_sampler(process: subprocess.Popen[str], io_metrics: dict | None = None) -> int | None:
    """Sample process RSS/working set while writing; do not load the archive."""
    peak = 0
    if os.name == "nt":
        import ctypes
        from ctypes import wintypes

        class Counters(ctypes.Structure):
            _fields_ = [
                ("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD),
                ("PeakWorkingSetSize", ctypes.c_size_t), ("WorkingSetSize", ctypes.c_size_t),
                ("QuotaPeakPagedPoolUsage", ctypes.c_size_t), ("QuotaPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t), ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
                ("PagefileUsage", ctypes.c_size_t), ("PeakPagefileUsage", ctypes.c_size_t),
                ("PrivateUsage", ctypes.c_size_t),
            ]

        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        psapi = ctypes.WinDLL("psapi", use_last_error=True)
        kernel32.OpenProcess.restype = wintypes.HANDLE
        kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        psapi.GetProcessMemoryInfo.argtypes = [ctypes.c_void_p, ctypes.POINTER(Counters), wintypes.DWORD]
        psapi.GetProcessMemoryInfo.restype = wintypes.BOOL
        class IoCounters(ctypes.Structure):
            _fields_ = [(name, ctypes.c_ulonglong) for name in (
                'read_calls', 'write_calls', 'other_calls', 'read_bytes', 'write_bytes', 'other_bytes')]
        kernel32.GetProcessIoCounters.argtypes = [wintypes.HANDLE, ctypes.POINTER(IoCounters)]
        def sample_io():
            if io_metrics is not None:
                counters = IoCounters()
                if kernel32.GetProcessIoCounters(handle, ctypes.byref(counters)):
                    io_metrics.update(io_read_bytes=int(counters.read_bytes), io_write_bytes=int(counters.write_bytes),
                                      io_kind='Windows process transfer bytes (includes cache/metadata)')
        kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
        handle = kernel32.OpenProcess(0x0400 | 0x0010, False, process.pid)
        if not handle:
            return None
        try:
            while process.poll() is None:
                counters = Counters()
                counters.cb = ctypes.sizeof(counters)
                if psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
                    peak = max(peak, int(counters.PeakWorkingSetSize))
                sample_io()
                time.sleep(0.002)
            counters = Counters()
            counters.cb = ctypes.sizeof(counters)
            if psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
                peak = max(peak, int(counters.PeakWorkingSetSize))
            sample_io()
        finally:
            kernel32.CloseHandle(handle)
        return peak or None

    if sys.platform.startswith("linux"):
        status = Path(f"/proc/{process.pid}/status")
        while process.poll() is None:
            try:
                for line in status.read_text(encoding="ascii").splitlines():
                    if line.startswith("VmHWM:"):
                        peak = max(peak, int(line.split()[1]) * 1024)
                        break
                if io_metrics is not None:
                    values = dict(line.split(':', 1) for line in status.with_name('io').read_text().splitlines())
                    io_metrics.update(io_read_bytes=int(values['rchar']), io_write_bytes=int(values['wchar']),
                                      io_kind='Linux sampled process rchar/wchar (includes stdio/metadata)')
            except (FileNotFoundError, ProcessLookupError):
                break
            time.sleep(0.002)
        return peak or None
    return None


def configurations(scenario: str) -> list[tuple[str, int, int, bool]]:
    result: set[tuple[str, int, int, bool]] = set()
    result.add(("raw", 3, 256, True))
    result.add(("zstd", 3, 256, True))
    if scenario in {"unique-random", "repetitive", "mixed-64m"}:
        result.update(("zstd", 3, chunk, True) for chunk in (64, 128, 512, 1024))
        result.update(("zstd", level, 256, True) for level in (1, 6, 9))
        result.add(("zstd", 3, 256, False))
    return sorted(result, key=lambda item: (item[0], item[2], item[1], item[3]))


def run_one(binary: Path, scenario: str, codec: str, level: int, chunk: int, verify: bool,
            repetition: int, archive: Path) -> tuple[dict[str, str], int | None, int, str, int]:
    command = [str(binary), scenario, str(archive), codec, str(level), str(chunk), str(verify).lower()]
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    start = time.perf_counter_ns()
    peak_rss = peak_rss_sampler(process)
    output, _ = process.communicate()
    wall_us = (time.perf_counter_ns() - start) // 1000
    if process.returncode:
        raise RuntimeError(f"writer benchmark failed ({scenario}/{codec}/{level}/{chunk}/{verify}):\n{output}")
    rows = list(csv.DictReader(output.splitlines()))
    if len(rows) != 1 or tuple(rows[0]) != WRITER_COLUMNS:
        raise RuntimeError(f"unexpected writer benchmark output:\n{output}")
    with archive.open("rb") as stream:
        blob_header = stream.read(72)
    if len(blob_header) != 72:
        raise RuntimeError(f"writer emitted truncated archive: {archive}")
    data_size = int.from_bytes(blob_header[64:72], "little")
    digest = hashlib.sha256()
    with archive.open("rb") as stream:
        for chunk_bytes in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk_bytes)
    archive.unlink()
    return rows[0], peak_rss, wall_us, digest.hexdigest(), data_size


def run_increment2(binary: Path, scenario: str, repetition: int, archive: Path) -> dict[str, object]:
    """Run the supplied Increment 2 RAW writer's original scenario interface."""
    process = subprocess.Popen([str(binary), scenario, str(archive)], stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True)
    start = time.perf_counter_ns()
    peak_rss = peak_rss_sampler(process)
    output, _ = process.communicate()
    wall_us = (time.perf_counter_ns() - start) // 1000
    if process.returncode:
        raise RuntimeError(f"Increment 2 benchmark failed ({scenario}):\n{output}")
    parsed = list(csv.DictReader(output.splitlines()))
    if len(parsed) != 1:
        raise RuntimeError(f"unexpected Increment 2 benchmark output:\n{output}")
    old = parsed[0]
    with archive.open("rb") as stream:
        header = stream.read(72)
    if len(header) != 72:
        raise RuntimeError("Increment 2 emitted a truncated archive")
    data_size = int.from_bytes(header[64:72], "little")
    digest = hashlib.sha256()
    with archive.open("rb") as stream:
        for part in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(part)
    archive.unlink()
    return {
        "scenario": scenario, "codec": "increment2-raw", "level": "", "chunk_kib": 256,
        "verify": True, "repetition": repetition, "input_bytes": int(old["input_bytes"]),
        "archive_bytes": int(old["archive_bytes"]), "data_region_bytes": data_size,
        "file_entries": int(old["file_entries"]), "file_records": int(old["file_records"]),
        "blocks": int(old["blocks"]), "raw_blocks": int(old["blocks"]), "zstd_blocks": 0,
        "deduplicated_blocks": "", "logical_stage_bytes": int(old["input_bytes"]),
        "stored_stage_bytes": data_size, "block_spool_bytes": int(old["input_bytes"]),
        "elapsed_us": int(old["elapsed_us"]), "wall_us": wall_us,
        "input_read_hash_us": "", "dedup_us": "", "compression_us": "",
        "archive_write_us": "", "verification_us": "",
        "input_bytes_per_second": int(old["input_bytes_per_second"]),
        "accounted_memory_bytes": int(old["accounted_memory_bytes"]),
        "peak_temporary_disk_bytes": int(old["peak_temporary_disk_bytes"]),
        "peak_rss_bytes": peak_rss or "", "archive_sha256": digest.hexdigest(),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--writer", type=Path, required=True, help="Release bench_writer_scenarios executable")
    parser.add_argument("--increment2-exe", type=Path, help="Optional matched Increment 2 Release benchmark executable")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "benchmarks" / "results" / "increment3_2026-11-03")
    parser.add_argument("--scenarios", nargs="+", choices=SCENARIOS, default=list(SCENARIOS))
    parser.add_argument("--repetitions", type=int, default=3)
    args = parser.parse_args()
    binary = args.writer.resolve()
    if not binary.is_file():
        parser.error(f"writer executable does not exist: {binary}")
    increment2 = args.increment2_exe.resolve() if args.increment2_exe else None
    if increment2 is not None and not increment2.is_file():
        parser.error(f"Increment 2 executable does not exist: {increment2}")
    if args.repetitions < 1:
        parser.error("--repetitions must be at least one")
    scenarios = tuple(args.scenarios)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    rows: list[dict[str, object]] = []
    increment2_rows: list[dict[str, object]] = []
    with tempfile.TemporaryDirectory(prefix="nx-writer-zstd-", dir=args.output_dir) as temp_dir:
        temp = Path(temp_dir)
        for scenario_index, scenario in enumerate(scenarios):
            selected = configurations(scenario)
            for repetition in range(args.repetitions):
                compare_old = increment2 is not None and scenario != "mixed-64m"
                if compare_old and repetition % 2 == 0:
                    increment2_rows.append(run_increment2(
                        increment2, scenario, repetition, temp / "increment2.nx"
                    ))
                rotation = (repetition + scenario_index) % len(selected)
                for codec, level, chunk_kib, verify in selected[rotation:] + selected[:rotation]:
                    archive = temp / "current.nx"
                    result, peak_rss, wall_us, archive_hash, data_size = run_one(
                        binary, scenario, codec, level, chunk_kib, verify, repetition, archive
                    )
                    rows.append({
                        "scenario": scenario, "codec": codec, "level": level, "chunk_kib": chunk_kib,
                        "verify": verify, "repetition": repetition,
                        "input_bytes": result["input_bytes"], "archive_bytes": result["archive_bytes"],
                        "data_region_bytes": data_size,
                        "file_entries": result["file_entries"], "file_records": result["file_records"],
                        "blocks": result["blocks"], "raw_blocks": result["raw_blocks"], "zstd_blocks": result["zstd_blocks"],
                        "deduplicated_blocks": result["deduplicated_blocks"],
                        "logical_stage_bytes": result["logical_stage_bytes"], "stored_stage_bytes": result["stored_stage_bytes"],
                        "block_spool_bytes": result["block_spool_bytes"], "elapsed_us": result["elapsed_us"],
                        "wall_us": wall_us, "input_read_hash_us": result["input_read_hash_us"],
                        "dedup_us": result["dedup_us"], "compression_us": result["compression_us"],
                        "archive_write_us": result["archive_write_us"], "verification_us": result["verification_us"],
                        "input_bytes_per_second": result["input_bytes_per_second"],
                        "accounted_memory_bytes": result["accounted_memory_bytes"],
                        "peak_temporary_disk_bytes": result["peak_temporary_disk_bytes"],
                        "peak_rss_bytes": peak_rss or "", "archive_sha256": archive_hash,
                    })
                    if compare_old and repetition % 2 == 1 and (codec, level, chunk_kib, verify) == ("raw", 3, 256, True):
                        increment2_rows.append(run_increment2(
                            increment2, scenario, repetition, temp / "increment2.nx"
                        ))
                    print(f"{scenario} {codec} L{level} {chunk_kib}KiB verify={verify} rep={repetition + 1}: "
                          f"{int(result['elapsed_us']) / 1000:.2f} ms, {result['archive_bytes']} bytes")

    samples_path = args.output_dir / "writer_compression_increment3_samples.csv"
    with samples_path.open("w", newline="", encoding="utf-8") as stream:
        writer = csv.DictWriter(stream, fieldnames=SAMPLE_COLUMNS, lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)
    if increment2_rows:
        baseline_path = args.output_dir / "writer_increment2_raw_baseline_samples.csv"
        with baseline_path.open("w", newline="", encoding="utf-8") as stream:
            writer = csv.DictWriter(stream, fieldnames=SAMPLE_COLUMNS, lineterminator="\n")
            writer.writeheader()
            writer.writerows(increment2_rows)
    summary: list[dict[str, object]] = []
    keys = ("scenario", "codec", "level", "chunk_kib", "verify")
    grouped: dict[tuple[object, ...], list[dict[str, object]]] = {}
    for row in rows:
        grouped.setdefault(tuple(row[key] for key in keys), []).append(row)
    for key, group in grouped.items():
        summary.append({
            **dict(zip(keys, key)), "samples": len(group),
            **{field: int(statistics.median(int(row[field]) for row in group)) for field in
               ("input_bytes", "archive_bytes", "data_region_bytes", "elapsed_us", "wall_us",
                "input_read_hash_us", "dedup_us", "compression_us", "archive_write_us", "verification_us",
                "accounted_memory_bytes", "peak_temporary_disk_bytes")},
            "peak_rss_bytes": int(statistics.median(int(row["peak_rss_bytes"]) for row in group if row["peak_rss_bytes"]))
            if any(row["peak_rss_bytes"] for row in group) else None,
            "archive_sha256_identical": len({row["archive_sha256"] for row in group}) == 1,
        })
    summary_path = args.output_dir / "writer_compression_increment3_summary.csv"
    with summary_path.open("w", newline="", encoding="utf-8") as stream:
        writer = csv.DictWriter(stream, fieldnames=list(summary[0]), lineterminator="\n")
        writer.writeheader()
        writer.writerows(summary)
    metadata = {
        "host": platform.platform(), "machine": platform.machine(), "cpu": platform.processor(),
        "python": platform.python_version(), "rustc": subprocess.run(["rustc", "--version"], text=True,
            stdout=subprocess.PIPE, check=True).stdout.strip(),
        "storage_path": str(args.output_dir.resolve()), "repetitions": args.repetitions,
        "scenarios": list(scenarios), "configurations_per_scenario": {scenario: len(configurations(scenario)) for scenario in scenarios},
        "measurement": "WriterStats phase durations and accounted peak temp/memory; wall time includes process launch and archive SHA; RSS/Windows peak working set sampled during writer process.",
        "memory_note": "Writer-accounted memory is conservative budget accounting, not allocator-enforced RSS. Process peak RSS/working set is measured separately.",
        "cache_note": "Writer conversion reads deterministic synthetic streams, not source archive files; cold/warm cache does not apply. Output/staging share the output volume.",
        "storage_device": "Not classified; no SSD claim.",
        "workload_note": "All payloads are deterministic synthetic streams; no proprietary data or representative emulator traces.",
        "samples_csv": str(samples_path), "summary_csv": str(summary_path),
        "increment2_baseline_csv": str(args.output_dir / "writer_increment2_raw_baseline_samples.csv") if increment2_rows else None,
        "increment2_note": "Matched RAW scenario data, same output storage and Release toolchain. Both default writers read back and verify RAW payloads before publication; Increment 2 has no Zstandard or per-phase timings and its implementation differs. Treat wall/WriterStats timings as an implementation comparison, not isolated compression cost." if increment2_rows else None,
    }
    metadata_path = args.output_dir / "writer_compression_increment3.metadata.json"
    metadata_path.write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {len(rows)} samples to {samples_path}")
    print(f"wrote {len(summary)} configuration medians to {summary_path}")
    print(f"metadata: {metadata_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
