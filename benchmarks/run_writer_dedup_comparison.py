"""Run the same synthetic RAW workloads against two release executables."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import platform
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path


SCENARIOS = (
    "unique-random",
    "repetitive",
    "snapshots-sharing-most",
    "limited-modifications",
    "many-small-files",
)


def expected_input_blocks(name: str) -> int:
    return {
        "unique-random": 256,
        "repetitive": 256,
        "snapshots-sharing-most": 512,
        "limited-modifications": 512,
        "many-small-files": 16_384,
    }[name]


def peak_rss_sampler(process: subprocess.Popen[str]) -> int | None:
    """Return Windows peak working set or Linux VmHWM while the process runs."""
    peak = 0
    if os.name == "nt":
        import ctypes
        from ctypes import wintypes

        class Counters(ctypes.Structure):
            _fields_ = [
                ("cb", wintypes.DWORD),
                ("PageFaultCount", wintypes.DWORD),
                ("PeakWorkingSetSize", ctypes.c_size_t),
                ("WorkingSetSize", ctypes.c_size_t),
                ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
                ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
                ("PagefileUsage", ctypes.c_size_t),
                ("PeakPagefileUsage", ctypes.c_size_t),
                ("PrivateUsage", ctypes.c_size_t),
            ]

        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        psapi = ctypes.WinDLL("psapi", use_last_error=True)
        kernel32.OpenProcess.restype = wintypes.HANDLE
        kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        psapi.GetProcessMemoryInfo.argtypes = [
            wintypes.HANDLE,
            ctypes.POINTER(Counters),
            wintypes.DWORD,
        ]
        psapi.GetProcessMemoryInfo.restype = wintypes.BOOL
        kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel32.CloseHandle.restype = wintypes.BOOL
        handle = kernel32.OpenProcess(0x0400 | 0x0010, False, process.pid)
        if not handle:
            return None
        try:
            while process.poll() is None:
                counters = Counters()
                counters.cb = ctypes.sizeof(counters)
                if psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
                    peak = max(peak, int(counters.PeakWorkingSetSize))
                time.sleep(0.002)
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
            except (FileNotFoundError, ProcessLookupError):
                break
            time.sleep(0.002)
        return peak or None
    return None


def run_one(binary: Path, variant: str, scenario: str, archive: Path, *, dedup_enabled: bool) -> dict[str, object]:
    process = subprocess.Popen(
        [str(binary), scenario, str(archive)],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    start = time.perf_counter()
    peak_rss = peak_rss_sampler(process)
    output, _ = process.communicate()
    wall_seconds = time.perf_counter() - start
    if process.returncode:
        raise RuntimeError(f"{variant}/{scenario} failed:\n{output}")
    rows = list(csv.DictReader(output.splitlines()))
    if len(rows) != 1:
        raise RuntimeError(f"unexpected benchmark output from {binary}:\n{output}")
    row = rows[0]
    blob = archive.read_bytes()
    data_size = struct.unpack_from("<Q", blob, 64)[0]
    block_count = int(row["blocks"])
    digest = hashlib.sha256(blob).hexdigest()
    return {
        "variant": variant,
        "scenario": scenario,
        "input_bytes": int(row["input_bytes"]),
        "input_blocks": expected_input_blocks(scenario),
        "elapsed_us": int(row["elapsed_us"]),
        "wall_seconds": f"{wall_seconds:.6f}",
        "throughput_mib_s": f"{int(row['input_bytes']) / int(row['elapsed_us']) * 1_000_000 / (1024 * 1024):.3f}",
        "archive_bytes": int(row["archive_bytes"]),
        "data_region_bytes": data_size,
        "staging_block_bytes_written": data_size if dedup_enabled else int(row["input_bytes"]),
        "logical_chunks_not_stored_as_physical_blocks": expected_input_blocks(scenario) - block_count,
        "physical_blocks": block_count,
        "file_records": int(row["file_records"]),
        "estimated_writer_memory_bytes": int(row["accounted_memory_bytes"]),
        "peak_temporary_disk_bytes": int(row["peak_temporary_disk_bytes"]),
        "actual_peak_rss_bytes": peak_rss,
        "archive_sha256": digest,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--increment1", type=Path, required=True, help="Increment 1 bench_writer_scenarios executable")
    parser.add_argument("--candidate", type=Path, required=True, help="Increment 2 bench_writer_scenarios executable")
    parser.add_argument("--baseline-label", default="increment1")
    parser.add_argument("--candidate-label", default="increment2")
    parser.add_argument("--output-dir", type=Path, required=True, help="destination directory on benchmark storage")
    parser.add_argument("--repetitions", type=int, default=3)
    args = parser.parse_args()
    if args.repetitions < 1:
        parser.error("--repetitions must be at least one")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    variants = (
        (args.baseline_label, args.increment1.resolve(), False),
        (args.candidate_label, args.candidate.resolve(), True),
    )
    if any(not binary.is_file() for _, binary, _ in variants):
        parser.error("both benchmark executables must exist")

    rows: list[dict[str, object]] = []
    with tempfile.TemporaryDirectory(prefix="nx-writer-bench-", dir=args.output_dir) as temporary:
        for scenario in SCENARIOS:
            for repetition in range(args.repetitions):
                order = variants if repetition % 2 == 0 else tuple(reversed(variants))
                for variant, binary, dedup_enabled in order:
                    archive = Path(temporary) / f"{variant}-{scenario}-{repetition}.nx"
                    rows.append(
                        run_one(binary, variant, scenario, archive, dedup_enabled=dedup_enabled)
                    )

    output_csv = args.output_dir / "writer_dedup_increment2_samples.csv"
    with output_csv.open("w", newline="", encoding="utf-8") as stream:
        writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)
    comparison_note = (
        "The baseline is the supplied Increment 1 ZIP. It uses the portable in-tree SHA-256 and does not read-verify output payloads; the current crate defaults to RustCrypto sha2 and read-verifies output payloads. Do not attribute timing differences solely to block deduplication."
        if args.baseline_label == "increment1"
        else "The baseline is a temporary control generated by prepare_no_dedup_control.py from the current source. It disables global block sharing and restores stage-every-block/same-path rollback while retaining the same SHA backend, output verification, input buffer, and toolchain. This isolates the global block-deduplication path."
    )
    metadata = {
        "host": platform.platform(),
        "machine": platform.machine(),
        "python": platform.python_version(),
        "storage_path": str(args.output_dir.resolve()),
        "baseline_label": args.baseline_label,
        "candidate_label": args.candidate_label,
        "baseline_binary": str(variants[0][1]),
        "candidate_binary": str(variants[1][1]),
        "repetitions": args.repetitions,
        "measurement": "Writer elapsed from WriterStats; peak RSS from Windows PeakWorkingSetSize or Linux VmHWM polling.",
        "staging_bytes": "Baseline writes each input RAW chunk before same-path reuse; dedup candidate is the unique RAW Data spool length. Values are bytes passed to the block staging file, not physical device writes.",
        "logical_chunks_not_stored_as_physical_blocks": "Derived from total input chunks minus physical blocks; includes entire same-path File-record reuse in Increment 1, not just block-level dedup hits.",
        "rust_toolchain": "1.82.0 for both executables",
        "source_comparison_note": comparison_note,
    }
    (args.output_dir / "writer_dedup_increment2_samples.metadata.json").write_text(
        json.dumps(metadata, indent=2) + "\n", encoding="utf-8"
    )
    print(output_csv)
    print(f"samples={len(rows)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
