"""Run deterministic Release repack workloads and preserve per-run raw data."""

from __future__ import annotations

import argparse
import csv
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PACKER = ROOT / "packer" / "Cargo.toml"
SCENARIOS = (
    "add_small_update",
    "remove_old_update",
    "add_dlc",
    "replace_update",
    "retain_multiple_updates",
    "delete_only_snapshot",
    "cancel_latency",
    "many_tiny_files",
    "compressible",
    "incompressible",
    "mixed_chunk_sizes",
    "chunk_64",
    "chunk_128",
    "chunk_256",
    "chunk_512",
    "chunk_1024",
    "full_rebuild",
)
RESULT_FIELDS = (
    "scenario",
    "operation",
    "status",
    "rust_elapsed_ns",
    "archive_bytes",
    "retained_stored_bytes_copied",
    "added_stored_bytes_copied",
    "added_blocks_reused",
    "logical_bytes_considered_for_compression",
    "logical_bytes_stored_as_zstd",
    "compression_time_ns",
    "source_payload_bytes_verified",
    "output_bytes_written",
    "peak_temporary_disk_bytes",
)
RAW_FIELDS = (
    "scenario",
    "repetition",
    "operation",
    "status",
    "rust_elapsed_ns",
    "parent_elapsed_ns",
    "source_archive_bytes",
    "archive_bytes",
    "retained_stored_bytes_copied",
    "added_stored_bytes_copied",
    "added_blocks_reused",
    "logical_bytes_considered_for_compression",
    "logical_bytes_stored_as_zstd",
    "compression_time_ns",
    "source_payload_bytes_verified",
    "output_bytes_written",
    "peak_temporary_disk_bytes",
    "process_read_bytes",
    "process_write_bytes",
    "process_peak_rss_bytes",
    "process_io_counter_kind",
    "diagnostic",
    "cancellation_response_ns",
)


