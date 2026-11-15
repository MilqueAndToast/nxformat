# Explicit multi-program NUP import

Some NUP producers label multiple executable packages as BASE. This fork can
import them through an explicit compatibility mapping, preserving each native
Program ID and NPDM profile under one application, with primary/auxiliary roles,
per-program updates and separate DLC. Ordinary NUP v1 validation is unchanged.
Available in Local Decryption 1.0.3.1 and newer.

NUP 1.0-draft [§3.1.2](https://github.com/a55uka/nup/blob/a4840f06c94ddeb414eea5b33c9704ffe995f11c/SPEC.md#312-package-kinds)
allows at most one BASE. A source with multiple BASE records is nonconforming
NUP v1, even when its decoded native contents are coherent. Explicit mapping
interprets those contents for compatibility; it does not certify or repair NUP
v1 conformance. The resulting NX must satisfy its specification and integrity
checks.

## Use

In the GUI, add the NUP and choose **Generate NUP mapping** when its source card
requires one, or **Generate NUP mappings for all** for the queue. Ordinary
strict-valid NUPs convert directly, including those without CNMT metadata.
Each affected file has its own warning and completeness review. **Cancel** is
the safe default; **Continue for this file** applies only to that source.
The batch continues after individual errors and reports generated, skipped,
failed and cancelled counts. Unchanged valid mappings are reused.

Successful generation shows **NUP mapping generated**. The app manages its
temporary mapping for the session; there is no save dialog. Queued conversions
retain it until their readers finish. Owned files are cleaned after conversion,
removal, clear, failed conversion or normal exit; retry generates a fresh map.
Cancelling a batch keeps earlier successful maps available for queued sources.
**Load mapping (advanced)…** selects an existing reviewed mapping instead;
imported mapping files and source NUPs are never deleted by session cleanup.
The CLI below still supports explicitly saved mapping files.
Existing mappings go through the same warning; their existence does not supply
consent. If the source TOC changes after review, review it again. Acknowledgment
lasts for the loaded source in this app session.
**Remove mapping** restores strict discovery. Generic rescans retain an active
mapping for an unchanged source; adjacent sidecars are never loaded automatically. Conversion rechecks
the selected file and source rather than trusting discovery results.

```text
nxconvert map-nup game.nup game.nxnupmap --assume-complete --allow-nonconforming-nup
nxconvert pack-nup game.nup game.nx --platform switch2 --nup-map game.nxnupmap --allow-nonconforming-nup
nxconvert verify game.nx
nxconvert check-nup game.nup game.nx --nup-map game.nxnupmap --allow-nonconforming-nup
nxconvert extract game.nx extracted --no-restore
```

Omit `--allow-nonconforming-nup` initially to read the detected violations; a
nonconforming input is rejected before generating, applying or converting with a
mapping. The flag explicitly acknowledges those violations for that command and
prints them again. Conforming sources need no such flag. Preflight identifies
multiple BASE records, noncanonical roots, standalone-flag inconsistencies,
executable-CNMT numeric-version mismatches and reserved listing artifacts; it is not an exhaustive conformance certificate.
Structural or integrity failures remain hard errors. Consent cannot override
unsupported patch composition, missing required content, incoherent identities
or NX output requirements. Strict ordinary import remains the default.

Generation reads embedded NPDM and `meta/*.cnmt` without decrypting or rewriting
the NUP. It requires one consistent executable application/platform and an unambiguous
CNMT relationship for each package. A package's own CNMT takes precedence;
otherwise a unique matching sibling CNMT is required. Native program index zero
is assigned the primary role; other indices are auxiliary. Matching earlier
bases become update dependencies; a Patch without a matching base is declared
standalone. These conventions and completeness remain explicit assumptions.

Missing/conflicting metadata stops generation with an error. Existing output
files are never overwritten. A unique control NACP is selected automatically;
multiple edition controls are preserved but not guessed between (display falls
back to package metadata). Generation validates the mapping before saving it.
This does not repair conflicting CNMTs or establish game completeness.

Source builds also support a narrow native-control fallback for one executable:
the selected NACP TLV must match its original control file, and its initial
program index, program index, accessibility flags, application and platform must
agree with the actual NPDM and native CNMT markers. This can select a nonzero
launch program and the corresponding `Acd_*` control without renaming IDs or
guessing from package order. Conflicting or absent targets reject. Field layout:
[libnx NACP](https://github.com/switchbrew/libnx/blob/master/nx/include/switch/nacp.h).

Automatic generation uses executable Application/Patch CNMT for platform. A
reviewed explicit mapping can also bind its declared application platform to a
validated DataPatch marker when executable CNMT is unavailable; launch role and
program version remain explicit declarations. Original
Switch 1 AddOnContent may accompany Switch 2 programs when its exact application
and DLC title/version match and it has no executable content. Its original
platform marker and bytes remain preserved. Executable platform conflicts,
unrelated DLC and newer-platform DLC attached to older-platform programs still
fail validation. Absence of program index zero does not prove missing game data:
generation requires validated launch metadata or a reviewed explicit mapping
naming an existing native launch Program. Complete materialized DataPatch views are
supported; raw base-dependent patch reconstruction remains
unsupported. [DataPatch scope](DATAPATCH.md).

This representation uses the existing [NX catalog rules](SPEC.md#4-fast-catalog):
platform belongs to the application, native executable IDs belong to Program
rows, and add-on-title DLC has no invented executable association. The importer
checks the application execution platform against validated native metadata;
the NX format itself does not guarantee runtime compatibility of older DLC.
Native DataPatch is a documented [CNMT type](https://switchbrew.org/wiki/CNMT#DataPatchMetaExtendedHeader),
not evidence of corruption. Its own CNMT type and patch identity are preserved;
the complete updated DLC view uses its native target DataId and patch version.
It cannot be relabelled as AddOnContent. A mapping declaring only auxiliary
programs cannot meet NX's one-primary requirement; numeric index alone does not
determine that role.

Generation errors appear on the affected source card with the reason and next
step, including temporary-storage failures, invalid mapping loads and unexpected
background-task termination. CLI mapping output files are never overwritten;
choose a new filename when saving again.

## Mapping format and checks

The UTF-8 file starts with `NXNUPMAP<TAB>1<TAB><lowercase TOC SHA-256>` and a
newline, followed by an ordinary `NXIMPORT` version 1 declaration. Use actual
tabs; the angle-bracket text here is explanatory. The hash is the validated NUP
superblock TOC digest, not a whole-file hash or an authentication signature.
See [the declaration grammar](CLI.md#extracted-tree-declarations) in CLI.md.

Declare exactly one snapshot per source package, using `package-0`, `package-1`,
etc. as roots. All declarations must share one application/platform and use
complete `materialized-v1` views. Each executable needs its actual NPDM Program
ID, matching NPDM profile and explicit primary/auxiliary role. Exactly one
Program ID may be primary. Updates require an earlier base of that same Program
ID through `dependency<TAB>update-key<TAB>required-base<TAB>base-key`.
Ordinary DLC retains its source title/version and has no executable Program.
DataPatch uses its native target DataId and patch version; differing NUP version
labels are reported by preflight and retained in provenance. A nonzero required
application version needs a `requires` dependency on a matching primary snapshot.

When executable CNMT is absent, a stored package version is retained as a source
value, not treated as a proven native application version. A missing authoritative
version differs from a known version of zero. Source builds retain DLC minimums
as **unresolved** rather than invent a satisfying version or dependency. Review
the per-file GUI warning before continuing, or explicitly pass
`--allow-unresolved-nup-requirements` to `map-nup`, mapped `pack-nup` and
`check-nup`. This acknowledgment is separate from NUP conformance consent and is
required even with an existing mapping. Known native version conflicts still
reject. User-supplied dependency/version declarations cannot bypass these checks.
Runtime compatibility remains unverified. Raw CNMT stays intact, and structured
`unresolved-requirement` rows (package index, CNMT title, application, minimum,
reason) persist in every `data/__nup_mapping__.txt` alongside original labels.

Optional `nacp` and `icon` records select original `control/` paths, including
subdirectories. For example, `nacp<TAB>main<TAB>control/Acd_1/control.nacp`
selects that file for display without removing other control data.

Embedded CNMTs in the package's `meta/` tree must agree with application,
version and snapshot kind and satisfy the platform rules above. Their Program content offsets must match
the declared program set; mixed generation prefixes are retained, not rewritten.
Duplicate offsets, unrelated IDs, ambiguous roles, stale hashes, incorrect
profiles and cross-program update dependencies are rejected. Composition,
exclusions and replacement identities are unsupported in this mapping.

## Preservation and limits

The source is opened read-only. ExeFS, RomFS and data retain their namespaces.
Other package roots (control, logo, legal, meta, etc.) are preserved byte-for-byte
under each snapshot's `data/__nup_root__/`. Original package labels, TOC hash and
declarations are retained at `data/__nup_mapping__.txt`. These reserved paths
must not already be occupied. Normal NX extraction exports both locations under
each snapshot root listed in `extracted.nximport`; it does not recreate a
byte-identical NUP container. DLC is never merged into a program's RomFS.

`check-nup --nup-map` checks mapped relationships, NACP, source file paths and
decoded file hashes against NX readback, including preserved extras and mapping
provenance. It does not authenticate Nintendo signatures or compare every
display-metadata TLV. Completeness and program roles are declarations checked for
consistency, not proof that missing assets can be recovered or a game will boot.
Only the documented materialized layouts are supported; arbitrary malformed
NUPs are not repaired. See [test evidence](../TESTING.md).
