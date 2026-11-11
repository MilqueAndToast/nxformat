# NXConvert local decryption fork

Ready: implemented, built, and verified on Windows x64. No outstanding blocker for the documented supported scope.

- Branch: `codex/local-key-decryption`.
- Upstream base: `fab918/nxformat` commit `d40e62d978ee2cd1a1261415f59b5aa5302ccfe6` (1.0.2).
- Runnable build: `dist-release/NXConvert-Local-Decryption/nxconvert.exe`.
- Windows ZIP: `dist-release/NXConvert-Local-Decryption-Windows-x64.zip`.
- Source ZIP: `dist-release/NXConvert-Local-Decryption-source.zip` (Git-tracked source only).
- Build instructions: `LOCAL-DECRYPTION.md`; reproducible build/package script: `build-windows.ps1`.
- Full test matrix and limits: `TESTING.md`.
- EXE SHA-256: `bd8d229eab0df6bde3ae05a5e5c9d884d662170a35931bda9119e3dc1eaed3a4`.

Implemented: read-only streaming NCA3 AES-XTS header / AES-CTR / CTR-EX decryption, common-ticket and recovered-title.keys support, key-area/titlekek derivation, validated package ranges, GUI local-key selection with automatic rescan and decrypted artwork, CLI key options, explicit decrypted-view recipe semantics.

Validation: 92 packer tests and 9 GUI tests passed. The Windows junction fixture test was blocked by the restricted sandbox and passed when rerun outside it. All 59 local NSP/XCI packages were individually recognized. Wargroove base/update/DLC, Mystery Dungeon XCI with bundled update, and Legends Z-A base/newer update converted and passed strict NX verification. Recipe replay/rebuild, wrong/missing keys, GUI rescan/conversion/recipe selection, two-game queue and safe cancellation were checked. Four original test inputs retained identical SHA-256 hashes.

The GUI and workflow remain upstream NXConvert with a key selector and clarified labels. Conversion uses bounded decrypted reads, never a full decrypted temporary package. Recipes default off when keys are loaded and reconstruct the decrypted view if explicitly enabled.

Explicit unsupported scope: encrypted Switch 2 input, sparse NCA layouts, NCA0/NCA2, AES-XTS content sections, NCZ/NSZ/XCZ. Emulator gameplay has not been tested. Discovery warnings for conflicting duplicate Sword DLC and missing Scarlet/Tears of the Kingdom bases are documented in the test matrix.

Private local fixtures/outputs/logs are under `results/local-decryption/`, which is gitignored and excluded from both deliverables. User key values and game data are not tracked, packaged, or uploaded. The original game library and emulator installation were not modified by this work.