class ProcessMeter:
    """Poll per-process RSS and I/O without adding project dependencies."""

    def __init__(self, pid: int) -> None:
        self.kind = "unavailable"
        self.handle = None
        self.peak_rss = 0
        self.last_read = 0
        self.last_write = 0
        if os.name == "nt":
            self._init_windows(pid)
        elif sys.platform.startswith("linux"):
            self.kind = "Linux /proc io bytes (read_bytes/write_bytes)"
            self.proc = Path(f"/proc/{pid}")
        else:
            self.kind = "not available on this operating system"

    def _init_windows(self, pid: int) -> None:
        import ctypes
        from ctypes import wintypes

        class IoCounters(ctypes.Structure):
            _fields_ = [(name, ctypes.c_ulonglong) for name in (
                "ReadOperationCount", "WriteOperationCount", "OtherOperationCount",
                "ReadTransferCount", "WriteTransferCount", "OtherTransferCount",
            )]

        class MemoryCountersEx(ctypes.Structure):
            _fields_ = [
                ("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD),
                ("PeakWorkingSetSize", ctypes.c_size_t), ("WorkingSetSize", ctypes.c_size_t),
                ("QuotaPeakPagedPoolUsage", ctypes.c_size_t), ("QuotaPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t), ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
                ("PagefileUsage", ctypes.c_size_t), ("PeakPagefileUsage", ctypes.c_size_t),
                ("PrivateUsage", ctypes.c_size_t),
            ]

        self.ctypes = ctypes
        self.IoCounters = IoCounters
        self.MemoryCountersEx = MemoryCountersEx
        self.kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        self.psapi = ctypes.WinDLL("psapi", use_last_error=True)
        self.kernel32.OpenProcess.restype = wintypes.HANDLE
        self.handle = self.kernel32.OpenProcess(0x1000 | 0x0010, False, pid)
        if self.handle:
            self.kind = "Windows process I/O transfer counters (not device-cache misses)"

    def sample(self) -> tuple[int | None, int | None, int | None]:
        if os.name == "nt" and self.handle:
            io = self.IoCounters()
            mem = self.MemoryCountersEx()
            mem.cb = self.ctypes.sizeof(mem)
            if self.kernel32.GetProcessIoCounters(self.handle, self.ctypes.byref(io)):
                self.last_read = int(io.ReadTransferCount)
                self.last_write = int(io.WriteTransferCount)
            if self.psapi.GetProcessMemoryInfo(self.handle, self.ctypes.byref(mem), mem.cb):
                self.peak_rss = max(self.peak_rss, int(mem.PeakWorkingSetSize))
            return self.last_read, self.last_write, self.peak_rss
        if self.kind.startswith("Linux"):
            try:
                values = {}
                for line in (self.proc / "io").read_text().splitlines():
                    key, value = line.split(":", 1)
                    values[key] = int(value.strip())
                status = (self.proc / "status").read_text().splitlines()
                hwm = next((line for line in status if line.startswith("VmHWM:")), None)
                rss = int(hwm.split()[1]) * 1024 if hwm else 0
                self.peak_rss = max(self.peak_rss, rss)
                return values.get("read_bytes"), values.get("write_bytes"), self.peak_rss
            except (OSError, StopIteration, ValueError):
                return self.last_read, self.last_write, self.peak_rss or None
        return None, None, None

    def close(self) -> None:
        if os.name == "nt" and self.handle:
            self.kernel32.CloseHandle(self.handle)
            self.handle = None


def cargo_executable() -> str:
    configured = os.environ.get("CARGO")
    if configured:
        return configured
    found = shutil.which("cargo")
    if found:
        return found
    candidate = Path.home() / ".cargo" / ("bin/cargo.exe" if os.name == "nt" else "bin/cargo")
    if candidate.is_file():
        return str(candidate)
    raise RuntimeError("Cargo is required for repack benchmarks")


def build_release() -> Path:
    subprocess.run(
        [cargo_executable(), "build", "--locked", "--release", "--manifest-path", str(PACKER), "--example", "repack_benchmark"],
        cwd=ROOT,
        check=True,
    )
    exe = ROOT / "packer" / "target" / "release" / "examples" / ("repack_benchmark.exe" if os.name == "nt" else "repack_benchmark")
    if not exe.is_file():
        raise RuntimeError(f"Release benchmark executable missing: {exe}")
    return exe


def output_zstd_logical_bytes(path: Path) -> int:
    with path.open("rb") as archive:
        archive.seek(88)
        index_offset = int.from_bytes(archive.read(8), "little")
        archive.seek(96)
        block_count = int.from_bytes(archive.read(8), "little")
        total = 0
        for block_id in range(block_count):
            archive.seek(index_offset + block_id * 24)
            descriptor = archive.read(24)
            if len(descriptor) != 24:
                raise RuntimeError("short output block index while measuring Zstandard logical bytes")
            if descriptor[16] == 1:
                total += int.from_bytes(descriptor[12:16], "little")
        return total


def run_one(executable: Path, scenario: str, repetition: int, temp_root: Path) -> dict[str, object]:
    work = temp_root / f"{scenario}-{repetition}"
    work.mkdir()
    process = subprocess.Popen(
        [str(executable), scenario, str(work)],
        cwd=ROOT,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    assert process.stdout is not None and process.stdin is not None
    try:
        ready = process.stdout.readline().strip()
        if ready != "READY":
            stderr = process.stderr.read() if process.stderr else ""
            raise RuntimeError(f"benchmark setup failed for {scenario}: {ready!r}\n{stderr}")
        source_bytes = (work / "source.nx").stat().st_size
        meter = ProcessMeter(process.pid)
        before_read, before_write, _ = meter.sample()
        started = time.perf_counter_ns()
        process.stdin.write("go\n")
        process.stdin.flush()
        while process.poll() is None:
            meter.sample()
            time.sleep(0.005)
        parent_elapsed = time.perf_counter_ns() - started
        after_read, after_write, peak_rss = meter.sample()
        meter.close()
        stdout = process.stdout.read()
        stderr = process.stderr.read() if process.stderr else ""
        if process.returncode != 0:
            raise RuntimeError(f"benchmark process failed for {scenario}:\n{stdout}\n{stderr}")
        result_line = next((line for line in stdout.splitlines() if line.startswith("RESULT,")), None)
        if result_line is None:
            raise RuntimeError(f"benchmark produced no RESULT row for {scenario}: {stdout!r}")
        fields = next(csv.reader([result_line]))
        if len(fields) != len(RESULT_FIELDS) + 1 or fields[0] != "RESULT":
            raise RuntimeError(f"unexpected benchmark result row: {result_line}")
        result = dict(zip(RESULT_FIELDS, fields[1:], strict=True))
        if scenario == "full_rebuild" and not result["logical_bytes_stored_as_zstd"]:
            result["logical_bytes_stored_as_zstd"] = str(output_zstd_logical_bytes(work / "output.nx"))
        cancellation_line = next((line for line in stdout.splitlines() if line.startswith("CANCEL,")), None)
        cancellation_response = ""
        if cancellation_line is not None:
            cancellation_fields = next(csv.reader([cancellation_line]))
            if len(cancellation_fields) != 3 or cancellation_fields[1] != scenario:
                raise RuntimeError(f"unexpected cancellation timing row: {cancellation_line}")
            cancellation_response = cancellation_fields[2]
        result.update(
            repetition=repetition,
            parent_elapsed_ns=parent_elapsed,
            source_archive_bytes=source_bytes,
            process_read_bytes=(after_read - before_read) if after_read is not None and before_read is not None else "",
            process_write_bytes=(after_write - before_write) if after_write is not None and before_write is not None else "",
            process_peak_rss_bytes=peak_rss if peak_rss is not None else "",
            process_io_counter_kind=meter.kind,
            cancellation_response_ns=cancellation_response,
        )
        if stderr.strip():
            result["diagnostic"] = stderr.strip()
        return result
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, default=ROOT / "benchmarks" / "results" / "repack_increment4.csv")
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--scenarios", nargs="*", default=list(SCENARIOS))
    args = parser.parse_args()
    if args.repetitions < 1:
        parser.error("--repetitions must be positive")
    unknown = sorted(set(args.scenarios) - set(SCENARIOS))
    if unknown:
        parser.error("unknown scenarios: " + ", ".join(unknown))
    executable = build_release()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    rows: list[dict[str, object]] = []
    with tempfile.TemporaryDirectory(prefix="nx-repack-bench-") as temp_name:
        temp_root = Path(temp_name)
        for scenario in args.scenarios:
            for repetition in range(args.repetitions):
                row = run_one(executable, scenario, repetition, temp_root)
                rows.append(row)
                print(
                    f"{scenario} rep={repetition + 1}: status={row['status']} "
                    f"elapsed={int(row['rust_elapsed_ns']) / 1e6:.2f} ms "
                    f"copied={int(row['retained_stored_bytes_copied']) / (1024 * 1024):.2f} MiB "
                    f"temp_peak={int(row['peak_temporary_disk_bytes']) / (1024 * 1024):.2f} MiB "
                    f"cancel_response={row['cancellation_response_ns'] or 'n/a'} ns "
                    f"diagnostic={row.get('diagnostic', '')}"
                )
    fields = list(RAW_FIELDS)
    with args.output.open("w", encoding="utf-8", newline="") as output:
        writer = csv.DictWriter(output, fieldnames=fields, extrasaction="ignore", lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)
    metadata = {
        "method": "Release Rust writer/repacker; synthetic data only; each operation is timed after setup and after a READY barrier; source files are generated in a temporary directory; 3 samples by default",
        "system_cache": "source archives were just created before measurement; the OS cache state is warm/unknown and not cleared",
        "physical_io": "Linux read_bytes/write_bytes are kernel-accounted storage I/O; Windows uses per-process I/O transfer counters, which do not distinguish file-cache hits from device I/O; macOS counters are unavailable in this harness",
        "scenarios": args.scenarios,
        "repetitions": args.repetitions,
        "measurements": ["Rust wall time", "parent wall time", "source archive bytes", "retained/additional stored bytes", "logical compression input and selected Zstandard bytes", "compression callback time", "cancellation response time", "output bytes", "temporary disk peak", "per-process OS I/O counters when available", "peak process working set when available"],
        "os": platform.platform(),
        "cpu": platform.processor() or platform.machine(),
        "compiler": "cargo build --release using the installed stable Rust toolchain",
        "result_rows": len(rows),
        "raw_samples_csv": str(args.output),
    }
    metadata_path = args.output.with_suffix(".metadata.json")
    metadata_path.write_text(__import__("json").dumps(metadata, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {len(rows)} raw repack samples to {args.output}")
    print(f"metadata: {metadata_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
