# NXConvert local decryption fork

Ready: implemented, built, and verified on Windows x64. No outstanding blocker for the documented supported scope.

- Branch: `codex/local-key-decryption`.
- Upstream base: `fab918/nxformat` commit `d40e62d978ee2cd1a1261415f59b5aa5302ccfe6` (1.0.2).
- Runnable build: `dist-release/NXConvert-Local-Decryption/nxconvert.exe`.
- Windows ZIP: `dist-release/NXConvert-Local-Decryption-Windows-x64.zip`.
- Source ZIP: `dist-release/NXConvert-Local-Decryption-source.zip` (Git-tracked source only).
- Build instructions: `LOCAL-DECRYPTION.md`; reproducible build/package script: `build-windows.ps1`.
- Full test matrix and limits: `TESTING.md`.
- Current executable SHA-256 and exact source revision: `dist-release/NXConvert-Local-Decryption/SHA256SUMS.txt` and `BUILD.txt`.
- Published checkpoint: [Preview 1](https://github.com/MilqueAndToast/nxformat/releases/tag/v1.0.2-local-decryption.1), commit `265fbd07217b3339393de953b242c233f5c81853`. Its public EXE SHA-256 is `4723a243acb5fec9b2d3a940231286bc85ff2d38e1d66f79447f0065993641ad`. That release is unchanged and predates sparse support.

Implemented: read-only streaming NCA3 AES-XTS header / AES-CTR / CTR-EX decryption, common-ticket and recovered-title.keys support, key-area/titlekek derivation, validated package ranges, GUI local-key selection with automatic rescan and decrypted artwork, CLI key options, explicit decrypted-view recipe semantics. Sparse support adds encrypted mapping-table parsing, logical content counters, zero-filled holes, adjusted NSP/HFS0 sizes and offsets, and the upstream flattened-sparse marker. Public builds remap compiler paths.

Validation: 99 packer tests and 9 GUI tests passed. The Windows junction fixture test was blocked by the restricted sandbox and passed when rerun outside it. Sparse synthetic fixtures cover holes, mappings, multiple entry sets, CTR/CTR-EX/skip-hash modes, patch metadata, resized NSP/nested HFS0, malformed inputs, and an actual sparse base/update conversion followed by strict NX verification and recipe replay. No real sparse game fixture was available. All 59 local NSP/XCI packages were individually recognized. Wargroove base/update/DLC, Mystery Dungeon XCI with bundled update, and Legends Z-A base/newer update converted and passed strict NX verification before and after the sparse change. Earlier recipe replay/rebuild, wrong/missing keys, GUI rescan/conversion/recipe selection, two-game queue and safe cancellation were checked; four original test inputs retained identical SHA-256 hashes.

The GUI and workflow remain upstream NXConvert with a key selector and clarified labels. Conversion uses bounded decrypted reads, never a full decrypted temporary package. Recipes default off when keys are loaded and reconstruct the decrypted view if explicitly enabled.

Explicit unsupported scope: encrypted Switch 2 input, NCA0/NCA2, AES-XTS content sections, NCZ/NSZ/XCZ, sparse trees above 1,000,000 entries or 64 MiB per-section table data. Sparse support cannot recover absent data; some bases need their matching update. Emulator gameplay has not been tested. Discovery warnings for conflicting duplicate Sword DLC and missing Scarlet/Tears of the Kingdom bases are documented in the test matrix.

Private local fixtures/outputs/logs are under `results/local-decryption/`, which is gitignored and excluded from both deliverables. User key values and game data are not tracked, packaged, or uploaded. The original game library and emulator installation were not modified by this work.

## Upstream sync — 2026-11-14

Merged upstream main through `10ad8f5` (NXConvert 1.0.3, native extraction/restoration and recipe-speed improvements). Local-key decryption, sparse NCA mapping, key-aware artwork and explicit decrypted recipe semantics are retained. README links continue to point to the published 1.0.2.2 release; the merged source and locally built `app/target/release/nxconvert.exe` report 1.0.3. No new release was published by this merge.

Validation: full locked packer suite passed (263 tests, one intentionally ignored), 11 GUI tests passed, packer formatting and patch whitespace checks passed, and the Windows release build succeeded. A real encrypted Wargroove NSP converted with paranoid source comparison, passed strict NX verification, extracted with a required restoration recipe, and restored to an NX with an identical SHA-256. The full suite was run outside the sandbox to allow its Windows junction fixture. Test logs and private conversion outputs remain under ignored `results/local-decryption/`.

## Explicit multi-program NUP import — 2026-11-14

Implemented an opt-in, TOC-bound `.nxnupmap` through CLI `--nup-map` and GUI
**Load NUP mapping…**. Strict ordinary NUP validation remains the default.
Mapping validation retains native Program IDs/profiles, stable primary/auxiliary
roles, application/platform/version relationships, same-program update bases
and separate DLC. Embedded CNMTs constrain the mapping; conversion revalidates
it. Extra roots and original labels are preserved in explicit data namespaces.
See [usage and limitations](docs/NUP-COMPATIBILITY.md).

Validation: 267 packer tests passed (one intentionally ignored), 12 GUI tests
passed, formatting/whitespace checks passed, and the Windows build and CLI smoke
checks succeeded. Synthetic cases cover valid mappings, ambiguity/corruption
rejection, conversion and extraction. Real XCDE converted, passed strict NX
verification and mapped comparison of all 37,718 source files plus provenance
and two NACPs. The real source was opened read-only. Completeness remains a
declaration; no emulator boot, program selection, DLC access or gameplay was
tested. No new release was published or existing release asset replaced.

Private evidence and the reviewed XCDE mapping: `results/multi-program-nup/`.
Local Windows package: `dist-release/NXConvert-Local-Decryption-Windows-x64.zip`.

### Mapping generation follow-up

Added GUI **Generate NUP mapping…** and CLI `map-nup --assume-complete`.
Generation derives relationships from embedded NPDM/CNMT, validates the complete
mapping before saving, refuses existing output paths, and loads successful GUI
results automatically. Index-zero primary assignment and materialization
completeness are disclosed assumptions. Missing/ambiguous metadata and conflicting
applications/platforms are rejected; edition-specific controls are not guessed.
The source NUP stays read-only.

The packer suite passed 270 tests (one ignored) and all 13 GUI tests passed, including generation through
conversion/source verification and refusal cases. Real XCDE generation passed;
XC2/XCX generation correctly refused conflicting CNMT relationships. Real generated
mapping conversion and gameplay were not tested; earlier manual-mapping evidence
remains separate. See [TESTING.md](TESTING.md) for exact scope.

## Release 1.0.3.1 — 2026-11-14

Full release of the upstream 1.0.3 merge, multi-program NUP compatibility, and checked GUI/CLI mapping generation. Includes the verified implementation through `fc9bc2c`. Latest validation: 270 packer tests and 13 GUI tests passed (one intentionally ignored packer test); the packaged CLI generated the XCDE mapping. The preceding compatibility implementation verified all 37,718 XCDE source files after conversion. No additional full real conversion or gameplay claim is made for the generation change.

## Spec warnings and error feedback — 2026-11-14

Unreleased changes add a per-file preflight before generating or loading a mapping.
Detected NUP-v1 violations are shown with Cancel as the default and explicit
Continue in compatibility mode. Existing mappings cannot bypass review; a changed
TOC invalidates the acknowledgment. CLI mapping generation, mapped conversion
and comparison require --allow-nonconforming-nup after reporting violations.
Consent does not override missing required content, unsupported composition,
identity conflicts, integrity failures or NX output requirements. Strict defaults
remain intact.

Corrected the legacy-DLC case by deriving application execution platform from
executable CNMT, retaining matching older nonexecutable DLC with its native bytes
and marker unchanged. A real source converted to four snapshots, passed strict NX
verification and full comparison of 180 source files. The original multiple-BASE
source is nonconforming NUP v1; explicit compatibility is not a conformance claim.
A second source remains blocked by missing main executable and unimplemented
DataPatch composition, rather than the previously reported identity conflict.

Generation/load/save/worker failures now remain visible on the affected source
card with a filename, explanation and next step. 274 packer tests passed (one
ignored), plus 14 GUI tests. Native picker verification was blocked by invalid
computer-use window geometry; automated behavior is tested. No gameplay claim.
See TESTING.md and docs/NUP-COMPATIBILITY.md for precise evidence and rules.
Changes are local pending PM review; no source push or release is authorized yet.

## DataPatch complete-view support — 2026-11-15

Unreleased source supports complete materialized NUP DataPatch views and complete
Switch 1 native Data NCAs, with typed target DataId/application/patch identities,
minimum-version requirements and validated forward links. NUP mapping retains
original labels/CNMT, uses native patch version and target DLC ID, and preserves
full updated views without guessing overlays. Native conversion preserves raw
Meta/Data NCAs; extraction identifies DLC by CNMT DataId, not patch title.

Corrected same-write dependency resolution to include Program ID, so DLC requiring
an exact primary snapshot cannot attach to an auxiliary of the same version.
GUI native discovery includes supported DataPatch records. No NX wire change.

280 packer tests (one ignored) and 15 GUI tests passed; coverage includes native
and materialized conversion, artificial CTR decryption, source comparison,
strict verification, extraction, exact restoration/recipe replay and rejection
of inconsistent/incomplete inputs. See TESTING.md and docs/DATAPATCH.md.
Raw BKTR/sparse DataPatch reconstruction remains unsupported and is reported
explicitly. No real DataPatch conversion/gameplay or new manual UI pass claimed.
The current real NUP is recognized past DataPatch parsing but still lacks its
main executable. No source content is fabricated or dropped. Existing NUP spec
warnings and opt-in remain mandatory. No source push or release authorized yet.
