# Explicit multi-program NUP import

Some NUP producers label multiple executable packages as BASE. This fork can
import them through an explicit compatibility mapping, preserving each native
Program ID and NPDM profile under one application, with primary/auxiliary roles,
per-program updates and separate DLC. Ordinary NUP v1 validation is unchanged.
This is a source-build feature; the existing 1.0.2.2 release does not include it.

## Use

In the GUI, add the NUP and choose **Generate NUP mapping…** on its source card
(including a rejected source). Read the completeness assumption, choose
**Assume complete and generate…**, and save a new `.nxnupmap` file. A successful
mapping is loaded automatically; then convert. **Load NUP mapping…** selects an
existing reviewed mapping instead.
**Remove mapping** restores strict discovery. Generic rescans retain an active
mapping; adjacent sidecars are never loaded automatically. Conversion rechecks
the selected file and source rather than trusting discovery results.

```text
nxconvert map-nup game.nup game.nxnupmap --assume-complete
nxconvert pack-nup game.nup game.nx --platform switch2 --nup-map game.nxnupmap
nxconvert verify game.nx
nxconvert check-nup game.nup game.nx --nup-map game.nxnupmap
nxconvert extract game.nx extracted --no-restore
```

Generation reads embedded NPDM and `meta/*.cnmt` without decrypting or rewriting
the NUP. It requires one consistent application/platform and an unambiguous
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
DLC must retain its source title/version and have no executable Program.

Optional `nacp` and `icon` records select original `control/` paths, including
subdirectories. For example, `nacp<TAB>main<TAB>control/Acd_1/control.nacp`
selects that file for display without removing other control data.

Embedded CNMTs in the package's `meta/` tree must agree with application,
platform, version and snapshot kind. Their Program content offsets must match
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
