"""Replay a held-out NXEmu VFS trace with the existing session benchmark.

Input capture/source paths stay private. Outputs contain timings/hashes only.
OS caches are uncontrolled; cold means a new NX application cache, not cold disk.
"""
import argparse
import json
import os
from pathlib import Path
import random
import subprocess
from run_final_performance import append_csv, digest, SEED


def main():
    p=argparse.ArgumentParser(__doc__)
    p.add_argument('--reader',type=Path,required=True)
    p.add_argument('--trace',type=Path,required=True)
    p.add_argument('--expected',type=Path,required=True)
    p.add_argument('--fixed256',type=Path,required=True)
    p.add_argument('--adaptive',type=Path,required=True)
    p.add_argument('--fixed64',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    args=p.parse_args()
    args.output.mkdir(parents=True,exist_ok=False)
    manifest={'trace_sha256':digest(args.trace),'reader_sha256':digest(args.reader),
              'scope':'serial VFS replay, 4 MiB maximum transfer buffer; latency sums transfer times for larger logical reads; no FPS or game execution measurements',
              'cache_policy':'64 MiB NX LRU, prefetch off; fresh application cache/cached replay, uncontrolled OS cache'}
    (args.output/'manifest.json').write_text(json.dumps(manifest,indent=2))
    canonical=None
    for repeat in range(3):
        jobs=[(name,path,mode,phase,capture) for name,path,mode,capture in
              [('native',args.fixed256,'native',False),('fixed256',args.fixed256,'nx',False),
               ('adaptive',args.adaptive,'nx',False),('fixed64',args.fixed64,'nx',False),
               ('capture_on',args.fixed256,'nx',True)] for phase in ('cold','replay')]
        random.Random(SEED+repeat).shuffle(jobs)
        for name,path,mode,phase,capture in jobs:
            result=args.output/f'{repeat}-{name}-{phase}.json'
            env=os.environ.copy();env.pop('NXEMU_VFS_TRACE_DIR',None)
            if capture:
                private=args.output/f'private-capture-{repeat}-{phase}';private.mkdir()
                env['NXEMU_VFS_TRACE_DIR']=str(private.resolve())
            subprocess.run([str(args.reader),'--captured',str(path),str(args.trace),str(args.expected),str(result),mode,phase],check=True,env=env)
            row=json.loads(result.read_text())
            if canonical is None:canonical=row['sha256']
            if row['sha256']!=canonical:raise RuntimeError('Native/NX read bytes differ')
            row.update(variant=name,repeat=repeat)
            append_csv(args.output/'samples.csv',row)
    print('PASS: 30 verified held-out replays, identical byte-stream SHA-256')


if __name__=='__main__':main()
