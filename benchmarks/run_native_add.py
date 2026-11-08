"""Private/synthetic Increment 3 measurement and independent byte verification.

Usage: python benchmarks/run_native_add.py OUTPUT [NATIVE_SOURCE ...]
Never place private OUTPUT inside the repository. Originals remain read-only.
"""
from __future__ import annotations
import contextlib
import csv
import ctypes
import hashlib
import json
import mmap
from pathlib import Path
import struct
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tests/conformance"))
from test_native_conversion import compile_reader
from test_rust_writer import load_validator
from run_native_conversion import Memory, IO

EXE = ROOT / "packer/target/release/examples" / ("native_add.exe" if sys.platform == "win32" else "native_add")
COLUMNS = ["operation", "elapsed_seconds", "archive_bytes", "peak_temporary_disk_bytes",
           "accounted_memory_bytes", "logical_input_bytes_staged", "logical_bytes_considered_for_compression",
           "nx_native_logical_bytes_read", "nx_native_stored_bytes_read", "native_source_inspection_comparison_bytes_read",
           "inherited_file_bytes", "source_payload_stored_bytes_verified", "retained_frame_bytes_copied"]

def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()

@contextlib.contextmanager
def deny_original_reads(paths):
    """Windows private test: original dump reads fail throughout add's child process."""
    handles=[]
    if sys.platform == "win32":
        create=ctypes.windll.kernel32.CreateFileW
        create.argtypes=[ctypes.c_wchar_p,ctypes.c_uint32,ctypes.c_uint32,ctypes.c_void_p,ctypes.c_uint32,ctypes.c_uint32,ctypes.c_void_p]
        create.restype=ctypes.c_void_p
        close=ctypes.windll.kernel32.CloseHandle
        close.argtypes=[ctypes.c_void_p]
        try:
            for path in paths:
                handle=create(str(path),0x80000000,0,None,3,0x80,None)
                if handle == ctypes.c_void_p(-1).value:
                    raise OSError(f"Cannot lock original dump for isolation test: {path}")
                handles.append(handle)
            yield True
        finally:
            for handle in handles:
                close(handle)
    else:
        yield False

def measured(command, log):
    peak=read=written=0
    if sys.platform == "win32":
        from ctypes import wintypes
        get_memory=ctypes.windll.psapi.GetProcessMemoryInfo
        get_memory.argtypes=[wintypes.HANDLE,ctypes.POINTER(Memory),wintypes.DWORD]
        get_io=ctypes.windll.kernel32.GetProcessIoCounters
        get_io.argtypes=[wintypes.HANDLE,ctypes.POINTER(IO)]
        memory=Memory();memory.cb=ctypes.sizeof(memory);io=IO()
    started=time.perf_counter()
    with log.open("w",encoding="utf8") as stream:
        process=subprocess.Popen(command,cwd=ROOT,stdout=stream,stderr=subprocess.STDOUT)
        while process.poll() is None:
            if sys.platform == "win32":
                if get_memory(int(process._handle),ctypes.byref(memory),ctypes.sizeof(memory)):
                    peak=max(peak,memory.PeakWorkingSetSize)
                if get_io(int(process._handle),ctypes.byref(io)):
                    read=max(read,io.read_bytes);written=max(written,io.write_bytes)
            time.sleep(0.005)
    if process.returncode:
        raise RuntimeError(log.read_text(encoding="utf8",errors="replace"))
    metric=next(line for line in log.read_text(encoding="utf8").splitlines() if line.startswith("METRIC,")).split(",")[1:]
    assert len(metric)==len(COLUMNS),(metric,COLUMNS)
    return dict(zip(COLUMNS,metric)) | {"process_wall_seconds":time.perf_counter()-started,
        "sampled_peak_working_set_bytes":peak if sys.platform=="win32" else None,
        "os_read_transfer_bytes":read if sys.platform=="win32" else None,
        "os_write_transfer_bytes":written if sys.platform=="win32" else None}

def archive_evidence(path, validator, full):
    with path.open("rb") as stream, mmap.mmap(stream.fileno(),0,access=mmap.ACCESS_READ) as data:
        validator.validate_archive(data,verify_payloads=full)
        cat=struct.unpack_from("<Q",data,24)[0]
        count=struct.unpack_from("<I",data,cat+28)[0]
        table=cat+struct.unpack_from("<Q",data,cat+64)[0]
        identities=[data[table+i*120+56:table+i*120+120] for i in range(count)] # identity + Build ID
        return identities

