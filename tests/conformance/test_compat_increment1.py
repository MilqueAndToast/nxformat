"""Verify that the historical and paged-header layouts reject each other."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def safe_extract_members(archive: zipfile.ZipFile, destination: Path, prefixes: tuple[str, ...]) -> None:
    root = destination.resolve()
    for member in archive.infolist():
        name = member.filename
        if not name.startswith(prefixes) or member.is_dir():
            continue
        target = (root / name).resolve()
        if not target.is_relative_to(root):
            raise ValueError(f"unsafe ZIP member path: {name}")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(archive.read(member))


def compiler_command() -> tuple[list[str], bool]:
    configured = os.environ.get("CXX")
    compiler = shutil.which(configured) if configured else (shutil.which("c++") or shutil.which("g++") or shutil.which("cl"))
    if not compiler:
        raise RuntimeError("no C++17 compiler found")
    return [compiler], Path(compiler).name.lower() in {"cl", "cl.exe"}


def build_smoke(source: Path, include_dist: Path, include_support: Path, output: Path, is_msvc: bool) -> None:
    compiler, _ = compiler_command()
    if is_msvc:
        command = [*compiler, "/nologo", "/std:c++17", "/EHsc", "/W4", "/WX", "/O2",
                   f"/I{include_dist}", f"/I{include_support}", str(source), f"/Fo:{output.with_suffix('.obj')}", f"/Fe:{output}"]
    else:
        dynamic_library = [] if sys.platform in ("darwin", "win32") else ["-ldl"]
        command = [*compiler, "-std=c++17", "-O2", "-Wall", "-Wextra", "-Werror", "-pedantic",
                   f"-I{include_dist}", f"-I{include_support}", str(source), "-pthread", *dynamic_library, "-o", str(output)]
    result = subprocess.run(command, cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    if result.returncode:
        raise RuntimeError(f"C++17 compatibility helper build failed:\n{result.stdout}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("increment1_zip", type=Path, help="previously delivered synthetic Increment 1 project ZIP")
    parser.add_argument("new_archive", type=Path, help="current archive with separate header pages")
    args = parser.parse_args()
    if not args.increment1_zip.is_file() or not args.new_archive.is_file():
        parser.error("both the Increment 1 ZIP and current archive must exist")
    _, is_msvc = compiler_command()
    with tempfile.TemporaryDirectory(prefix="nx-increment1-compat-") as temporary:
        temp = Path(temporary)
        previous = temp / "previous"
        previous.mkdir()
        with zipfile.ZipFile(args.increment1_zip) as archive:
            safe_extract_members(archive, previous, (
                "nxformat/tests/fixtures/", "nxformat/tests/conformance/reader_smoke.cpp",
                "nxformat/tests/support/", "nxformat/dist/",
            ))
        old_root = previous / "nxformat"
        index = json.loads((old_root / "tests/fixtures/fixture_index.json").read_text(encoding="utf-8"))
        current_smoke = temp / ("current_reader_smoke.exe" if is_msvc else "current_reader_smoke")
        build_smoke(ROOT / "tests/conformance/reader_smoke.cpp", ROOT / "dist", ROOT / "tests/support",
                    current_smoke, is_msvc)
        for item in index["fixtures"]:
            result = subprocess.run([str(current_smoke), str(old_root / "tests/fixtures" / item["file"]),
                                     "reject_open"], cwd=ROOT, text=True,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            if result.returncode:
                raise RuntimeError(f"current reader failed Increment 1 fixture {item['file']}:\n{result.stdout}")
        print(f"PASS current reader rejects all {len(index['fixtures'])} historical fixtures at open()")

        old_smoke = temp / ("increment1_reader_smoke.exe" if is_msvc else "increment1_reader_smoke")
        build_smoke(old_root / "tests/conformance/reader_smoke.cpp", old_root / "dist",
                    old_root / "tests/support", old_smoke, is_msvc)
        result = subprocess.run([str(old_smoke), str(args.new_archive.resolve()), "reject_open"], cwd=ROOT,
                                text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        if result.returncode:
            raise RuntimeError(f"Increment 1 reader did not reject the paged header layout at open():\n{result.stdout}")
        print("PASS previous Increment 1 reader rejects the paged header layout at open()")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
