"""Create a temporary no-dedup control crate for the Increment 2 A/B run.

The generated tree stays outside the product sources. It retains the current
toolchain, SHA backend, verification, and shared input buffer, while restoring
Increment 1's stage-every-block and same-path rollback behavior.
"""

from __future__ import annotations

import argparse
import shutil
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def replace_once(source: str, old: str, new: str, label: str) -> str:
    count = source.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected one source match, found {count}")
    return source.replace(old, new)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("destination", type=Path, help="new temporary directory for the control crate")
    args = parser.parse_args()
    destination = args.destination.resolve()
    if destination.exists():
        parser.error(f"refusing to overwrite existing path: {destination}")
    source_packer = ROOT / "packer"
    target_packer = destination / "packer"
    shutil.copytree(source_packer, target_packer, ignore=shutil.ignore_patterns("target"))

    writer = target_packer / "src" / "writer.rs"
    code = writer.read_text(encoding="utf-8")
    code = replace_once(
        code,
        """    if let Some(id) = find_existing_block(key, bytes, data_spool, blocks, index, options)? {
        return Ok((id, true));
    }
""",
        """    // Benchmark control: always stage a new RAW block.
    let _ = (key, index);
""",
        "disable candidate sharing",
    )
    code = replace_once(
        code,
        """    let digest_collision = index.buckets.contains_key(&key);
    if digest_collision {
        index.charge(DEDUP_COLLISION_ESTIMATE, options, budget)?;
        let bucket = index.buckets.get_mut(&key).ok_or_else(|| {
            WriterError::new(WriterErrorKind::Integrity, "dedup bucket disappeared")
        })?;
        bucket
            .collision_ids
            .try_reserve(1)
            .map_err(|_| limit_error("unable to reserve digest-collision candidates"))?;
    } else {
        index.charge(DEDUP_BUCKET_ESTIMATE, options, budget)?;
        index
            .buckets
            .try_reserve(1)
            .map_err(|_| limit_error("unable to reserve block-deduplication index"))?;
    }
""",
        "",
        "disable index allocation",
    )
    code = replace_once(
        code,
        """    if let Some(bucket) = index.buckets.get_mut(&key) {
        bucket.collision_ids.push(id);
    } else {
        index.buckets.insert(
            key,
            BlockBucket {
                first_id: id,
                collision_ids: Vec::new(),
            },
        );
    }
""",
        """    // No global index entry is written by this control build.
""",
        "disable index insertion",
    )
    code = replace_once(
        code,
        """    let block_count_usize = usize::try_from(block_count)
        .map_err(|_| limit_error("file block-reference count exceeds host address space"))?;
""",
        """    let block_count_usize = usize::try_from(block_count)
        .map_err(|_| limit_error("file block-reference count exceeds host address space"))?;
    let new_block_start = blocks.len();
    let spool_start = data_spool.seek(SeekFrom::End(0))?;
""",
        "restore same-path rollback tracking",
    )
    code = replace_once(
        code,
        """                    if old_id != new_id || a.logical_size != b.logical_size {
                        equal = false;
                        break;
                    }
""",
        """                    if a.logical_size != b.logical_size
                        || !compare_spool_ranges(
                            data_spool,
                            a.spool_offset,
                            b.spool_offset,
                            a.logical_size as u64,
                            options,
                        )?
                    {
                        equal = false;
                        break;
                    }
""",
        "restore byte-confirmed same-path comparison",
    )
    code = replace_once(
        code,
        """    if let Some(id) = reused {
        return Ok(id);
    }
""",
        """    if let Some(id) = reused {
        blocks.truncate(new_block_start);
        data_spool.set_len(spool_start)?;
        data_spool.seek(SeekFrom::End(0))?;
        *peak_temp = (*peak_temp).max(
            data_spool
                .seek(SeekFrom::End(0))?
                .checked_add(media_spool_len)
                .ok_or_else(|| limit_error("temporary disk accounting overflow"))?,
        );
        return Ok(id);
    }
""",
        "restore same-path rollback",
    )
    writer.write_text(code, encoding="utf-8", newline="\n")
    print(target_packer)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

