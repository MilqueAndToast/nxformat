"""Compare an increment3 reader CSV with the checked-in increment2 baseline."""

from __future__ import annotations

import csv
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / "benchmarks" / "results" / "reader_after_patch.csv"
CURRENT = ROOT / "benchmarks" / "results" / "reader_increment3.csv"
OUTPUT = ROOT / "benchmarks" / "results" / "reader_increment3_vs_increment2.csv"


def main() -> int:
    if not BASELINE.is_file() or not CURRENT.is_file():
        print("run the increment2 and increment3 reader benchmarks first", file=sys.stderr)
        return 2
    with BASELINE.open(newline="", encoding="utf-8") as stream:
        before = {tuple(row[key] for key in ("profile", "codec", "block_kib", "verify_read_sha256", "read_mode", "operation")): row
                  for row in csv.DictReader(stream)}
    rows = []
    with CURRENT.open(newline="", encoding="utf-8") as stream:
        for after in csv.DictReader(stream):
            provider = after["sha_provider"]
            if provider not in {"none", "portable"}:
                continue
            key = (after["profile"], after["codec"], after["block_kib"], after["verify_read_sha256"],
                   after["read_mode"], after["operation"])
            old = before.get(key)
            if not old:
                continue
            def percent(field: str) -> str:
                old_value, new_value = float(old[field]), float(after[field])
                return "" if old_value == 0 else f"{(new_value / old_value - 1.0) * 100.0:.3f}"
            rows.append({
                "profile": after["profile"], "codec": after["codec"], "block_kib": after["block_kib"],
                "verify_read_sha256": after["verify_read_sha256"], "sha_provider": provider,
                "read_mode": after["read_mode"], "operation": after["operation"],
                "before_p50_us": old["p50_us"], "after_p50_us": after["p50_us"], "p50_delta_percent": percent("p50_us"),
                "before_p95_us": old["p95_us"], "after_p95_us": after["p95_us"], "p95_delta_percent": percent("p95_us"),
                "before_p99_us": old["p99_us"], "after_p99_us": after["p99_us"], "p99_delta_percent": percent("p99_us"),
                "before_allocations": old["allocations_per_sample"], "after_allocations": after["allocations_per_sample"],
                "new_data_region_bytes_per_sample": after["data_region_bytes_per_sample"],
                "new_decode_p50_us": after["decode_p50_us"], "new_sha_p50_us": after["sha_p50_us"],
            })
    if not rows:
        print("no matching before/after cases", file=sys.stderr)
        return 1
    with OUTPUT.open("w", newline="", encoding="utf-8") as stream:
        writer = csv.DictWriter(stream, fieldnames=list(rows[0]), lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)
    read_rows = [row for row in rows if row["operation"].startswith(("random_", "sequential_"))]
    regressed = sum(float(row["p50_delta_percent"]) > 0 for row in read_rows)
    print(f"wrote {len(rows)} paired cases ({len(read_rows)} read cases) to {OUTPUT}")
    print(f"read cases with higher p50: {regressed}/{len(read_rows)}")
    for provider in ("none", "portable"):
        selected = [row for row in read_rows if row["sha_provider"] == provider]
        if selected:
            deltas = sorted(float(row["p50_delta_percent"]) for row in selected if row["p50_delta_percent"])
            middle = deltas[len(deltas) // 2] if deltas else 0.0
            print(f"{provider}: paired reads={len(selected)}, median p50 change={middle:+.1f}%")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
