"""Run real_archive_benchmark read cases over a matrix and append to a CSV.

usage: python benchmarks/run_real_archive_benchmark.py <out.csv> <archive.nx> [<archive.nx> ...]
         --build name=exe:mode1,mode2 [--build ...]
         [--workloads rand4k,rand64k,rand1m,seq1m,asset] [--threads 1,4,8] [--states warm,cold]

Each executable is real_archive_benchmark.cpp built against one reader version.
Modes: atomic, lichii (one block per call), fast (default read()), partial.
"""
from __future__ import annotations

import argparse
import csv
import subprocess
from pathlib import Path

FIELDS = ["build", "archive", "mode", "workload", "threads", "state", "reads", "p50_us", "p95_us", "p99_us",
          "p999_us", "max_us", "mib_s", "allocs_per_read", "alloc_bytes_per_read", "peak_heap_bytes",
          "peak_private_bytes", "decompressions", "prefetch_submitted"]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("out", type=Path)
    parser.add_argument("archives", nargs="+", type=Path)
    parser.add_argument("--build", action="append", required=True)
    parser.add_argument("--workloads", default="rand4k,rand64k,rand1m,seq1m,asset")
    parser.add_argument("--threads", default="1,4,8")
    parser.add_argument("--states", default="warm,cold")
    args = parser.parse_args()
    builds = []
    for item in args.build:
        name, rest = item.split("=", 1)
        exe, modes = rest.rsplit(":", 1)
        builds.append((name, exe, modes.split(",")))
    new_file = not args.out.exists()
    with args.out.open("a", newline="") as handle:
        writer = csv.writer(handle)
        if new_file:
            writer.writerow(FIELDS)
        for archive in args.archives:
            for state in args.states.split(","):
                for workload in args.workloads.split(","):
                    for threads in args.threads.split(","):
                        for name, exe, modes in builds:
                            for mode in modes:
                                command = [exe, "read", str(archive), mode, workload, threads, state]
                                result = subprocess.run(command, capture_output=True, text=True)
                                line = next((l for l in result.stdout.splitlines() if l.startswith("RESULT,")), None)
                                if result.returncode or not line:
                                    print(f"FAIL {name} {archive.name} {mode} {workload} {threads} {state}: "
                                          f"{result.stderr.strip()[:200]}", flush=True)
                                    continue
                                row = line.split(",")[1:]
                                writer.writerow([name, *row])
                                handle.flush()
                                print(name, ",".join(row), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
