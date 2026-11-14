# Local-decryption validation

## Multi-program NUP compatibility — 2026-11-14

### Mapping generation follow-up

`map-nup --assume-complete` and GUI **Generate NUP mapping…** now derive a
TOC-bound declaration from embedded NPDM/CNMT. The full packer suite passed
270 tests (one intentionally ignored), and all 13 GUI tests passed. Added coverage checks generated mixed
profiles, roles, per-program update dependencies and DLC through conversion,
strict verification and source comparison; explicit completeness opt-in;
no overwrite; missing/ambiguous/conflicting metadata; cancellation; size limits;
and unique, ambiguous or malformed controls. GUI generation-job coverage checks
automatic loading, conversion and rejection of an existing output file.

Real XCDE mapping generation and validation passed, preserving the manual
mapping's application/platform/program/profile/version relationships. With
multiple edition controls present, generation intentionally leaves display
selection to package metadata; it does not reproduce the manual artwork choice.
The generated mapping was not used for another full real-game conversion;
the manual mapping's conversion evidence below remains separate.

Real XC2 generation was rejected for conflicting CNMT application/platform
pairs (`0100E95004038000` / Switch1 and `0100E95004039862` / Switch2). Real XCX
generation was rejected for both Switch1 and Switch2 CNMT markers under
`0100453019AA8000`. Neither container was altered or declared corrupt. These
conflicts need separate investigation and cannot be overridden by this mapping
workflow. No gameplay validation or manual native file-picker test was performed.

### Initial explicit mapping implementation

The explicit [NUP mapping](docs/NUP-COMPATIBILITY.md) was checked on Windows x64:

- Packer regression suite: 267 passed, one intentionally ignored; doctest run passed (no doctests). The Windows junction fixture ran outside the restricted sandbox.
- GUI regression suite: 12 passed. Automated coverage includes rejected discovery, explicit mapping load/replacement/removal, rescan retention, application grouping and conversion using the GUI-generated command. Native file-picker interaction was not manually tested.
- Synthetic coverage: mixed native Program IDs and ACID-2048/3072 profiles, primary/auxiliary roles, matching-program updates, separate DLC, streaming and fully staged conversion, strict verification, source comparison, extraction of extras/provenance and unchanged original input bytes.
- Rejection coverage: stale TOC binding (including replacement after discovery), duplicate package roots, wrong profiles/IDs, ambiguous roles, cross-program dependencies, missing/extra/unrelated CNMT program offsets, duplicate offsets, platform/version/kind contradictions and corrupt compressed payloads.

The original XCDE NUP (32,772,783,634 bytes) converted to a 32,597,862,072-byte
NX and passed strict archive verification. The mapped source comparison passed
for all 37,718 source files across three snapshots, reading back 31,681 distinct
NX file block ranges (33,809,939,534 logical bytes) and checking two NACPs. It also
verified program roles/platform and preserved extras/provenance. The Windows
release build succeeded; its CLI help and real-NX contents commands passed smoke
checks. It retains application
`0100FF500E34A000` on Switch2, primary program `0100FF500E34A000` (ACID-2048),
auxiliary program `0400FF500E34A001` (ACID-3072), both declared as complete
standalone updates at version 262144, and separate DLC `0100FF500E34B001` v0.
The original BASE-v0 container labels are retained as provenance. Completeness
is declared, not established by NPDM/CNMT consistency or successful conversion.
No emulator boot, program selection, DLC access or gameplay validation was done.
The real source was opened read-only; no keys were needed. Private fixtures,
mapping and logs remain under ignored `results/multi-program-nup/`.

## Earlier local-decryption validation

For the earlier XCDE/NUP investigation and its follow-up, see [XCDE NUP findings](docs/XCDE-NUP-FINDINGS.md). Current multi-program validation is recorded below separately from the earlier decryption tests.

Validated on Windows x64 with Rust 1.95.0, using the owner's existing local key files and game dumps. No key values or game data are in the source or Windows distribution.