def retained_frames(before, after):
    def records(data):
        table,count=struct.unpack_from("<QQ",data,88)
        integrity=struct.unpack_from("<Q",data,104)[0]
        result={}
        for i in range(count):
            off,stored,logical,codec,_,_,first=struct.unpack_from("<QIIBBHI",data,table+i*24)
            slices=(logical+16383)//16384  # NX 0.2: one CRC-32C per 16 KiB slice
            key=(logical,codec,data[integrity+first*4:integrity+(first+slices)*4])
            result.setdefault(key,[]).append((off,stored))
        return result
    with before.open("rb") as a, after.open("rb") as b:
        with mmap.mmap(a.fileno(),0,access=mmap.ACCESS_READ) as old, mmap.mmap(b.fileno(),0,access=mmap.ACCESS_READ) as new:
            previous=records(old);current=records(new)
            for key,ranges in previous.items():
                for off,size in ranges:
                    assert any(size==n and old[off:off+size]==new[p:p+n] for p,n in current.get(key,[])),"retained frame changed"
            return sum(map(len,previous.values()))

def run(output, sources, *, prepared=False, verifier=None):
    if output.resolve().is_relative_to(ROOT):
        raise ValueError("Keep native test outputs outside the source repository")
    output.mkdir(parents=True,exist_ok=True)
    sources=[p.resolve() for p in sources]
    if any(p.parent==output.resolve() for p in sources):
        raise ValueError("Keep original native sources outside the setup output directory")
    before={str(p):digest(p) for p in sources}
    if not prepared:
        subprocess.run([str(EXE),"prepare",str(output),*map(str,sources)],cwd=ROOT,check=True)
    else:
        if not (output/"base.nx").is_file() or not (output/"jobs.tsv").is_file():
            raise ValueError("prepared validation inputs are missing")
    reader=output/("native_reader.exe" if sys.platform=="win32" else "native_reader")
    if not prepared or not reader.exists():
        reader=compile_reader(output)
    validator=load_validator()
    current=output/"base.nx";results=[]
    for i,line in enumerate((output/"jobs.tsv").read_text().splitlines()):
        source,title,version=line.split("\t")
        added=output/f"step-{i}.nx"; rebuilt=output/f"rebuilt-{i}.nx"
        original_nx=digest(current); original_new=digest(Path(source))
        with deny_original_reads(sources) as locked:
            addition=measured([str(EXE),"add",str(current),str(added),source,title,version],output/f"add-{i}.log")
        assert digest(current)==original_nx and digest(Path(source))==original_new
        reconstruction=measured([str(EXE),"rebuild",str(current),str(rebuilt),title,version,*map(str,sources)],output/f"rebuild-{i}.log")
        a=archive_evidence(added,validator,full=False)
        b=archive_evidence(rebuilt,validator,full=False)
        assert a==b,"snapshot identities or Build IDs differ"
        frames=retained_frames(current,added)
        for path in [added,rebuilt]:
            subprocess.run([str(verifier or EXE),"verify",str(path),*map(str,sources)],cwd=ROOT,check=True)
            subprocess.run([str(reader),str(path),str(path.with_suffix(".expected.tsv")),str(path.with_suffix(".selection.tsv"))],cwd=ROOT,check=True)
        result={"step":i,"original_dumps_read_locked_during_add":locked,
                "source_nx_unchanged":True,"new_native_source_unchanged":True,
                "identities_and_build_ids_equal":True,"retained_frames_byte_identical":frames,
                "add":addition,"rebuild":reconstruction}
        results.append(result);print(json.dumps(result),flush=True);current=added
    assert before=={str(p):digest(p) for p in sources},"original dump changed"
    report={"date":"2026-11-04","source_file_hashes_checked":len(sources),"sources_unchanged":True,
        "sample_period_seconds":0.005,"verification":"Strict NX source/output; Strict native inspection with documented native memory caps; reconstruction additionally performs writer block readback; setup and independent verification excluded from process metrics",
        "scope":"one process per operation; OS transfers include cache/metadata, not physical disk IO; sampling can miss final peak; accounted memory is conservative, not RSS; no source paths/hashes/native bytes in this report",
        "results":results}
    (output/"measurements.json").write_text(json.dumps(report,indent=2)+"\n")
    with (output/"metrics.csv").open("w",newline="") as stream:
        fields=["step",*COLUMNS,"process_wall_seconds","sampled_peak_working_set_bytes","os_read_transfer_bytes","os_write_transfer_bytes"]
        writer=csv.DictWriter(stream,fields);writer.writeheader()
        for result in results:
            for operation in ["add","rebuild"]:writer.writerow({"step":result["step"],**result[operation]})
    return report

def main():
    subprocess.run(["cargo","build","--release","--locked","--manifest-path",str(ROOT/"packer/Cargo.toml"),"--example","native_add"],cwd=ROOT,check=True)
    output=Path(sys.argv[1]).resolve();sources=[Path(p) for p in sys.argv[2:]]
    if not sources:
        fixtures=output/"synthetic-sources"
        subprocess.run([str(EXE),"synthetic",str(fixtures)],cwd=ROOT,check=True)
        sources=sorted(fixtures.glob("*.nsp"))
    run(output,sources)
    return 0

if __name__=="__main__":raise SystemExit(main())
