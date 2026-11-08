"""Check required documentation, directories, and fixture catalog coverage."""

from __future__ import annotations

import json
import re
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit


ROOT = Path(__file__).resolve().parents[2]
REQUIRED_FILES = (
    "README.md",
    "docs/SPEC.md",
    "docs/SECURITY.md",
    "docs/READER.md",
    "docs/RECIPE.md",
    "docs/CLI.md",
    "benchmarks/README.md",
    ".github/workflows/ci.yml",
    "tests/fixtures/README.md",
    "tests/conformance/build_fixtures.py",
    "tests/conformance/check_phase0.py",
    "tests/conformance/check_reproducible.py",
    "tests/conformance/validate_fixtures.py",
    "tests/conformance/test_reader.py",
    "tests/conformance/test_compat_increment1.py",
    "tests/conformance/test_rust_writer.py",
    "tests/conformance/test_rust_repack.py",
    "tests/conformance/repack_output_reader.cpp",
    "tests/conformance/reader_smoke.cpp",
    "tests/fuzz/nx_reader_fuzz.cpp",
    "reader/nx_reader.hpp",
    "reader/generate_header.py",
    "dist/nx_reader.hpp",
    "benchmarks/run_writer_compression.py",
    "benchmarks/run_repack_benchmarks.py",
    "benchmarks/run_reader_cache_benchmark.py",
)
REQUIRED_DIRS = ("reader", "packer", "benchmarks", "dist", "tests/fixtures", "tests/conformance", "tests/fuzz")


class HtmlReferences(HTMLParser):
    """Include README navigation and badge/image paths in the offline check."""

    def __init__(self) -> None:
        super().__init__()
        self.targets: list[str] = []
        self.anchors: set[str] = set()

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        for key, value in attrs:
            if value is None:
                continue
            if (tag == "a" and key == "href") or (tag == "img" and key == "src"):
                self.targets.append(value)
            if key == "id" or (tag == "a" and key == "name"):
                self.anchors.add(value)


def document_references(source: str) -> tuple[list[str], set[str]]:
    # Code examples can contain placeholder paths and Markdown headings.
    source = re.sub(r"^(`{3,}|~{3,}).*?^\1[^\n]*$", "", source, flags=re.M | re.S)
    html = HtmlReferences()
    html.feed(source)
    targets = re.findall(r"\]\(([^)]+)\)", source) + html.targets
    anchors = html.anchors
    counts: dict[str, int] = {}
    for heading in re.findall(r"^#{1,6}\s+(.+?)\s*#*\s*$", source, flags=re.M):
        heading = re.sub(r"<[^>]+>", "", heading).lower()
        slug = re.sub(r"[^\w\- ]", "", heading).replace(" ", "-")
        count = counts.get(slug, 0)
        counts[slug] = count + 1
        anchors.add(f"{slug}-{count}" if count else slug)
    return targets, anchors


def main() -> int:
    missing = [item for item in REQUIRED_FILES if not (ROOT / item).is_file()]
    missing.extend(item + "/" for item in REQUIRED_DIRS if not (ROOT / item).is_dir())
    if missing:
        print("missing Phase 0 paths: " + ", ".join(missing), file=sys.stderr)
        return 1

    index_path = ROOT / "tests/fixtures/fixture_index.json"
    if not index_path.is_file():
        print("missing tests/fixtures/fixture_index.json; run the fixture builder", file=sys.stderr)
        return 1
    index = json.loads(index_path.read_text(encoding="utf-8"))
    indexed = {record["file"] for record in index["fixtures"]}
    actual = {path.name for path in (ROOT / "tests/fixtures").glob("*.nx")}
    if indexed != actual:
        print(f"fixture index mismatch: missing={sorted(actual-indexed)} stale={sorted(indexed-actual)}", file=sys.stderr)
        return 1
    catalog = (ROOT / "tests/fixtures/README.md").read_text(encoding="utf-8")
    undocumented = sorted(filename for filename in indexed if filename not in catalog)
    if undocumented:
        print("fixtures missing from README: " + ", ".join(undocumented), file=sys.stderr)
        return 1

    bad_links: list[str] = []
    generated_directories = {".git", "build", "target", "__pycache__", "node_modules"}
    for markdown in ROOT.rglob("*.md"):
        if generated_directories.intersection(markdown.relative_to(ROOT).parts):
            continue
        targets, _ = document_references(markdown.read_text(encoding="utf-8"))
        for target in targets:
            target = target.strip()
            url = urlsplit(target)
            if url.scheme or url.netloc:
                continue
            resolved = (markdown.parent / unquote(url.path)).resolve() if url.path else markdown
            if not resolved.is_relative_to(ROOT.resolve()) or not resolved.exists():
                bad_links.append(f"{markdown.relative_to(ROOT)} -> {target}")
            elif url.fragment and resolved.suffix == ".md":
                _, anchors = document_references(resolved.read_text(encoding="utf-8"))
                if unquote(url.fragment) not in anchors:
                    bad_links.append(f"{markdown.relative_to(ROOT)} -> {target} (missing anchor)")
    if bad_links:
        print("broken local Markdown links: " + ", ".join(bad_links), file=sys.stderr)
        return 1
    print(f"PASS Phase 0 structure: {len(REQUIRED_FILES)} required files, {len(REQUIRED_DIRS)} directories, {len(indexed)} documented fixtures")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