| Check | Result |
|---|---|
| Packer regression suite | 99 tests passed across the suite and added focused test; the Windows junction fixture test required execution outside the restricted sandbox |
| GUI regression suite | 9 tests passed |
| Discovery of the local NSP/XCI library | All 59 packages recognized individually |
| Wargroove NSP base + update + DLC | Converted together with paranoid source comparison; strict NX verification passed; all three snapshots present |
| Mystery Dungeon XCI, including bundled game update | Converted with firmware omitted; strict NX verification passed; base and update snapshots present |
| Legends Z-A base + newer update | Converted after handling the newer CTR-EX metadata boundary; strict NX verification passed |
| Original preservation | SHA-256 before/after matched for Wargroove's three source files and the Mystery Dungeon XCI |
| Embedded common tickets without title.keys | Wargroove base/update/DLC discovery passed using an explicitly empty title-key file |
| Personalized-ticket fixture | A private copy of a small DLC with its original personalized ticket loaded with the recovered title.keys entry; rejected with an actionable missing-title-key error without it |
| Wrong header key | Conversion failed and published no final NX file |
| Recipes | All three Wargroove recipes passed `check-recipe`; rebuilt packages loaded without keys as decrypted sources |
| GUI missing keys, then loading keys | Initial rejection became a recognized Wargroove source automatically; title and artwork displayed correctly |
| GUI conversion with recipe checkbox enabled | Created a valid NX with one embedded recipe, verified by CLI |
| GUI two-game queue and cancellation | Both XCIs grouped with artwork; active job and one queued job shown; elapsed time and spinner continued; cancel returned to the source list and published no output or leftover staging file |
| Cryptography regression tests | NIST AES vector and independent OpenSSL XTS/unaligned CTR vectors passed |
| Patch-reader regression tests | Legacy/new table boundaries, plaintext regions, odd read boundaries, and malformed coverage passed |
| Sparse reader fixtures (synthetic) | Mapped/plain/CTR/CTR-EX/skip-hash content; logical counters; holes; empty and multiple entry sets; patch metadata; growth/shrinkage in NSP and nested HFS0; 15 malformed cases and out-of-bounds reads passed |
| Sparse conversion (synthetic) | A sparse base with an absent executable and an encrypted mapping table for its second section, plus its matching update, converted with paranoid source comparison; strict NX verification and both embedded recipes passed |
| Sparse release regressions (real, non-sparse sources) | The same 59-package scan and Wargroove, Mystery Dungeon, and Legends Z-A conversions were repeated after introducing the mapped source reader |

Representative conversion results (level 1, two workers; hardware/cache-dependent):

- Wargroove: 437,798,768 input bytes → 428,197,052 NX bytes, 1.95 s conversion.
- Mystery Dungeon: 3,992,977,408 input bytes → 2,190,648,092 NX bytes, 16.43 s conversion.
- Legends Z-A: 6,448,023,705 input bytes → 6,385,213,748 NX bytes, 50.37 s conversion.

The tests verify archive structure, integrity, metadata, and reconstruction. Gameplay/emulator boot testing was not performed. Recognized sources are not a promise that every combination can be converted: the full library scan also reported two conflicting Sword DLC duplicates (standalone versus included in the XCI), plus Scarlet and Tears of the Kingdom updates with no matching base in this library. Select one copy of each conflicting DLC and supply missing bases as appropriate.

Not claimed/tested as supported: encrypted Switch 2 content, NCA0/NCA2, AES-XTS content sections, NCZ/NSZ/XCZ, sparse trees beyond the documented limits. Sparse validation uses generated fixtures only; no real sparse game dump was available. Skip-layer-hash modes have isolated synthetic reader coverage, but no separate real-game end-to-end fixture. Nintendo signatures are not verified. Rebuild recipes refer to the decrypted view (including resized sparse sections) and retain package extras such as ticket/certificate records; they do not restore the original encrypted bytes.

No full decrypted intermediate is created by conversion: the production source adapter opens files read-only, holds bounded header/table data, and applies AES to requested read ranges. Rebuilt NSPs used in recipe tests were created only by the explicitly invoked rebuild command.
