# XCDE multi-program NUP investigation

Recorded 2026-11-14 from the user's `XCDE_NUP_Troubleshooting_Handoff.txt`, a summary of testing with another agent. This is a research note for future importer work, not a successful-conversion report or a request to modify game files. Original uploads and the original NUP are not included in this repository.

## Outcome and preferred workflow

The user prefers a faithfully structured NUP that can be converted through the NXConvert GUI. An explicit extracted-tree manifest was proposed as an alternative, but no conversion output, verification output, exit code, or emulator launch result was supplied. **Neither successful conversion nor successful verification was established. No corrected, working NUP was produced.**

The original NUP's producer and version are unknown, and its bytes were not inspected in the prior investigation. NUP Extractor exposed `main_base`, `sub_prog`, and `dlc` directories. Two packages reportedly marked BASE are evidence of a representation/import limitation, not proof that the game's second program is invalid or that game data is corrupt.

## Metadata reported by the prior investigation

The other agent examined a directory listing, uploaded NPDM/CNMT metadata, and control directories. These binary findings have not been independently revalidated in this fork. The NPDM ZIP flattened two identical filenames; association with `main_base` and `sub_prog` was inferred from input order and should be confirmed at the original paths before implementation decisions.

| Item | Reported finding |
|---|---|
| Main NPDM | 1,572 bytes; ACI0 offset `0x450`; Program ID `0100FF500E34A000`; ACID magic at file offset `0x280`; profile `meta-acid2048-v1` |
| Auxiliary NPDM | 1,876 bytes; ACI0 offset `0x560`; Program ID `0400FF500E34A001`; ACID magic at file offset `0x380`; profile `meta-acid3072-v1` |
| Patch CNMT | Title `0100FF500E34A800`; application `0100FF500E34A000`; version `262144`; platform byte `1` (Switch2); Program/Control offsets 0 and 1 plus Legal |
| DLC CNMT | Title `0100FF500E34B001`; application `0100FF500E34A000`; version 0; platform byte 1; one Data record |
| Controls | Four 16,384-byte NACPs; display version 2.0.0; top-level title Xenoblade Chronicles: Definitive Edition; `Acd_1` title names the Nintendo Switch 2 Edition |

The main tree contained a large RomFS and executable files; the auxiliary tree contained executable files and only `romfs/dummy.bin` in its listed RomFS. Both had top-level and `Acd_1` control data. DLC had its own CNMT and substantial RomFS assets. The controls and CNMT support a shared application relationship across mixed executable IDs; they do not establish complete assets or runtime compatibility.

An earlier claim that the main ID was `0400FF500E34A000`, with both executables using the 3072 profile, contradicted the reported NPDM bytes. A second executable is not automatically an update. An explicit manifest declares relationships but does not bypass NPDM validation.

## Version-pinned format and importer findings

The handoff reviewed default-branch commits, not necessarily the installed 1.0.3 release binary:

- [NUP specification at a4840f0](https://github.com/a55uka/nup/blob/a4840f06c94ddeb414eea5b33c9704ffe995f11c/SPEC.md): the handoff reports that v1 permits at most one base package and acknowledges a multi-program limitation. The standalone-update flag does not establish a faithful auxiliary-program representation.
- [NXFormat NUP importer at 10ad8f5](https://github.com/fab918/nxformat/blob/10ad8f5ed15edebaf19850b6f33b76253e5380d0/packer/src/import/nup.rs): inspection and staging reject multiple bases; executable add-ons are rejected. These rejection paths were also confirmed in this fork after the upstream merge. The handoff reports that non-DLC executable staging declares primary roles rather than exposing the auxiliary mapping available to explicit manifests.
- [NPDM validation](https://github.com/fab918/nxformat/blob/10ad8f5ed15edebaf19850b6f33b76253e5380d0/packer/src/import/structures.rs) and [manifest documentation](https://github.com/fab918/nxformat/blob/10ad8f5ed15edebaf19850b6f33b76253e5380d0/docs/CLI.md) provide the relevant profile and role definitions.

The proposed manifest used one application, separate primary/auxiliary Program IDs and profiles, and a separate DLC snapshot. Its `standalone-update` / `complete` declarations assumed that extraction supplied complete materialized program views. Metadata alone did not prove that assumption. Metadata folders were reportedly moved out of snapshot roots, not deleted. Historical commands are deliberately not reproduced as a repair procedure.

## Implications for future changes

- Preserve both native Program IDs, both executables, their application relationship, and separate DLC. Do not patch NPDM IDs, merge DLC assets into the main RomFS, or relabel the auxiliary as an update/DLC merely to pass validation.
- Investigate explicit multi-program representation in the format and importer together. Merely relaxing the multiple-base rejection is insufficient when role, profile, and application association would be lost.
- Review platform detection for mixed `0100`/`0400` IDs; one ID prefix must not override authoritative metadata or force one NPDM profile on all programs.
- Future coverage should include two programs under one application, mixed NPDM profiles, separate DLC, faithful NUP extraction/repacking, GUI discovery/grouping, conversion integrity, and independently tested runtime program selection/DLC access.
- Before further troubleshooting, confirm the exact installed build, original producer, original-path NPDM associations, materialization completeness, and actual conversion/verification results. In the user's test setup, commands run one directory above `XCDE_Extracted` and invoke `nxconvert` from PATH.

No importer change or game-file repair is implemented by this note. These findings identify a known limitation in the inspected versions, not a claim that future or alternative implementations cannot support the package.
