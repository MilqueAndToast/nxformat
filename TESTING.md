# Local-decryption validation

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
