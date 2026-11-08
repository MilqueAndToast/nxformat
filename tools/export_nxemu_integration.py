"""Export only the NX reader integration, preserving unrelated NXEmu edits."""
import argparse
import difflib
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
TRACKED = [
    "src/nxemu-loader/CMakeLists.txt", "src/nxemu-loader/nxemu-loader.vcxproj",
    "src/nxemu-loader/core/file_sys/nca_metadata.h",
    "src/nxemu-loader/core/file_sys/program_metadata.cpp",
    "src/nxemu-loader/core/file_sys/registered_cache.h",
    "src/nxemu-loader/core/file_sys/romfs_factory.cpp",
    "src/nxemu-loader/core/file_sys/romfs_factory.h",
    "src/nxemu-loader/core/file_sys/patch_manager.cpp", "src/nxemu-loader/core/file_sys/patch_manager.h",
    "src/nxemu-loader/core/loader/loader.cpp", "src/nxemu-loader/core/loader/loader.h",
    "src/nxemu-loader/rom_info.cpp", "src/nxemu-loader/system_loader.cpp",
    "src/nxemu-loader/game_list_cache.cpp",
    "src/nxemu-module-spec/system_loader.h",
]
NEW = [
    "src/nxemu-loader/core/file_sys/vfs/vfs_nx.cpp",
    "src/nxemu-loader/core/file_sys/vfs/vfs_nx.h",
    "src/nxemu-loader/core/loader/nx.cpp", "src/nxemu-loader/core/loader/nx.h",
    "src/nxemu-loader/nxformat/nx_reader.hpp",
    "src/nxemu-loader/nxformat/nx_session.hpp",
    "src/nxemu-loader/nxformat/nx_disk_source.hpp",
    "src/nxemu-loader/nxformat/nx_native_sha256.hpp",
    "src/nxemu-loader/nxformat/nx_read_trace.hpp",
    "tests/unit/nxformat/native_reader.cpp",
    "tests/unit/nxformat/build_native_test.py",
    "tests/unit/nxformat/module_lifecycle.cpp",
    "tests/unit/nxformat/test_module_lifecycle.py",
    "tests/unit/nxformat/build_trace_posix.py",
]


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("--nxemu", type=Path, required=True)
    parser.add_argument("--base", default="HEAD", help="unchanged pre-integration revision, e.g. upstream/master in an isolated checkout")
    args = parser.parse_args()
    for name in ("nx_reader.hpp", "nx_session.hpp", "nx_disk_source.hpp", "nx_native_sha256.hpp", "nx_read_trace.hpp"):
        if (args.nxemu / "src/nxemu-loader/nxformat" / name).read_bytes() != (ROOT / "dist" / name).read_bytes():
            parser.error(f"Stale vendored header: {name}")
    patch = subprocess.check_output(["git", "diff", "--binary", args.base, "--", *TRACKED], cwd=args.nxemu).decode("utf-8")
    for name in NEW:
        data = (args.nxemu / name).read_text(encoding="utf-8").splitlines(keepends=True)
        patch += f"diff --git a/{name} b/{name}\nnew file mode 100644\n"
        patch += "".join(difflib.unified_diff([], data, fromfile="/dev/null", tofile="b/" + name))
    output = ROOT / "integration/nxemu-reader.patch"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(patch, encoding="utf-8", newline="\n")
    # Validate against the unchanged index; --check never applies or stages it.
    # In an integration checkout the live index already contains these files.
    # Use a disposable index at the requested baseline, leaving both real index
    # and unrelated work untouched.
    import os
    import tempfile
    with tempfile.TemporaryDirectory(prefix="nx-integration-index-") as temp:
        env=os.environ.copy();env['GIT_INDEX_FILE']=str(Path(temp)/'index')
        subprocess.run(["git","read-tree",args.base],cwd=args.nxemu,env=env,check=True)
        subprocess.run(["git", "apply", "--cached", "--check", str(output)], cwd=args.nxemu,env=env,check=True)
    print(f"PASS isolated NXEmu integration patch: {len(TRACKED)+len(NEW)} files")


if __name__ == "__main__":
    main()
