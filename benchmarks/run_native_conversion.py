"""Private local conversion benchmark; never copies native data into the repo.
Usage: python benchmarks/run_native_conversion.py OUTPUT SOURCE...
Without sources, generates the five redistributable synthetic scenarios.
"""
from __future__ import annotations
import csv
import ctypes
from ctypes import wintypes
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]

class Memory(ctypes.Structure):
    _fields_ = [("cb",wintypes.DWORD),("PageFaultCount",wintypes.DWORD)] + [(name,ctypes.c_size_t) for name in ["PeakWorkingSetSize","WorkingSetSize","QuotaPeakPagedPoolUsage","QuotaPagedPoolUsage","QuotaPeakNonPagedPoolUsage","QuotaNonPagedPoolUsage","PagefileUsage","PeakPagefileUsage"]]

class IO(ctypes.Structure):
    _fields_ = [(name,ctypes.c_ulonglong) for name in ["read_ops","write_ops","other_ops","read_bytes","write_bytes","other_bytes"]]

def digest(path):
    with path.open("rb") as stream: return hashlib.file_digest(stream,"sha256").hexdigest()

def main():
    output = Path(sys.argv[1]).resolve()
    output.mkdir(parents=True,exist_ok=True)
    sources = [Path(p).resolve() for p in sys.argv[2:]]
    before = {str(p):digest(p) for p in sources if p.is_file()}
    subprocess.run(["cargo","build","--release","--locked","--manifest-path",str(ROOT / "packer/Cargo.toml"),"--example","native_convert"],check=True,cwd=ROOT)
    executable = ROOT / "packer/target/release/examples" / ("native_convert.exe" if sys.platform == "win32" else "native_convert")
    command = [str(executable),str(output / "game.nx"),*map(str,sources)] if sources else [str(executable),"--synthetic",str(output)]
    memory = Memory(); memory.cb=ctypes.sizeof(memory); io=IO(); peak=read=written=0
    start=time.perf_counter()
    with (output / "driver.log").open("w",encoding="utf-8") as log:
        process = subprocess.Popen(command,stdout=log,stderr=subprocess.STDOUT,cwd=ROOT)
        if sys.platform == "win32":
            get_memory=ctypes.windll.psapi.GetProcessMemoryInfo
            get_memory.argtypes=[wintypes.HANDLE,ctypes.POINTER(Memory),wintypes.DWORD]
            get_io=ctypes.windll.kernel32.GetProcessIoCounters
            get_io.argtypes=[wintypes.HANDLE,ctypes.POINTER(IO)]
        while process.poll() is None:
            if sys.platform == "win32":
                if get_memory(int(process._handle),ctypes.byref(memory),ctypes.sizeof(memory)): peak=max(peak,memory.PeakWorkingSetSize)
                if get_io(int(process._handle),ctypes.byref(io)): read=max(read,io.read_bytes); written=max(written,io.write_bytes)
            time.sleep(0.02)
    after = {str(p):digest(p) for p in sources if p.is_file()}
    assert before == after,"source bytes changed"
    metadata = {"platform":sys.platform,"harness_wall_seconds":time.perf_counter()-start,"driver_sampled_peak_working_set_bytes":peak,"driver_os_read_transfer_bytes":read,"driver_os_write_transfer_bytes":written,"source_file_hashes_checked":len(before),"source_hashes_unchanged":before == after if before else None,"exit_code":process.returncode,"sample_period_seconds":0.02,"scope":"harness wall includes post-run source hashing; process memory and transfers cover the whole native driver: inventories, conversion, Strict inspect, Strict no-op repack, full reconstruction; OS transfers include cache and metadata, not physical disk IO"}
    (output / "measurement.json").write_text(json.dumps(metadata,indent=2)+"\n",encoding="utf-8")
    columns=["scenario","operation","elapsed_seconds","logical_snapshot_file_bytes","archive_bytes","peak_temporary_disk_bytes","deduplicated_blocks","initial_writer_accounted_memory_bytes","caller_inventory_bytes_read"]
    scenario="local";rows=[]
    for line in (output / "driver.log").read_text(encoding="utf-8").splitlines():
        if line.startswith("scenario="): scenario=line.split("=",1)[1]
        if line.startswith("METRIC,"):rows.append([scenario,*line.split(",")[1:]])
    with (output / "metrics.csv").open("w",newline="",encoding="utf-8") as stream:
        writer=csv.writer(stream);writer.writerow(columns);writer.writerows(rows)
    print(json.dumps(metadata,indent=2))
    return process.returncode

if __name__ == "__main__": raise SystemExit(main())
