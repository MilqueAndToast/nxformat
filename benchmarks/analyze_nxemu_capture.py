"""Analyze actual captured frame/read events; never convert I/O timings to FPS.

Capture CSV units must be explicit: frame_time_ms and optional read_latency_us.
This is offline analysis, not an injected loader or a frame-capture provider.
"""
import argparse
import csv
import json
import math
from pathlib import Path
import statistics


def values(path, column):
    with path.open(newline='',encoding='utf-8-sig') as stream:
        rows=list(csv.DictReader(stream))
    result=[float(row[column]) for row in rows]
    if not result or any(not math.isfinite(v) or v<=0 for v in result):
        raise ValueError('capture durations must be finite and positive')
    return sorted(result)


def percentile(samples, fraction):
    return samples[max(0,math.ceil(len(samples)*fraction)-1)]


def main():
    parser=argparse.ArgumentParser(__doc__)
    parser.add_argument('--frames',type=Path,required=True)
    parser.add_argument('--reads',type=Path)
    parser.add_argument('--runtime',type=Path,help='optional session counters: cache_hits,cache_misses,cpu_seconds,rss_bytes')
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    frames=values(args.frames,'frame_time_ms')
    worst=frames[-max(1,math.ceil(len(frames)*.01)):]
    report={'frame_samples':len(frames),'average_fps':1000/statistics.mean(frames),
            'one_percent_low_fps':1000/statistics.mean(worst),
            'frametime_p99_ms':percentile(frames,.99),'frametime_p999_ms':percentile(frames,.999),
            'tail_sample_warning':None if len(frames)>=10000 else 'fewer than ten p99.9 tail samples; extend capture',
            'read_stalls':None,'runtime':None,
            'one_percent_low_definition':'1000 / mean slowest ceil(1% of frames), in milliseconds',
            'capture_kind':'caller-supplied real frame events; input provenance must be recorded separately'}
    if args.reads:
        reads=values(args.reads,'read_latency_us')
        report['read_stalls']={'samples':len(reads),'p99_us':percentile(reads,.99),'p999_us':percentile(reads,.999)}
    if args.runtime:
        with args.runtime.open(newline='',encoding='utf-8-sig') as stream:rows=list(csv.DictReader(stream))
        if len(rows)<2:raise ValueError('runtime counters require start and end samples')
        report['runtime']={key:float(rows[-1][key])-float(rows[0][key]) for key in ('cache_hits','cache_misses','cpu_seconds')}
        report['runtime']['max_rss_bytes']=max(int(row['rss_bytes']) for row in rows)
        if any(v<0 for v in report['runtime'].values()):raise ValueError('counter reset during capture')
    args.output.write_text(json.dumps(report,indent=2),encoding='utf-8')


if __name__=='__main__':main()
