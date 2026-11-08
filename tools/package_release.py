"""Packs one platform's release archive.

    python tools/package_release.py --platform windows-x64 --version 1.0.0 \
        --lib-dir ffi/target/x86_64-pc-windows-msvc/release \
        --app app/target/x86_64-pc-windows-msvc/release/nxconvert.exe --out out

Layout of nxformat-<version>-<platform>.zip|.tar.gz:
    include/   nxformat.h (C API) and the header-only C++17 NX reader
    lib/       shared library (Unix) or import library (Windows), plus
               nxformat_static.a|.lib; lib/cmake/nxformat for find_package
    bin/       nxformat.dll (Windows) and the nxconvert app (desktop);
               macOS also bin/NXConvert.app, Linux share/ (desktop entry, icon)
"""
import argparse
import hashlib
import shutil
import sys
import tarfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# What a program linking the static library also links.
STATIC_LIBS = {
    'windows': ['ws2_32', 'userenv', 'ntdll', 'bcrypt', 'advapi32', 'kernel32'],
    'linux': ['pthread', 'dl', 'm'],
    'macos': [],
    'android': ['dl', 'm', 'log'],
}

CMAKE_CONFIG = '''# nxformat {version} ({platform}).
#   nxformat::nxformat  the C API, shared library
#   nxformat::static    the C API, static library
#   nxformat::reader    the header-only C++17 NX reader (needs a zstd decoder)
get_filename_component(_nxformat_root "${{CMAKE_CURRENT_LIST_DIR}}/../../.." ABSOLUTE)
set(nxformat_VERSION {version})
if(NOT TARGET nxformat::nxformat)
  add_library(nxformat::nxformat SHARED IMPORTED)
  set_target_properties(nxformat::nxformat PROPERTIES
    INTERFACE_INCLUDE_DIRECTORIES "${{_nxformat_root}}/include"
    IMPORTED_LOCATION "${{_nxformat_root}}/{shared}"{implib})
  add_library(nxformat::static STATIC IMPORTED)
  set_target_properties(nxformat::static PROPERTIES
    INTERFACE_INCLUDE_DIRECTORIES "${{_nxformat_root}}/include"
    INTERFACE_COMPILE_DEFINITIONS NXFORMAT_STATIC
    INTERFACE_LINK_LIBRARIES "{static_libs}"
    IMPORTED_LOCATION "${{_nxformat_root}}/lib/{static}")
  add_library(nxformat::reader INTERFACE IMPORTED)
  target_compile_features(nxformat::reader INTERFACE cxx_std_17)
  set_target_properties(nxformat::reader PROPERTIES
    INTERFACE_INCLUDE_DIRECTORIES "${{_nxformat_root}}/include")
endif()
# The file to copy next to the program that uses nxformat::nxformat.
set(NXFORMAT_RUNTIME "${{_nxformat_root}}/{shared}")
unset(_nxformat_root)
'''

CMAKE_VERSION = '''set(PACKAGE_VERSION {version})
if(PACKAGE_FIND_VERSION VERSION_GREATER PACKAGE_VERSION OR
   NOT PACKAGE_FIND_VERSION_MAJOR EQUAL {major})
  set(PACKAGE_VERSION_COMPATIBLE FALSE)
else()
  set(PACKAGE_VERSION_COMPATIBLE TRUE)
  if(PACKAGE_FIND_VERSION VERSION_EQUAL PACKAGE_VERSION)
    set(PACKAGE_VERSION_EXACT TRUE)
  endif()
endif()
'''


ASSETS = ROOT / 'app/assets'
DESKTOP_ENTRY = """[Desktop Entry]
Type=Application
Name=NXConvert
Comment=Convert games to NX and manage NX archives
Exec=nxconvert
Icon=nxconvert
Terminal=false
Categories=Utility;Archiving;
"""
INFO_PLIST = """<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>NXConvert</string>
  <key>CFBundleDisplayName</key><string>NXConvert</string>
  <key>CFBundleIdentifier</key><string>io.github.fab918.nxconvert</string>
  <key>CFBundleExecutable</key><string>nxconvert</string>
  <key>CFBundleIconFile</key><string>nxconvert</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"""


