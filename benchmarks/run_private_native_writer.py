"""Private optional native conversion/reconstruction sweep, existing framework.

Only measurements/manifests belong in public results. Sources stay read-only;
archives and native expected-byte manifests stay in the explicit private folder.
"""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import random
import subprocess
from run_final_performance import append_csv, digest, writer_call, SEED

FIELDS = ('elapsed_ns,input_read_ns,input_hash_ns,dedup_ns,compression_ns,'
          'staging_write_ns,archive_write_ns,verification_ns,compression_stage_ns,'
          'compression_staging_read_bytes,compression_queue_peak_bytes,'
          'accounted_memory_bytes,temp_peak_bytes,archive_bytes,input_bytes,'
          'output_sync_ns,atomic_publication_ns').split(',')


def parse(output):
    row = {}
    for line in output.splitlines():
        values = line.split(',')
        if values[0] == 'WRITER_DETAIL':
            if len(values) != len(FIELDS) + 2:
                raise RuntimeError('unexpected writer detail schema')
            row.update({values[1] + '_' + key: value for key, value in zip(FIELDS, values[2:])})
        elif values[:2] == ['METRIC', 'conversion']:
            row['conversion_elapsed_s'] = values[2]
            row['conversion_logical_bytes'] = values[3]
            row['conversion_archive_bytes'] = values[4]
            row['conversion_temp_peak_bytes'] = values[5]
            row['initial_inventory_read_bytes'] = values[8]
    if 'rebuild_elapsed_ns' not in row or 'initial_elapsed_ns' not in row:
        raise RuntimeError('native example did not complete both paths')
    return row


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--converter', type=Path, required=True)
    parser.add_argument('--private-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cpp-reader', type=Path, required=True)
    parser.add_argument('--repeats', type=int, default=5)
    parser.add_argument('sources', type=Path, nargs='+')
    args = parser.parse_args()
    if args.output.exists() or args.private_dir.exists():
        parser.error('select new result and private directories')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.private_dir.mkdir(parents=True)
    before = {str(p.resolve()): digest(p) for p in args.sources}
    expected = {}
    for repeat in range(args.repeats):
        workers = [1, 2, 4, 8]
        random.Random(SEED + repeat).shuffle(workers)
        for worker in workers:
            destination = (args.private_dir / f'workers{worker}.nx').resolve()
            environment = dict(os.environ, NX_BENCH_WORKERS=str(worker))
            row = writer_call([str(args.converter.resolve()), str(destination),
                               *[str(p.resolve()) for p in args.sources]], parse, environment)
            row.update(workers=worker, repeat=repeat)
            for kind, suffix in [('converted', '.nx'), ('rebuilt', '.rebuilt.nx'), ('repacked', '.repacked.nx')]:
                path = destination if kind == 'converted' else destination.with_suffix(suffix)
                value = digest(path)
                if expected.setdefault(kind, value) != value:
                    raise RuntimeError(f'nondeterministic {kind} output with {worker} workers')
                row[kind + '_sha256'] = value
            append_csv(args.output, row)
            print(f'PASS private native workers={worker} repeat={repeat}', flush=True)
    for worker in (1, 2, 4, 8):
        destination = (args.private_dir / f'workers{worker}.nx').resolve()
        for path in [destination, destination.with_suffix('.rebuilt.nx')]:
            subprocess.run([str(args.cpp_reader.resolve()), str(path), str(destination.with_suffix('.expected.tsv'))], check=True)
    after = {str(p.resolve()): digest(p) for p in args.sources}
    if before != after:
        raise RuntimeError('native sources changed during experiment')
    args.output.with_suffix('.metadata.json').write_text(json.dumps({
        'source_files': {Path(p).name: value for p, value in before.items()},
        'unchanged_sources': True, 'repeats': args.repeats, 'workers': [1, 2, 4, 8],
        'converter_sha256': digest(args.converter), 'cpp_reader_sha256': digest(args.cpp_reader),
        'verification': 'Strict conversion, repack, reconstruction, Rust inspection; full C++17 byte comparison on final outputs',
        'peak_rss_scope': 'entire native example process, including conversion/repack/reconstruction',
        'not_execution_validation': True,
    }, indent=2), encoding='utf-8')


if __name__ == '__main__':
    main()
