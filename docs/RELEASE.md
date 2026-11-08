# NXFormat 1.0.0

NXFormat 1.0.0 includes the NXConvert desktop application and CLI, the Rust
archive writer, the C API and the header-only C++17 reader.

## Features

- Convert supported already-decrypted DNSP/DXCI packages, extracted trees and
  NUP v1 archives into NX.
- Add updates and DLC, remove snapshots and compact unused space.
- Store new game-data blocks without compression using CLI `--no-compress`
  or the desktop **Non** preset. Remove existing game-data compression with
  `compact ARCHIVE.nx --no-compress`. Deduplication and integrity checks remain active.
- Read independent RAW/Zstandard blocks with per-slice CRC-32C checks; verify
  archive metadata, Media payloads and logical files with SHA-256.
- Keep supported native-package reconstruction recipes by default; exact
  rebuilding requires the recipe and all referenced file content.
- Embed commands through the C API, including callback-backed plaintext sources,
  progress reporting and cancellation.

NXConvert does not decrypt content or provide keys or game files. Emulator
compatibility depends on the emulator integration and the selected content.
This release is **NXFormat / NXConvert 1.0.0**, with separate 4096-byte header
pages. The exact binary header fields are defined in the format specification.

## Packages

| Package | Contents |
| --- | --- |
| Windows x64 | NXConvert app, shared/static libraries and headers |
| Linux x64 / arm64 | NXConvert app, shared/static libraries and headers |
| macOS universal | NXConvert app bundle, CLI, shared/static libraries and headers |
| Android arm64 | Shared/static libraries and headers, no desktop app |

Release archives include CMake package configuration. `SHA256SUMS` is published
as a separate attachment. `nxformat-xmake-1.0.0.zip` contains the xmake package
repository populated with those archive hashes. Extract it and register the
local directory with xmake. Publication adds no commits to the source repository.

Build NXConvert from source with Rust 1.95 or newer:

```bash
cargo build --release --locked --manifest-path app/Cargo.toml
```

The Rust writer and C API support Rust 1.80 or newer. The independent C++ reader
requires C++17 and a caller-supplied Zstandard decoder for compressed blocks.
See the [README](https://github.com/fab918/nxformat/blob/main/README.md), [CLI guide](https://github.com/fab918/nxformat/blob/main/docs/CLI.md), [reader guide](https://github.com/fab918/nxformat/blob/main/docs/READER.md)
and [format specification](https://github.com/fab918/nxformat/blob/main/docs/SPEC.md) for usage and integration.
