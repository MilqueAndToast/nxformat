"""Create a deterministic source-and-fixture ZIP without build products."""

from __future__ import annotations

import hashlib
import os
import stat
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SKIP_DIRS = {".git", "__pycache__", ".pytest_cache", ".mypy_cache", "node_modules", "target", "build"}
SKIP_SUFFIXES = {".pyc", ".pyo", ".obj", ".o", ".exe", ".dll", ".so", ".dylib", ".pdb", ".ilk", ".class"}
SKIP_NAMES = {"Thumbs.db", ".DS_Store"}


def included_files(output_path: Path) -> list[Path]:
    output_real = output_path.resolve()
    files: list[Path] = []
    for current, directories, names in os.walk(ROOT):
        directories[:] = sorted(name for name in directories if name not in SKIP_DIRS)
        for name in sorted(names):
            path = Path(current) / name
            if path.name in SKIP_NAMES or path.suffix.lower() in SKIP_SUFFIXES:
                continue
            if path.resolve() == output_real or not path.is_file():
                continue
            files.append(path)
    return sorted(files, key=lambda path: path.relative_to(ROOT).as_posix())


def create(output_path: Path) -> str:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output_path, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9, strict_timestamps=True) as archive:
        for path in included_files(output_path):
            relative = Path("nxformat") / path.relative_to(ROOT)
            info = zipfile.ZipInfo(relative.as_posix(), date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.create_system = 3
            info.external_attr = (stat.S_IFREG | 0o644) << 16
            info.flag_bits = 0
            archive.writestr(info, path.read_bytes(), compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)
    digest = hashlib.sha256(output_path.read_bytes()).hexdigest()
    return digest


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: python tools/package_source.py <output.zip>", file=sys.stderr)
        return 2
    output = Path(sys.argv[1]).resolve()
    digest = create(output)
    print(f"created {output} ({output.stat().st_size} bytes, sha256 {digest})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
