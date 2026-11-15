# Fork release versions

Fork releases use the upstream release version incorporated into their codebase,
followed by one lowercase letter: `1.0.3a`, `1.0.3b`, through `1.0.3z`.
Git tags use a `v` prefix, for example `v1.0.3e`.

Increment the letter only after a proper public release on that upstream base.
Previews use the upcoming proper release's letter plus `-previewN`, starting
at 1: `1.0.3f-preview1`, `1.0.3f-preview2`, then `1.0.3f`.
After that proper release, the next preview is `1.0.3g-preview1`.
The preview counter resets after each proper release. Previews do not consume
letters. A proper release can also be published without preceding previews.
After merging a newer upstream release, use its version and restart at `a`.
An upstream announcement alone does not change the fork's version: verify the
released commit's ancestry. Do not reuse a letter or invent a scheme beyond `z`;
agree on an extension if that limit is reached. A new upstream release base
also starts its preview sequence at `a-preview1`, if previews are needed.

Use the same version in the GitHub title, canonical tag, asset filenames,
checksum entries, README badge and download links. Publish normal releases as
proper releases, with the newest qualified release marked Latest. Keep notes
brief, understandable to nontechnical readers, limited to changes since the
previous release, and free of game names or abbreviations.

## Historical name migration

| Previous name | Canonical version |
| --- | --- |
| Preview 1 (`v1.0.2-local-decryption.1`) | `1.0.2a-preview1` (hidden draft) |
| 1.0.2.2 | `1.0.2a` |
| 1.0.3.1 | `1.0.3a` |
| 1.0.3.2 | `1.0.3b` |
| 1.0.3.3 | `1.0.3c` |
| 1.0.3.4 | `1.0.3d` |
| 1.0.3.5 | `1.0.3e` |

The migration preserves release commits and ZIP bytes. Archived source and
bundled documents can therefore retain historical names. Legacy Git tags remain
as source-history aliases; use canonical release links for downloads.
The next release on upstream 1.0.3 is `1.0.3f`.
