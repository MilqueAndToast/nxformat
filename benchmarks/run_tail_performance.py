"""Paired tail-latency follow-up using the existing production-session benchmark.

No game payloads are written. OS caches are warm/uncontrolled, not flushed.
Provide separately compiled baseline and candidate executables.
"""
import argparse
import csv
import io
import json
import random
import subprocess
from pathlib import Path
from run_final_performance import digest, SEED


def main():
    p = argparse.ArgumentParser(__doc__)
    p.add_argument('--baseline', type=Path, required=True)
    p.add_argument('--optimized', type=Path, required=True)
    p.add_argument('--fixtures', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--traces', nargs='+', default=['hot4k', 'cross4k', 'sequential1m', 'mixed'],
                   choices=['hot4k', 'cross4k', 'sequential1m', 'mixed'])
    p.add_argument('--scan-reads', type=int, default=20000)
    args = p.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = {'binaries': {n: digest(getattr(args, n)) for n in ('baseline', 'optimized')},
                'policy': 'warm OS, 8 MiB NX cache, no prefetch, native SHA, fresh session; three shuffled paired repetitions',
                'scope': 'synthetic production-writer fixtures; C++ Session, not game frametimes'}
    (args.output / 'manifest.json').write_text(json.dumps(manifest, indent=2))
    jobs = [(r, codec, trace, threads, count) for r in range(3)
            for codec in ('raw', 'zstd')
            for trace, threads, count in [('hot4k', 1, 100000), ('cross4k', 1, 100000),
                                          ('sequential1m', 1, 20000), ('mixed', 4, 25000)]]
    jobs = [(r, c, t, th, args.scan_reads if t == 'sequential1m' else n)
            for r, c, t, th, n in jobs if t in args.traces]
    random.Random(SEED).shuffle(jobs)
    for index, (r, codec, trace, threads, count) in enumerate(jobs):
        variants = ['baseline', 'optimized']
        random.Random(SEED + index).shuffle(variants)
        checksum = None
        for variant in variants:
            archive = args.fixtures / f'mixed_16_{codec}_256k_l3.nx'
            command = [str(getattr(args, variant)), str(archive), 'mixed', codec, '256', trace,
                       '8', '0', str(count), str(threads), str(SEED), '-', 'native', 'cold', '5']
            result = subprocess.run(command, check=True, text=True, capture_output=True)
            row = list(csv.DictReader(io.StringIO(result.stdout)))[0]
            if checksum is not None and row['checksum'] != checksum:
                raise RuntimeError('paired returned-byte checksum mismatch')
            checksum = row['checksum']
            row.update(variant=variant, repeat=r)
            name = f'{r}-{codec}-{trace}-{variant}.json'
            (args.output / name).write_text(json.dumps(row, indent=2))
        print(f'PASS {r} {codec} {trace}', flush=True)


if __name__ == '__main__':
    main()
