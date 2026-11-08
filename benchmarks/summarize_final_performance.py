"""Summarize retained rows without pooling per-repetition percentiles."""
import argparse
import csv
from collections import defaultdict
from pathlib import Path
import statistics as stats


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('directory', type=Path)
    args = parser.parse_args()
    rows = list(csv.DictReader((args.directory / 'reader_samples.csv').open()))

    def median(group, field):
        return stats.median(float(row[field]) for row in group)

    lines = ['# Measured summary', '',
             f'{len(rows)} scenarios; {sum(int(r["samples"]) for r in rows):,} checked reads.', '',
             'Columns are medians of the per-repetition measurements. Percentiles are not pooled.', '',
             '## Random 4 KiB, compressible 16 MiB, 256 KiB blocks', '',
             '| Storage | Cache MiB | p50 µs | p99 µs | p99.9 µs | CPU s | Peak RSS MiB |',
             '|---|---:|---:|---:|---:|---:|---:|']
    for codec, budget in [('loose', '0'), ('loose-cached', '8'), ('raw', '8'), ('zstd', '8')]:
        group = [r for r in rows if r['profile'] == 'compressible' and r['size_mib'] == '16'
                 and r['block_kib'] == '256' and r['trace'] == 'uniform4k' and r['codec'] == codec
                 and r['cache_mib'] == budget and r['threads'] == '1' and r['phase'] == 'cold'
                 and r['hash_provider'] != 'portable' and r['level'] == '3']
        if group:
            lines.append(f'| {codec} | {budget} | {median(group,"p50_us"):.3f} | '
                         f'{median(group,"p99_us"):.3f} | {median(group,"p999_us"):.3f} | '
                         f'{median(group,"cpu_s"):.6f} | {median(group,"peak_rss_bytes")/1048576:.2f} |')
    # Historical SHA-provider comparisons do not apply to current CRC-32C reads.
    if any(r['hash_provider'] == 'portable' for r in rows):
        lines += ['', '## Paired SHA-256, no cache, 256 KiB blocks', '',
                  '| Profile | Codec | Portable p99 µs | Native p99 µs | Portable p99.9 µs | Native p99.9 µs |',
                  '|---|---|---:|---:|---:|---:|']
        for profile in sorted({r['profile'] for r in rows}):
            for codec in ('raw', 'zstd'):
                base = [r for r in rows if r['profile'] == profile and r['size_mib'] == '16'
                        and r['block_kib'] == '256' and r['trace'] == 'uniform4k'
                        and r['codec'] == codec and r['cache_mib'] == '0']
                portable = [r for r in base if r['hash_provider'] == 'portable']
                native = [r for r in base if r['hash_provider'] != 'portable']
                if portable and native:
                    for a in portable:
                        b = next(r for r in native if r['repeat'] == a['repeat'])
                        for field in ('checksum', 'physical_read_calls', 'physical_read_bytes', 'hits', 'misses', 'decodes'):
                            assert a[field] == b[field], (profile, codec, field)
                    lines.append(f'| {profile} | {codec} | {median(portable,"p99_us"):.3f} | '
                                 f'{median(native,"p99_us"):.3f} | {median(portable,"p999_us"):.3f} | '
                                 f'{median(native,"p999_us"):.3f} |')
    lines += ['', '## Memory by cache budget (all retained workloads)', '',
              '| Cache MiB | Maximum process peak RSS MiB | Maximum cache payload MiB |',
              '|---:|---:|---:|']
    for budget in ('0', '8', '16', '32', '64'):
        group = [r for r in rows if r['cache_mib'] == budget]
        if group:
            # /proc/statm RSS is approximate and can exceed ru_maxrss slightly;
            # report the conservative maximum of all actual OS observations.
            maximum = max(max(int(r[k]) for k in ('peak_rss_bytes', 'baseline_rss_bytes', 'idle_rss_bytes',
                                                   'ready_rss_bytes', 'end_rss_bytes')) for r in group)
            lines.append(f'| {budget} | {maximum/1048576:.2f} | '
                         f'{max(int(r["cache_peak_bytes"]) for r in group)/1048576:.2f} |')
    if rows and 'mean_rss_bytes' in rows[0]:
        groups = defaultdict(list)
        keys = ('profile', 'size_mib', 'codec', 'block_kib', 'level', 'trace', 'cache_mib', 'prefetch', 'threads', 'phase')
        for row in rows:
            groups[tuple(row[k] for k in keys)].append(row)
        lines += ['', '## All extended configurations', '',
                  '| Profile / size MiB / codec / block KiB / level / trace / cache MiB / prefetch / threads / phase | n | p50 µs | p95 µs | p99 µs | p99.9 µs | MiB/s | CPU s | Mean RSS MiB | Max peak RSS MiB |',
                  '|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|']
        for key, group in sorted(groups.items()):
            rates = [float(r['logical_mib']) / float(r['elapsed_s']) for r in group]
            lines.append(f'| {" / ".join(key)} | {len(group)} | '
                         + ' | '.join(f'{median(group,field):.3f}' for field in ('p50_us', 'p95_us', 'p99_us', 'p999_us'))
                         + f' | {stats.median(rates):.2f} | {median(group,"cpu_s"):.6f} | '
                         f'{median(group,"mean_rss_bytes")/1048576:.2f} | '
                         f'{max(int(r["peak_rss_bytes"]) for r in group)/1048576:.2f} |')
    writer_path = args.directory / 'writer_samples.csv'
    if writer_path.exists():
        writers = list(csv.DictReader(writer_path.open()))
        if writers and 'workers' in writers[0]:
            groups = defaultdict(list)
            output_hashes = {}
            for row in writers:
                exact = tuple(row[k] for k in ('profile', 'codec', 'block_kib', 'level', 'layout'))
                assert output_hashes.setdefault(exact, row['sha256']) == row['sha256'], ('writer output mismatch', exact)
                assert int(row['rust_peak_increment_bytes']) <= int(row['accounted_memory_bytes'])
                groups[exact + (row['workers'],)].append(row)
            lines += ['', '## Writer: identical output across worker counts', '',
                      '| Profile / codec / block KiB / level / layout / workers | n | Archive MiB | Median ms | Min–max ms | CPU s | Max RSS MiB | Accounted MiB | Rust allocations |',
                      '|---|---:|---:|---:|---:|---:|---:|---:|---:|']
            for key, group in sorted(groups.items()):
                elapsed = [int(r['elapsed_ns']) / 1e6 for r in group]
                lines.append(f'| {" / ".join(key)} | {len(group)} | {median(group,"archive_bytes")/1048576:.4f} | '
                             f'{stats.median(elapsed):.3f} | {min(elapsed):.3f}–{max(elapsed):.3f} | '
                             f'{median(group,"cpu_s"):.6f} | {max(int(r["peak_rss_bytes"] or 0) for r in group)/1048576:.2f} | '
                             f'{median(group,"accounted_memory_bytes")/1048576:.2f} | {median(group,"rust_allocation_calls"):.0f} |')
            lines += ['', 'Compression worker durations are summed; parallel stage wall time includes spool reads,',
                      'ordered publication of frames, worker startup and teardown. These times overlap and must not be added.',
                      'Fixed-context workspace capacity is real reserved storage; RSS may be smaller because untouched pages are not resident.',
                      'Rust allocation requests exclude C allocations in the serial compressor and OS/thread-runtime bookkeeping.', '']
    lines += ['', 'All configurations pre-read their pinned source to warm the OS cache.',
              'Cold/replay labels refer to application caches, not controlled cold storage.',
              'CPU and wall throughput include the untimed-per-read full-byte correctness checksum.',
              'RSS includes the executable, caller buffers, allocator retention and instrumentation.',
              'Allocation counts cover non-aligned C++ new/new[] only, not C/provider allocations.',
              'Short Windows CPU samples are quantized; zero does not prove zero CPU cost.',
              'These are file-service measurements, not frame times or FPS.', '']
    (args.directory / 'SUMMARY.md').write_text('\n'.join(lines), encoding='utf-8')
    print(args.directory / 'SUMMARY.md')


if __name__ == '__main__':
    main()
