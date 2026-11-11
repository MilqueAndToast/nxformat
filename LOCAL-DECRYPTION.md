# NXConvert — Local Decryption

A local fork of [fab918/nxformat](https://github.com/fab918/nxformat), retaining its GUI, game grouping, conversion queue, NX writer, and archive tools.

## Use

1. Start `nxconvert.exe` without arguments.
2. Click **Load keys…** and select your `prod.keys`. If `title.keys` exists beside it, it is also loaded. Files and key values are never uploaded or bundled. The selection lasts for this session.
3. Add your NSP/XCI files or source folders. Add matching base, updates, and DLC together; upstream groups them by application metadata.
4. Select the games, choose a separate output folder, and convert.

Loading keys after scanning automatically rescans the listed sources. A failed key load leaves the previous keys available. Keys cannot be changed while the queue is running.

```powershell
.\nxconvert.exe scan 'D:\Games' --keys 'D:\Keys\prod.keys'
.\nxconvert.exe pack 'D:\Games\base.nsp' 'D:\Games\update.nsp' 'D:\Games\dlc.nsp' 'D:\NX\game.nx' --keys 'D:\Keys\prod.keys'
.\nxconvert.exe pack 'D:\Games\game.xci' 'D:\NX\game.nx' --keys 'D:\Keys\prod.keys' --omit-firmware
.\nxconvert.exe verify 'D:\NX\game.nx'
```

Use `--title-keys PATH` to select a title-key file in another directory. This is the conventional `rights_id = encrypted_title_key` format (the title key is unwrapped with the matching titlekek). Personalized tickets require a recovered matching entry in this file; this fork does not recover console secrets or perform ticket RSA decryption. Common tickets can supply their encrypted title key directly. Content without tickets uses its key area when it has no rights ID; rights-managed content without a matching key is rejected.

## Streaming and originals

The reader opens input files read-only. It decrypts bounded header/table buffers during discovery and decrypts content ranges on demand during conversion. It does **not** write a full decrypted NSP/XCI temporary file. The upstream NX writer still uses its normal bounded staging/work files and atomic output publication.

Recipes are off by default when keys are loaded. **Keep decrypted rebuild data** / `--recipe` stores a recipe for the logical decrypted package view; it does **not** recreate the byte-identical encrypted original. Keep the originals separately. Gamecard header/signature/hash fields retained as opaque source data are not re-signed; a rebuilt decrypted view is an NXConvert interchange package, not an authentic cartridge image.

## Supported decryption layouts

- Switch NCA3 headers: AES-XTS.
- NCA content: AES-CTR, AES-CTR-EX patch tables, and corresponding skip-hash-layer modes.
- Common tickets, recovered `title.keys`, and rights-free key-area keys.
- Direct derived keys or derivation from matching master keys and standard key sources.
- PFS0 NSP and HFS0 XCI. Console firmware in an XCI's `update` partition remains opaque (or use `--omit-firmware`).

Unsupported layouts fail explicitly: sparse NCAs, NCA0/NCA2, AES-XTS content sections, NCZ/NSZ/XCZ. Switch 2 encrypted input is not claimed; upstream support for already decrypted sources remains available. Loading a key file does not guarantee its keys match a particular game. Cryptographic signatures are not verified; NXConvert's normal filesystem and NX integrity checks still apply.

## Build

Windows prerequisites: Git, Rust 1.95.0 MSVC toolchain, Visual Studio C++ build tools and Windows SDK.

```powershell
rustup toolchain install 1.95.0 --profile minimal
cargo +1.95.0 build --locked --release --manifest-path app/Cargo.toml
cargo +1.95.0 test --locked --manifest-path packer/Cargo.toml --lib
```

Executable: `app/target/release/nxconvert.exe`. A lightweight CLI-only test build is available with `cargo +1.95.0 build --locked --release --manifest-path packer/Cargo.toml --example nxconvert-cli`.

Alternatively, `./build-windows.ps1 -Test` runs the tests and creates the executable folder and ZIP in `dist-release/`. Omit `-Test` to build/package only. The source ZIP is a separate archive of the Git commit; it excludes build products, private test fixtures, keys, and games.

The Rust AES implementation is from RustCrypto (`aes` crate). Keys are kept in process memory; file text and key-map allocations use `zeroize`. No crash-dump or OS memory protection guarantees are made.

Validation and deliverable paths are recorded in [STATUS.md](STATUS.md).
