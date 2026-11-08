"""Production-writer fixtures and paired, warm-OS reader/memory experiments.

Default: five repetitions with raw samples. Windows can use the actual NXEmu
VFS trace executable; Linux uses the production C++17 Session and OpenSSL.
"""
from __future__ import annotations
import argparse
import csv
from datetime import datetime, timezone
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import random
import subprocess
import time
import tempfile
from run_session_synthetic import compile_benchmark
from run_writer_compression import peak_rss_sampler

ROOT = Path(__file__).resolve().parents[1]
SEED = 0x4E5853455353494F


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def append_csv(path, row):
    exists = path.exists()
    with path.open('a', newline='', encoding='utf-8') as stream:
        writer = csv.DictWriter(stream, fieldnames=list(row))
        if not exists:
            writer.writeheader()
        writer.writerow(row)


def writer_call(command, parse_output=None, env=None):
    started = time.perf_counter()
    before = 0
    if os.name != 'nt':
        import resource
        r = resource.getrusage(resource.RUSAGE_CHILDREN)
        before = r.ru_utime + r.ru_stime
    # The private native example prints detailed stats. A pipe can fill while
    # the RSS sampler waits for exit; capture to a file so progress never
    # depends on draining a pipe in another thread.
    with tempfile.TemporaryFile(mode='w+t', encoding='utf-8') as log:
        process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, text=True, env=env)
        io_metrics = {'io_read_bytes': '', 'io_write_bytes': '', 'io_kind': 'unavailable'}
        peak = peak_rss_sampler(process, io_metrics)
        process.wait()
        log.seek(0)
        output = log.read(16 * 1024 * 1024 + 1)
        if len(output) > 16 * 1024 * 1024:
            raise RuntimeError('benchmark diagnostic output exceeds 16 MiB')
    if process.returncode:
        raise RuntimeError(output)
    cpu = 0
    if os.name == 'nt':
        import ctypes
        from ctypes import wintypes
        kernel = ctypes.WinDLL('kernel32')
        kernel.GetProcessTimes.argtypes = [wintypes.HANDLE, *([ctypes.POINTER(wintypes.FILETIME)] * 4)]
        create, exit_, system, user = (wintypes.FILETIME() for _ in range(4))
        if kernel.GetProcessTimes(int(process._handle), create, exit_, system, user):
            cpu = sum((v.dwHighDateTime << 32) | v.dwLowDateTime for v in (system, user)) / 1e7
    else:
        r = resource.getrusage(resource.RUSAGE_CHILDREN)
        cpu = r.ru_utime + r.ru_stime - before
    rows = [parse_output(output)] if parse_output else list(csv.DictReader(io.StringIO(output)))
    if len(rows) != 1:
        raise RuntimeError('unexpected writer output: ' + output)
    # Prefer the child's kernel high-water observation to a polling lower bound.
    child_peak = rows[0].get('self_peak_rss_bytes', '')
    if child_peak:
        peak = max(peak or 0, int(child_peak))
    return {**rows[0], 'wall_s': time.perf_counter() - started,
            'peak_rss_bytes': peak or '', 'cpu_s': cpu, **io_metrics}


