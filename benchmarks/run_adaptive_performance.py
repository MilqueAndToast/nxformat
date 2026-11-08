"""Targeted Windows adaptive follow-up using the existing writer/trace framework.

No new archive generator, reader, sampler or compression implementation.
Identical archives share a timed run; aliases are labeled, never counted twice.
"""
from __future__ import annotations
import argparse
import csv
import io
import json
from pathlib import Path
import random
import subprocess
from run_final_performance import append_csv, digest, writer_call, SEED


def main():
    parser=argparse.ArgumentParser(__doc__)
    parser.add_argument('--fixtures',type=Path,required=True)
    parser.add_argument('--output-dir',type=Path,required=True)
    parser.add_argument('--writer',type=Path,required=True)
    parser.add_argument('--reader',type=Path,required=True)
    parser.add_argument('--repeats',type=int,default=3)
    parser.add_argument('--reads',type=int,default=10000)
    args=parser.parse_args()
    if not 10000<=args.reads<=1000000 or not 3<=args.repeats<=10:
        parser.error('at least 10000 reads and three repetitions are required')
    if args.output_dir.exists():parser.error('select a fresh result directory')
    args.output_dir.mkdir(parents=True);args.fixtures.mkdir(parents=True,exist_ok=True)
    cases=[]
    for profile in ('compressible','incompressible','mixed','marginal','sample-trap'):
        cases.append((profile,16,'single','uniform4k','cold',1,'adaptive-small'))
    for trace in ('uniform64k','uniform1m','sequential64k','sequential1m','mixed','hot4k'):
        hint='adaptive-sequential' if trace.startswith('sequential') else 'adaptive-large' if trace=='uniform1m' else 'adaptive-small' if trace=='hot4k' else 'adaptive-medium'
        cases.append(('mixed',16,'single',trace,'cold',1,hint))
    cases.append(('mixed',16,'single','hot4k','replay',1,'adaptive-small'))
    for trace in ('scatter4k','gameload'):
        for phase,threads in [('cold',1),('replay',1),('cold',4)]:
            cases.append(('mixed',16,'multi16',trace,phase,threads,'adaptive-heterogeneous'))
    cases.append(('mixed',1,'multi16','gameload','cold',1,'adaptive-heterogeneous'))
    builds=set()
    for profile,size,layout,_,_,_,hint in cases:
        builds.update((profile,size,layout,codec,chunk,policy) for codec,chunk,policy in
                      [('raw',256,'fixed'),('zstd',256,'fixed'),('zstd',256,hint),('zstd',64,'fixed')])
    archives={};manifest={}
    for profile,size,layout,codec,chunk,policy in sorted(builds):
        # First invocation prepares sources/loose parts outside recorded builds.
        command=[str(args.writer),str(args.fixtures),str(size),profile,codec,str(chunk),'3','1','512',layout,policy]
        writer_call(command)
        expected=None
        for repeat in range(args.repeats):
            row=writer_call(command)
            suffix='' if layout=='single' else '_multi16'
            suffix+= '' if policy=='fixed' else '_'+policy
            path=args.fixtures/f'{profile}_{size}_{codec}_{chunk}k_l3{suffix}.nx'
            value=digest(path)
            if expected is not None and value!=expected:raise RuntimeError('writer nondeterminism')
            expected=value;row.update(repeat=repeat,sha256=value)
            append_csv(args.output_dir/'writer_samples.csv',row)
        archives[(profile,size,layout,codec,chunk,policy)]=(path,value)
        manifest[path.name]={'sha256':value,'bytes':path.stat().st_size}
    for path in args.fixtures.glob('*.bin'):
        manifest[path.name]={'sha256':digest(path),'bytes':path.stat().st_size}
    (args.output_dir/'input_manifest.json').write_text(json.dumps(manifest,indent=2))
    samples=args.output_dir/'reads';samples.mkdir()
    checks={};actual=0;aliases=0
    for repeat in range(args.repeats):
        jobs=cases.copy();random.Random(SEED+repeat).shuffle(jobs)
        for profile,size,layout,trace,phase,threads,hint in jobs:
            storage=[('loose','loose',256,'fixed'),('raw256','raw',256,'fixed'),
                     ('fixed256','zstd',256,'fixed'),('adaptive','zstd',256,hint)]
            random.Random(SEED+repeat+len(trace)).shuffle(storage)
            measured={}
            for variant,codec,chunk,policy in storage:
                if codec=='loose':path=args.fixtures/f'{profile}_{size}.bin';value=manifest[path.name]['sha256']
                else:path,value=archives[(profile,size,layout,codec,chunk,policy)]
                # First file: adaptive heterogeneous starts with a small-read profile.
                actual_chunk=64 if codec=='zstd' and policy!='fixed' and (policy in ('adaptive-small','adaptive-medium','adaptive-heterogeneous') or size==1) else chunk
                if codec=='loose':actual_chunk=256
                key=('loose' if codec=='loose' else 'nx',value)
                alias=key in measured
                if alias:row=dict(measured[key]);aliases+=1
                else:
                    sample=samples/f'{actual:05d}_{profile}_{size}_{layout}_{variant}_{trace}_{phase}_t{threads}_r{repeat}.csv'
                    command=[str(args.reader),str(path),profile,codec,str(actual_chunk),trace,
                             '0' if codec=='loose' else '8','0',str(args.reads),str(threads),str(SEED),str(sample),'native',phase,'2']
                    result=subprocess.run(command,capture_output=True,text=True,timeout=600,check=True)
                    row=list(csv.DictReader(io.StringIO(result.stdout)))[0]
                    row['read_samples']=sample.name;actual+=1
                    expected_key=(profile,size,trace,threads)
                    if checks.setdefault(expected_key,row['checksum'])!=row['checksum']:raise RuntimeError('logical byte mismatch')
                    measured[key]=dict(row)
                row.update(variant=variant,layout=layout,size_mib=size,repeat=repeat,policy=policy,
                           archive_sha256=value,measurement='byte-identical alias' if alias else 'executed')
                append_csv(args.output_dir/'reader_samples.csv',row)
        print(f'completed repetition {repeat+1}/{args.repeats}; {actual} actual executions, {aliases} exact aliases',flush=True)
    (args.output_dir/'metadata.json').write_text(json.dumps({
        'platform':'Windows', 'repeats':args.repeats,'reads_per_thread':args.reads,
        'executed_scenarios':actual,'byte_identical_aliases':aliases,
        'os_cache':'explicitly warm; cold/replay means application cache',
        'trace_profiles':'synthetic calibration; no native/game identity claim',
        'writer_sha256':digest(args.writer),'reader_sha256':digest(args.reader),
        'acceptance':'assess both tails and throughput; retain fixed default if representative real traces are missing',
    },indent=2))
    print('PASS adaptive archive/trace campaign',flush=True)


if __name__=='__main__':main()
