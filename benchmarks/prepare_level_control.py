"""Prepare an explicitly experimental writer with levels 14..18 enabled.

Uses the production writer and verification unchanged. The product limit stays
13 until measurements justify changing it. No proprietary inputs are copied.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('destination', type=Path)
    args = parser.parse_args()
    destination = args.destination.resolve()
    if destination.exists():
        parser.error('destination must be a new directory')
    target = destination / 'packer'
    shutil.copytree(ROOT / 'packer', target, ignore=shutil.ignore_patterns('target'))
    (destination / 'benchmarks').mkdir()
    shutil.copyfile(ROOT / 'benchmarks/alloc_metrics.rs', destination / 'benchmarks/alloc_metrics.rs')
    path = target / 'src/writer.rs'
    code = path.read_text(encoding='utf-8')
    original = hashlib.sha256(code.encode()).hexdigest()
    old = '!(1..=13).contains(&options.zstd_compression_level)'
    if code.count(old) != 1:
        raise RuntimeError('review required: writer level validation changed')
    code = code.replace(old, '!(1..=18).contains(&options.zstd_compression_level)')
    code = code.replace('Zstandard compression level must be from 1 through 13',
                        'EXPERIMENTAL benchmark level must be from 1 through 18')
    path.write_text(code, encoding='utf-8')
    (destination / 'EXPERIMENTAL_LEVEL_CONTROL.json').write_text(json.dumps({
        'product_writer_sha256_normalized_utf8': original,
        'control_writer_sha256_normalized_utf8': hashlib.sha256(code.encode()).hexdigest(),
        'changes': ['validation range 1..13 to 1..18', 'validation error text'],
        'limits': 'unchanged memory, window, dedup, RAW fallback, verification and publication policies',
        'not_product_support': True,
    }, indent=2), encoding='utf-8')
    print(target / 'Cargo.toml')


if __name__ == '__main__':
    main()