def cases(suites, quick=False, profiles=None, levels=None):
    jobs = set()
    profiles = profiles or (('compressible',) if quick else ('compressible', 'incompressible'))

    def add(p, size, codec, block, level, mode, cache=8, prefetch=0, threads=1, sha='native', phase='cold'):
        jobs.add((p, size, codec, block, level, mode, cache, prefetch, threads, sha, phase))

    def four(p, size, block, mode, cache=8, threads=1, phase='cold'):
        for codec in ('loose', 'loose-cached', 'raw', 'zstd'):
            add(p, size, codec, block, 3, mode, 0 if codec == 'loose' else cache,
                threads=threads, phase=phase)

    for p in profiles:
        if suites & {'sha', 'integrity'}:
            for codec in ('raw', 'zstd'):
                for sha in ('native',):  # legacy provider argument; current reads use CRC-32C
                    for cache in (0, 64):
                        add(p, 16, codec, 256, 3, 'uniform4k', cache, sha=sha)
        if 'blocks' in suites:
            for block in ((256,) if quick else (64, 128, 256, 512, 1024)):
                for mode in ('uniform4k', 'cross4k', 'sequential64k'):
                    four(p, 16, block, mode)
        if 'memory' in suites:
            for cache in ((8,) if quick else (8, 16, 32, 64)):
                for threads in ((4,) if quick else (1, 2, 4, 8)):
                    for codec in ('loose-cached', 'raw', 'zstd'):
                        add(p, 16, codec, 256, 3, 'mixed', cache, threads=threads)
            for cache in (8, 64):
                for phase in ('cold', 'replay'):
                    four(p, 16, 256, 'hot4k', cache, phase=phase)
                for threads in (1, 8):
                    for codec in ('raw', 'zstd'):
                        for prefetch in (0, 1):
                            add(p, 16, codec, 256, 3, 'mixed', cache, prefetch, threads)
        if 'levels' in suites:
            for level in ((3,) if quick else (levels or (1, 3, 6, 9))):
                for mode in ('uniform4k', 'sequential1m'):
                    add(p, 16, 'zstd', 256, level, mode)
        if 'extended' in suites:
            for block in (64, 128, 256, 512, 1024):
                for mode in ('uniform64k', 'uniform1m', 'scatter4k', 'gameload'):
                    four(p, 16, block, mode)
            for mode in ('scatter4k', 'gameload'):
                four(p, 16, 256, mode, threads=4)
                four(p, 16, 256, mode, phase='replay')
        if 'policy' in suites:
            for block in (64, 128, 256, 512, 1024):
                four(p, 16, block, 'uniform4k')
        if 'large' in suites and not quick:
            for size in (250, 1024):
                for mode in ('uniform4k', 'sequential64k', 'sequential1m'):
                    four(p, size, 256, mode)
                for cache in (8, 16, 32, 64):
                    for threads in (1, 2, 4, 8):
                        for codec in ('raw', 'zstd'):
                            add(p, size, codec, 256, 3, 'mixed', cache, threads=threads)
                for codec in ('raw', 'zstd'):
                    add(p, size, codec, 256, 3, 'sequential64k', 8, 1)
    return sorted(jobs)


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--fixtures', type=Path, required=True)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--writer', type=Path)
    parser.add_argument('--reader', type=Path)
    parser.add_argument('--reuse-fixtures', action='store_true')
    parser.add_argument('--prepare-only', action='store_true')
    parser.add_argument('--quick', action='store_true')
    parser.add_argument('--suites', default='integrity,blocks,memory,levels,large')
    parser.add_argument('--profiles', help='optional synthetic profiles, e.g. marginal for threshold study')
    parser.add_argument('--levels', default='1,3,6,9', help='explicit levels; >13 requires a labeled experimental writer build')
    parser.add_argument('--level-control-manifest', type=Path)
    parser.add_argument('--workers', default='1', help='writer worker sweep; output hashes must match exactly')
    parser.add_argument('--worker-levels', default='3,9', help='levels swept with multiple workers')
    parser.add_argument('--memory-mib', type=int, default=512)
    parser.add_argument('--rss-interval-ms', type=int, default=0, help='opt-in reader mean-RSS sampling; overhead included')
    parser.add_argument('--storage-label', default='unspecified', help='explicit filesystem/storage description for the report')
    parser.add_argument('--repeats', type=int, default=5)
    parser.add_argument('--reads', type=int, default=1000)
    args = parser.parse_args()
    if not 1 <= args.repeats <= 50 or not 1000 <= args.reads <= 1_000_000:
        parser.error('repeats 1..50, reads 1000..1000000')
    suites = set(args.suites.split(','))
    if suites - {'sha', 'integrity', 'blocks', 'memory', 'levels', 'large', 'extended', 'policy'}:
        parser.error('unknown suite')
    args.output_dir.mkdir(parents=True, exist_ok=True)
    args.fixtures.mkdir(parents=True, exist_ok=True)
    summary = args.output_dir / 'reader_samples.csv'
    if summary.exists():
        parser.error('output already exists; select a fresh output directory')
    profiles = tuple(args.profiles.split(',')) if args.profiles else None
    if profiles and set(profiles) - {'compressible', 'incompressible', 'mixed', 'marginal', 'sample-trap'}:
        parser.error('unknown synthetic profile')
    levels = tuple(int(v) for v in args.levels.split(','))
    workers = tuple(int(v) for v in args.workers.split(','))
    worker_levels = {int(v) for v in args.worker_levels.split(',')}
    if not levels or any(v < 1 or v > 18 for v in levels) or not workers or any(v < 1 or v > 8 for v in workers):
        parser.error('levels must be 1..18, workers 1..8')
    if max(levels) > 13 and not args.level_control_manifest:
        parser.error('levels >13 require --level-control-manifest from prepare_level_control.py')
    if not 1 <= args.memory_mib <= 4096 or not 0 <= args.rss_interval_ms <= 1000:
        parser.error('invalid memory or sampling budget')
    jobs = cases(suites, args.quick, profiles, levels)
    needed = {(p, size, codec, block, level) for p, size, codec, block, level, *_ in jobs
              if codec in ('raw', 'zstd')}
    manifest_path = args.fixtures / 'manifest.json'
    manifest = json.loads(manifest_path.read_text(encoding='utf-8')) if manifest_path.exists() else {}
    if not args.reuse_fixtures:
        binary = args.writer
        if not binary:
            subprocess.run(['cargo', 'build', '--release', '--locked', '--manifest-path', str(ROOT / 'packer/Cargo.toml'),
                            '--example', 'session_perf_cases'], check=True)
            binary = ROOT / 'packer/target/release/examples' / ('session_perf_cases.exe' if os.name == 'nt' else 'session_perf_cases')
        for p, size in sorted({(p, size) for p, size, *_ in jobs}):
            # Prepare payload outside timed construction. All chunk sizes/levels
            # read identical existing bytes through the real writer.
            if not (args.fixtures / f'{p}_{size}.bin').exists():
                writer_call([str(binary), str(args.fixtures), str(size), p, 'raw', '256', '3'])
            if 'extended' in suites and not all((args.fixtures / f'{p}_{size}.part{i:02}.bin').exists() for i in range(16)):
                # Materialize loose-tree references before measured builds.
                writer_call([str(binary), str(args.fixtures), str(size), p, 'raw', '256', '3',
                             '1', str(args.memory_mib), 'multi16'])
        for p, size, codec, block, level in sorted(needed):
            layouts = ('single', 'multi16') if 'extended' in suites else ('single',)
            for layout in layouts:
                suffix = '' if layout == 'single' else '_multi16'
                filename = f'{p}_{size}_{codec}_{block}k_l{level}{suffix}.nx'
                builds = [(repeat, w) for repeat in range(args.repeats)
                          for w in (workers if codec == 'zstd' and level in worker_levels else (1,))]
                random.Random(SEED + block + level).shuffle(builds)
                expected = None
                for repeat, w in builds:
                    command = [str(binary), str(args.fixtures), str(size), p, codec, str(block), str(level)]
                    # Retain compatibility with earlier six-argument binaries.
                    if workers != (1,) or layout != 'single' or args.memory_mib != 512:
                        command += [str(w), str(args.memory_mib), layout]
                    row = writer_call(command)
                    row['repeat'] = repeat
                    row['sha256'] = digest(args.fixtures / filename)
                    if expected is not None and row['sha256'] != expected:
                        raise RuntimeError('nondeterministic writer output: ' + filename)
                    expected = row['sha256']
                    manifest[filename] = {'sha256': expected, 'bytes': int(row['archive_bytes'])}
                    append_csv(args.output_dir / 'writer_samples.csv', row)
            print(f'Built/verified {filename}', flush=True)
        for p, size in sorted({(p, size) for p, size, *_ in jobs}):
            name = f'{p}_{size}.bin'
            manifest[name] = {'sha256': digest(args.fixtures / name), 'bytes': (args.fixtures / name).stat().st_size}
            if 'extended' in suites:
                for index in range(16):
                    part = f'{p}_{size}.part{index:02}.bin'
                    manifest[part] = {'sha256': digest(args.fixtures / part), 'bytes': (args.fixtures / part).stat().st_size}
        manifest_path.write_text(json.dumps(manifest, indent=2), encoding='utf-8')
    # Validate the retained source manifest using bounded reads, not filenames.
    for name, item in manifest.items():
        if digest(args.fixtures / name) != item['sha256']:
            raise RuntimeError('input SHA-256 mismatch: ' + name)
    if 'extended' in suites:
        for p, size in {(p, size) for p, size, *_ in jobs}:
            combined = hashlib.sha256()
            for index in range(16):
                with (args.fixtures / f'{p}_{size}.part{index:02}.bin').open('rb') as stream:
                    for chunk in iter(lambda: stream.read(65536), b''):
                        combined.update(chunk)
            if combined.hexdigest() != manifest[f'{p}_{size}.bin']['sha256']:
                raise RuntimeError('loose-tree bytes differ from the original payload')
    (args.output_dir / 'input_manifest.json').write_text(json.dumps(manifest, indent=2), encoding='utf-8')
    fingerprints = {'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                    'source_files_at_invocation': {name: digest(ROOT / name) for name in (
                        'packer/src/writer.rs', 'packer/src/writer_parallel.rs', 'packer/examples/session_perf_cases.rs',
                        'benchmarks/session_trace_benchmark.cpp', 'benchmarks/session_metrics.hpp',
                        'benchmarks/run_final_performance.py')},
                    'binaries': {str(p): digest(p) for p in (args.writer, args.reader) if p is not None}}
    fingerprints['source_hash_scope'] = 'working tree at invocation; binary hashes identify the executable, not a claim that it was rebuilt at invocation'
    (args.output_dir / 'build_fingerprints.json').write_text(json.dumps(fingerprints, indent=2), encoding='utf-8')
    if args.level_control_manifest:
        control = json.loads(args.level_control_manifest.read_text(encoding='utf-8'))
        if control.get('not_product_support') is not True:
            parser.error('invalid experimental level control manifest')
        (args.output_dir / 'EXPERIMENTAL_LEVEL_CONTROL.json').write_text(json.dumps(control, indent=2), encoding='utf-8')
    if args.prepare_only:
        (args.output_dir / 'metadata.json').write_text(json.dumps({
            'kind': 'writer-only', 'os': platform.platform(), 'cpu': platform.processor(),
            'repeats': args.repeats, 'workers': workers, 'levels': levels,
            'source_storage': args.storage_label, 'memory_mib': args.memory_mib,
            'output_equivalence': 'exact archive SHA-256 across repetitions and worker counts',
            'verification': 'unchanged production Strict output readback',
            'experimental_levels': max(levels) > 13,
        }, indent=2), encoding='utf-8')
        return
    samples = args.output_dir / 'reads'
    samples.mkdir(exist_ok=True)
    checksums = {}
    with tempfile.TemporaryDirectory(prefix='nx-final-perf-') as folder:
        reader = args.reader or compile_benchmark(Path(folder))
        done = 0
        for repeat in range(args.repeats):
            ordered = jobs.copy()
            random.Random(SEED + repeat).shuffle(ordered)
            for job in ordered:
                p, size, codec, block, level, mode, cache, prefetch, threads, sha, phase = job
                suffix = '_multi16' if mode in ('scatter4k', 'gameload') else ''
                name = f'{p}_{size}.bin' if codec in ('loose', 'loose-cached') else f'{p}_{size}_{codec}_{block}k_l{level}{suffix}.nx'
                sample = samples / (f'{done:05d}_{p}_{size}_{codec}_b{block}_l{level}_{mode}_c{cache}_p{prefetch}_t{threads}_{sha}_{phase}_r{repeat}.csv')
                command = [str(reader), str(args.fixtures / name), p, codec, str(block), mode, str(cache),
                           str(prefetch), str(args.reads), str(threads), str(SEED), str(sample), sha, phase]
                if args.rss_interval_ms:
                    command.append(str(args.rss_interval_ms))
                process = subprocess.run(command, capture_output=True, text=True, timeout=300)
                if process.returncode:
                    raise RuntimeError(f'failed {job}: {process.stdout}\n{process.stderr}')
                parsed = list(csv.DictReader(io.StringIO(process.stdout)))
                if len(parsed) != 1:
                    raise RuntimeError('unexpected reader CSV: ' + process.stdout)
                row = parsed[0]
                key = (p, size, block if mode == 'cross4k' else 0, mode, threads)
                if checksums.setdefault(key, row['checksum']) != row['checksum']:
                    raise RuntimeError('decoded checksum mismatch: ' + str(job))
                row.update(size_mib=size, level=level, repeat=repeat, read_samples=sample.name)
                append_csv(summary, row)
                done += 1
                if done % 20 == 0:
                    print(f'{done}/{len(jobs)*args.repeats} verified scenarios', flush=True)
    metadata = {'date': datetime.now(timezone.utc).isoformat(), 'os': platform.platform(), 'cpu': platform.processor(),
                'repeats': args.repeats, 'reads_per_worker': args.reads, 'scenarios': done,
                'suites': sorted(suites), 'quick': args.quick, 'seed': hex(SEED),
                'levels': levels, 'writer_workers': workers, 'memory_mib': args.memory_mib,
                'rss_sampling_interval_ms': args.rss_interval_ms,
                'experimental_levels': max(levels) > 13,
                'source_storage': args.storage_label,
                'order': 'deterministically shuffled/interleaved every repetition',
                'cache_policy': 'explicit full source warmup, cold NX/application cache or untimed trace replay',
                'timing': 'per-read service time; CPU/wall include full-byte correctness checksum; raw samples retained',
                'allocation_counter': 'non-aligned C++ new/new[] counts and requested bytes only; C/provider allocations excluded',
                'memory': 'OS process RSS/peak; owned metadata capacities; cache payload; ZSTD_sizeof_DCtx and pool storage when reported',
                'integrity': 'production writer strict readback; source SHA-256 manifest; mandatory block slice CRC-32C; full-byte rolling trace checksum',
                'limits': 'warm OS only, synthetic payloads; no FPS or mobile/Steam Deck hardware claim'}
    (args.output_dir / 'metadata.json').write_text(json.dumps(metadata, indent=2), encoding='utf-8')
    print(f'PASS {done} scenarios: {summary}', flush=True)


if __name__ == '__main__':
    main()
