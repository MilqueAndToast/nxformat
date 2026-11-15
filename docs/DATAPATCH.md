# DataPatch support

DataPatch (`CNMT` type `0x84`) is a DLC update. Its patch title ID, target
`DataId`, owning `ApplicationId` and required application version are separate
fields. The parser retains them, including AddOnContent's optional forward
`DataPatchId` link. Layout reference: [libnx NCM types](https://github.com/switchbrew/libnx/blob/master/nx/include/switch/services/ncm_types.h).

| Input | Supported scope |
|---|---|
| Materialized NUP | Explicit compatibility mapping; complete file tree, package add-on ID matching native DataId, version from DataPatch CNMT. Switch 1/2 application rules still apply. |
| Native NSP/XCI | Switch 1 complete Data NCAs with a standalone IVFC RomFS. Existing local CTR decryption works with this path. |
| Raw BKTR/sparse DataPatch | Unsupported; rejected before conversion. Reconstructing these layouts, including any required base data, needs a separate implementation. |

Use **Generate NUP mapping…** / **Load NUP mapping…** for materialized sources,
or the normal native conversion flow for supported NSP/XCI. The existing NUP
spec warning and affirmative opt-in remain mandatory for detected violations.
No DataPatch record is renamed to an ordinary AddOnContent record. Native NX
keeps its actual Meta/Data NCAs and Content IDs; materialized NX stores a complete
updated DLC snapshot identified by target DataId and patch version, retaining
the original CNMT and package labels under `data/__nup_root__/` and
`data/__nup_mapping__.txt`.

Conversion checks application/target identity, Data content, any supplied
forward link and minimum application version. Native Data NCA ownership must
match either explicitly declared DataId or patch title; unrelated owners fail.
The target DLC always comes from CNMT DataId, including extracted folder/import
declarations. A matching supplied application snapshot satisfies required-version
dependencies through its exact identity. Incomplete raw patches are never
overlaid by guessing filenames or filling missing bytes.

Complete materialized views do not inherit files from the old DLC: this preserves
deletions as well as replacements. If both materialized base and patch would have
the same DataId/version, this mapping cannot represent them unambiguously and
rejects the source. It never silently drops either package. A complete standalone
DLC view may be converted without its old version; completeness remains a caller
declaration. Missing main-game content remains a separate blocker for full-game
NUP mapping.

Native filesystem extraction retains its existing single-Data-NCA limit; raw NCA
export preserves stored multi-Data views. With an application supplied, required
versions must be satisfied by accepted snapshots. Standalone native DLC archives
retain their requirements in CNMT and report the absent application.

Synthetic tests cover materialized and native conversion, minimum-version
dependencies, mixed Program profiles, encryption, strict verification, extraction,
exact NX restoration and package recipe replay. No real DataPatch conversion or
gameplay validation is claimed. See [TESTING.md](../TESTING.md).
