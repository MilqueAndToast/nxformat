# Local-decryption validation

## Native NUP launch selection and unknown versions — 2026-11-15

Source builds distinguish an unknown authoritative application version from a
known native version of zero. Original package numeric values and raw DLC CNMT
minimums remain intact. Unresolved requirements are persisted as structured
`unresolved-requirement` provenance inside NX; conversion requires a per-file GUI
review or `--allow-unresolved-nup-requirements` in addition to any NUP conformance
consent. No satisfying version or exact dependency identity is fabricated.

285 packer tests passed (one intentionally ignored), plus 16 GUI tests. Added
coverage checks selected native NACP/TLV/control agreement, nonzero launch and
actual NPDM identity, absent accessibility/targets and conflicting controls;
unknown, known-zero, too-old and satisfying application versions; unchanged
source bytes; strict verification, full source comparison and extracted
unresolved provenance. Existing-map CLI and GUI paths require fresh explicit
review and carry acknowledgment into conversion. False dependency links and
display-derived/source-version overrides reject. Native raw-content behavior,
partial patch refusal and integrity requirements remain unchanged.
Paranoid comparison also reopens streamed raw/Zstd NUP files through their
SHA-256-checked reader; both ordinary and mapped DataPatch conversion paths are
covered. A real validation run exposed the former writer-only reader error;
failure published no final archive before this correction.

A real 30,973,597,620-byte NUP then converted to a 28,438,890,116-byte NX with
paranoid source comparison enabled. Strict archive verification passed. The
independent mapped check passed for all 41,676 original files across nine
snapshots, reading back 41,530 distinct NX files (31,057,834,386 bytes) and checking
the selected NACP. Original metadata/control files and compatibility provenance
were included. A separate metadata readback confirmed one unchanged nonzero
native launch Program, eight DLC snapshots, all eight original CNMT minimums,
structured unresolved rows in all nine snapshots and no fabricated dependencies.
The original was opened read-only; size, modification time and validated TOC
digest remained unchanged. Packaged Windows CLI warning/opt-in, mapping parity
and real-NX contents checks passed. Runtime compatibility remains unresolved;
no gameplay, real native NSP/XCI DataPatch conversion or new manual UI validation
is claimed.

## DataPatch support — initial 1.0.3.3 validation, 2026-11-15

Unreleased source adds the [documented complete-view subset](docs/DATAPATCH.md).
The shared CNMT parser decodes target DataId, owning application, patch title,
required application version and AddOnContent's forward DataPatchId separately.
Native snapshots preserve actual Meta/Data NCAs and Content IDs; materialized
snapshots preserve original metadata while identifying the updated target DLC.

Validation: 281 packer tests passed (one intentionally ignored), plus 15 GUI
tests. Synthetic coverage includes:

- Full materialized NUP conversion in staged and streamed modes; separate old and
  updated DLC, preserved patch metadata, deletions without filename overlay,
  exact primary-Program minimum-version dependencies, explicit nonzero launch
  Program mapping without ID rewriting, strict NX verification,
  complete source comparison and extraction.
- Complete native DataPatch discovery, planning, conversion and addition through
  normal CLI/GUI flows; exact application/target metadata, standalone IVFC Data
  NCAs, extraction under the target DLC ID, exact NX restoration and native
  package recipe replay. Real gameplay is not implied by these artificial files.
- Artificial AES-CTR input through the production decrypted-source adapter,
  paranoid conversion, strict verification and decrypted recipe replay. Both
  declared target/patch NCA-owner forms are covered; no console keys are used.
- Truncated headers, zero/self/unrelated targets, conflicting forward links,
  unmet application versions, ambiguous materialized version scopes, wrong NCA
  owners, corrupt IVFC, missing Data content and raw BKTR refusal. Sparse
  DataPatch remains untested and is rejected by the same storage guard.

An in-write dependency bug found by the multi-program case was corrected:
pending dependencies now include the exact Program ID, avoiding selection of an
auxiliary snapshot with the same type/version as the primary. This changes no
NX wire format. GUI discovery now includes supported native DataPatch content.

The current real NUP containing a DataPatch was reread and its TOC hash verified.
Generation recognizes DataPatch but cannot infer its launch role from the
index-zero convention. The earlier claim that absence of index zero proves a
missing main executable was too categorical: NX roles are independent of index,
and an explicit nonzero-index primary is valid. A private explicit mapping then
passed launch identity/platform checks but failed the DLC minimum-version check:
one add-on requires application version 983040, while the only executable package
is labelled version 0 and has no executable CNMT establishing a different version.
This does not establish missing game bytes. No version was invented, no accepted
mapping/output was produced, and no real DataPatch conversion, gameplay or new
manual UI validation is claimed.
The prior spec warning/affirmative opt-in remains in force. Raw BKTR/sparse
DataPatch reconstruction, native encrypted Switch 2 input and additional native
layouts remain unsupported. Originals were read-only. Private evidence and logs
are under ignored `results/multi-program-nup/`.

## Spec warnings and compatibility corrections — 2026-11-14

This unreleased follow-up supersedes the earlier blanket mixed-platform refusal.
One real multi-program source converted to NX with both native executables and
two separate DLC snapshots, including legacy-platform AddOnContent owned by the
same application. Input: 15,657,028,673 bytes; NX: 15,379,680,788 bytes. Strict NX
verification passed. Mapped source comparison passed all 180 source files,
146 distinct NX file readbacks (15,679,341,355 logical bytes), and one NACP.
Source bytes were opened read-only; original CNMT platform markers are preserved.
Completeness is declared; no gameplay, executable selection or DLC access test
was performed. This demonstrates NX structure/integrity, not NUP-v1 conformance:
the original has multiple BASE records and noncanonical extra roots. The
[compatibility guide](docs/NUP-COMPATIBILITY.md) cites the applicable rules.

A second real source has only an auxiliary executable in its package table and
contains native DataPatch metadata. That is unsupported patch composition and
a missing main-program blocker for this workflow, not proof of corrupt or
unrelated DLC. The previous parser's generic application fallback for DataPatch
was not valid ownership evidence. The generator now reports these actual
blockers rather than a misleading application/platform conflict. It neither
drops patches nor relabels the auxiliary executable.

All 274 packer tests passed (one intentionally ignored) and all 14 GUI tests
passed. Coverage includes legacy DLC conversion/source comparison; unrelated,
unsupported or executable platform conflicts; file-specific visible errors;
save-dialog cancellation/unavailability; invalid mapping loads; lost workers;
warning-before-generation; Cancel; stale TOC acknowledgment; existing-mapping
bypass rejection; CLI acknowledgment for generation/conversion/checking; and
TOC corruption remaining fatal with consent. Formatting and whitespace checks
passed. Windows build and final CLI smoke results are recorded locally.

Manual native picker verification was blocked by the computer-use helper's
invalid window geometry. UI input was stopped, the test app was closed and the
computer-use session was explicitly ended. Visible-card behavior is covered by
automated application-state tests; no manual picker/visual pass is claimed.
Private evidence remains under ignored `results/multi-program-nup/`.

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