def add_app_icon(stage: Path, app: Path, system: str, version: str) -> None:
    """Give the app its icon where the platform takes it from outside the
    binary (Windows embeds it in nxconvert.exe at build time).

    macOS: bin/NXConvert.app, a bundle around the same binary, opens the
    window from Finder. Linux: a desktop entry and its icon, under share/.
    """
    if system == 'macos':
        bundle = stage / 'bin/NXConvert.app/Contents'
        (bundle / 'MacOS').mkdir(parents=True)
        (bundle / 'Resources').mkdir()
        shutil.copy(app, bundle / 'MacOS/nxconvert')
        shutil.copy(ASSETS / 'nxconvert.icns', bundle / 'Resources')
        (bundle / 'Info.plist').write_text(INFO_PLIST.format(version=version), newline='\n')
    elif system == 'linux':
        icons = stage / 'share/icons/hicolor/512x512/apps'
        icons.mkdir(parents=True)
        shutil.copy(ASSETS / 'nxconvert.png', icons)
        (stage / 'share/applications').mkdir(parents=True)
        (stage / 'share/applications/nxconvert.desktop').write_text(DESKTOP_ENTRY, newline='\n')


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument('--platform', required=True, help='windows-x64, linux-x64, linux-arm64, macos-universal, android-arm64')
    p.add_argument('--version', required=True)
    p.add_argument('--lib-dir', type=Path, required=True)
    p.add_argument('--app', type=Path, help='the nxconvert binary (desktop platforms)')
    p.add_argument('--out', type=Path, required=True)
    a = p.parse_args()
    system = a.platform.split('-')[0]
    name = f'nxformat-{a.version}-{a.platform}'
    stage = a.out / name
    if stage.exists():
        print(f'{stage} already exists', file=sys.stderr)
        return 1
    (stage / 'include').mkdir(parents=True)
    (stage / 'lib/cmake/nxformat').mkdir(parents=True)
    (stage / 'bin').mkdir()

    shutil.copy(ROOT / 'LICENSE', stage)
    shutil.copy(ROOT / 'ffi/include/nxformat.h', stage / 'include')
    for header in sorted((ROOT / 'dist').glob('*.hpp')):
        shutil.copy(header, stage / 'include')

    lib = a.lib_dir
    implib = ''
    if system == 'windows':
        shutil.copy(lib / 'nxformat.dll', stage / 'bin')
        shutil.copy(lib / 'nxformat.dll.lib', stage / 'lib/nxformat.lib')
        shutil.copy(lib / 'nxformat.lib', stage / 'lib/nxformat_static.lib')
        shared, static = 'bin/nxformat.dll', 'nxformat_static.lib'
        implib = '\n    IMPORTED_IMPLIB "${_nxformat_root}/lib/nxformat.lib"'
    elif system == 'macos':
        shutil.copy(lib / 'libnxformat.dylib', stage / 'lib')
        shutil.copy(lib / 'libnxformat.a', stage / 'lib/libnxformat_static.a')
        shared, static = 'lib/libnxformat.dylib', 'libnxformat_static.a'
        implib = '\n    IMPORTED_SONAME "@rpath/libnxformat.dylib"'
    else:
        shutil.copy(lib / 'libnxformat.so', stage / 'lib')
        shutil.copy(lib / 'libnxformat.a', stage / 'lib/libnxformat_static.a')
        shared, static = 'lib/libnxformat.so', 'libnxformat_static.a'
        implib = '\n    IMPORTED_SONAME "libnxformat.so"'
    if a.app:
        shutil.copy(a.app, stage / 'bin')
        add_app_icon(stage, a.app, system, a.version)
    if not any((stage / 'bin').iterdir()):
        (stage / 'bin').rmdir()

    (stage / 'lib/cmake/nxformat/nxformatConfig.cmake').write_text(
        CMAKE_CONFIG.format(version=a.version, platform=a.platform, shared=shared, implib=implib,
                            static=static, static_libs=';'.join(STATIC_LIBS[system])), newline='\n')
    (stage / 'lib/cmake/nxformat/nxformatConfigVersion.cmake').write_text(
        CMAKE_VERSION.format(version=a.version, major=a.version.split(".")[0]), newline='\n')
    (stage / 'README.txt').write_text(
        f'nxformat {a.version} for {a.platform}\n'
        'https://github.com/fab918/nxformat\n\n'
        'include/nxformat.h  C API: runs the nxconvert commands in process\n'
        'include/*.hpp       header-only C++17 NX reader\n'
        'lib/                libraries; lib/cmake/nxformat for CMake find_package\n'
        'bin/                runtime library (Windows) and the nxconvert app\n'
        + ('bin/NXConvert.app   the same app as a macOS bundle, with its icon\n'
           if system == 'macos' and a.app else '')
        + ('share/              desktop entry and icon for the nxconvert app\n'
           if system == 'linux' and a.app else '')
        + '\n'
        'Static linking: define NXFORMAT_STATIC and link nxformat_static with: '
        + (' '.join(STATIC_LIBS[system]) or 'nothing else') + '\n', newline='\n')

    if system == 'windows':
        archive = a.out / f'{name}.zip'
        with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as z:
            for f in sorted(stage.rglob('*')):
                if f.is_file():
                    z.write(f, f.relative_to(stage).as_posix())
    else:
        archive = a.out / f'{name}.tar.gz'
        with tarfile.open(archive, 'w:gz') as t:
            for f in sorted(stage.rglob('*')):
                if f.is_file():
                    info = t.gettarinfo(f, f.relative_to(stage).as_posix())
                    info.uid = info.gid = 0
                    info.uname = info.gname = ''
                    with open(f, 'rb') as data:
                        t.addfile(info, data)
    shutil.rmtree(stage)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    print(f'{digest}  {archive.name}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
