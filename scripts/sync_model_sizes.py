#!/usr/bin/env python3
"""Verify or synchronize pinned Hugging Face model file byte sizes.

Usage:
    python3 scripts/sync_model_sizes.py --check
    python3 scripts/sync_model_sizes.py --write
    python3 scripts/sync_model_sizes.py          # defaults to --write

Queries Hugging Face Hub's tree API for every pinned (repo, commit) in
`crates/koharu-ml/src/*/mod.rs` and `crates/koharu-llm/src/local/catalog.rs`.
Optional `HF_TOKEN` environment variable is sent as a Bearer token.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ML_DIR = ROOT / "crates" / "koharu-ml" / "src"
LLM_CATALOG = ROOT / "crates" / "koharu-llm" / "src" / "local" / "catalog.rs"

ML_REPO_RE = re.compile(
    r'const\s+([A-Z0-9_]+)\s*:\s*Repository\s*=\s*Repository::(model|dataset)\(\s*"([^"]+)"\s*,\s*"([0-9a-fA-F]{40})"\s*,?\s*\);',
    re.MULTILINE,
)
PINNED_FILE_RE = re.compile(
    r'PinnedFile::new\(\s*([A-Z0-9_]+)\s*,\s*"([^"]+)"\s*,\s*([0-9_]+)\s*(,?)\s*\)',
    re.MULTILINE,
)
LLM_ENTRY_RE = re.compile(
    r'repository\s*:\s*Repository::(model|dataset)\(\s*"([^"]+)"\s*,\s*"([0-9a-fA-F]{40})"\s*,?\s*\)(.*?)(?=\n\s*Entry\s*\{|\n\];)',
    re.DOTALL,
)
QUANT_RE = re.compile(
    r'Quantization::new\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*,\s*"([^"]+)"\s*,\s*([0-9_]+)\s*(,?)\s*\)',
    re.MULTILINE,
)
PROJECTOR_RE = re.compile(
    r'Projector::new\(\s*"([^"]+)"\s*,\s*([0-9_]+)\s*(,?)\s*\)',
    re.MULTILINE,
)
NEXT_LINK_RE = re.compile(r'<([^>]+)>\s*;\s*rel="next"')


@dataclass(frozen=True)
class RepoRef:
    kind: str  # "models" or "datasets"
    repo_id: str
    commit: str


def format_rust_int(value: int) -> str:
    return f"{value:_}"


def parse_rust_int(literal: str) -> int:
    return int(literal.replace("_", ""))


def fetch_json_with_retry(url: str, token: str | None) -> tuple[object, str | None]:
    headers = {"User-Agent": "koharu-sync-model-sizes/1.0"}
    if token:
        headers["Authorization"] = f"Bearer {token}"

    delay = 0.5
    for attempt in range(5):
        req = urllib.request.Request(url, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=30) as resp:
                body = resp.read().decode("utf-8")
                link = resp.headers.get("Link")
                next_url = None
                if link:
                    m = NEXT_LINK_RE.search(link)
                    if m:
                        next_url = m.group(1)
                return json.loads(body), next_url
        except urllib.error.HTTPError as exc:
            if exc.code in (429, 500, 502, 503, 504) and attempt < 4:
                time.sleep(delay)
                delay *= 2
                continue
            raise RuntimeError(f"HTTP {exc.code} for {url}: {exc.reason}") from exc
        except urllib.error.URLError as exc:
            if attempt < 4:
                time.sleep(delay)
                delay *= 2
                continue
            raise RuntimeError(f"request failed for {url}: {exc.reason}") from exc

    raise RuntimeError(f"exhausted retries for {url}")


def fetch_repo_tree(repo: RepoRef, token: str | None) -> dict[str, int]:
    url: str | None = (
        f"https://huggingface.co/api/{repo.kind}/{repo.repo_id}/tree/{repo.commit}?recursive=true"
    )
    files: dict[str, int] = {}
    while url:
        payload, next_url = fetch_json_with_retry(url, token)
        if not isinstance(payload, list):
            raise RuntimeError(f"unexpected tree payload for {repo.repo_id}@{repo.commit}")
        for item in payload:
            if item.get("type") == "file":
                path = item.get("path")
                size = item.get("size")
                if not isinstance(path, str) or not isinstance(size, int) or size <= 0:
                    raise RuntimeError(
                        f"invalid file entry {item!r} in {repo.repo_id}@{repo.commit}"
                    )
                files[path] = size
        url = next_url
    return files


def collect_repos() -> list[tuple[Path, str, list[RepoRef]]]:
    targets: list[tuple[Path, str, list[RepoRef]]] = []

    for mod_rs in sorted(ML_DIR.glob("*/mod.rs")):
        text = mod_rs.read_text(encoding="utf-8")
        repos: list[RepoRef] = []
        for match in ML_REPO_RE.finditer(text):
            kind = "datasets" if match.group(2) == "dataset" else "models"
            repos.append(RepoRef(kind=kind, repo_id=match.group(3), commit=match.group(4)))
        if repos:
            if not PINNED_FILE_RE.search(text):
                raise RuntimeError(
                    f"{mod_rs.relative_to(ROOT)} defines Repository::* but has no PinnedFile::new entries"
                )
            targets.append((mod_rs, text, repos))

    llm_text = LLM_CATALOG.read_text(encoding="utf-8")
    llm_repos = [
        RepoRef(
            kind="datasets" if m.group(1) == "dataset" else "models",
            repo_id=m.group(2),
            commit=m.group(3),
        )
        for m in LLM_ENTRY_RE.finditer(llm_text)
    ]
    if not llm_repos:
        raise RuntimeError(f"no Entry blocks found in {LLM_CATALOG.relative_to(ROOT)}")
    targets.append((LLM_CATALOG, llm_text, llm_repos))
    return targets


def process_ml_file(
    path: Path,
    text: str,
    trees: dict[RepoRef, dict[str, int]],
    write: bool,
) -> tuple[int, list[str], bool]:
    var_to_repo: dict[str, RepoRef] = {}
    for match in ML_REPO_RE.finditer(text):
        var_name = match.group(1)
        kind = "datasets" if match.group(2) == "dataset" else "models"
        var_to_repo[var_name] = RepoRef(kind=kind, repo_id=match.group(3), commit=match.group(4))

    checked = 0
    mismatches: list[str] = []

    def replace_pinned(match: re.Match[str]) -> str:
        nonlocal checked
        var_name, filename, raw_bytes, trailing_comma = (
            match.group(1),
            match.group(2),
            match.group(3),
            match.group(4),
        )
        repo = var_to_repo.get(var_name)
        if repo is None:
            raise RuntimeError(
                f"{path.relative_to(ROOT)}: unknown repository constant {var_name}"
            )
        tree = trees[repo]
        actual = tree.get(filename)
        if actual is None:
            raise RuntimeError(
                f"{path.relative_to(ROOT)}: {filename!r} not found in {repo.repo_id}@{repo.commit}"
            )
        checked += 1
        declared = parse_rust_int(raw_bytes)
        if declared != actual:
            mismatches.append(
                f"{path.relative_to(ROOT)}: {repo.repo_id} {filename}: declared {declared}, actual {actual}"
            )
            if write:
                formatted = format_rust_int(actual)
                full = match.group(0)
                start_bytes = match.start(3) - match.start(0)
                end_bytes = match.end(3) - match.start(0)
                return full[:start_bytes] + formatted + full[end_bytes:]
        return match.group(0)

    updated = PINNED_FILE_RE.sub(replace_pinned, text)
    changed = updated != text
    if write and changed:
        path.write_text(updated, encoding="utf-8")
    return checked, mismatches, changed


def process_llm_catalog(
    path: Path,
    text: str,
    trees: dict[RepoRef, dict[str, int]],
    write: bool,
) -> tuple[int, list[str], bool]:
    checked = 0
    mismatches: list[str] = []

    def replace_entry(entry_match: re.Match[str]) -> str:
        nonlocal checked
        kind = "datasets" if entry_match.group(1) == "dataset" else "models"
        repo = RepoRef(
            kind=kind,
            repo_id=entry_match.group(2),
            commit=entry_match.group(3),
        )
        tree = trees[repo]
        block = entry_match.group(0)

        def replace_quant(q_match: re.Match[str]) -> str:
            nonlocal checked
            filename = q_match.group(3)
            raw_bytes = q_match.group(4)
            actual = tree.get(filename)
            if actual is None:
                raise RuntimeError(
                    f"{path.relative_to(ROOT)}: {filename!r} not found in {repo.repo_id}@{repo.commit}"
                )
            checked += 1
            declared = parse_rust_int(raw_bytes)
            if declared != actual:
                mismatches.append(
                    f"{path.relative_to(ROOT)}: {repo.repo_id} {filename}: declared {declared}, actual {actual}"
                )
                if write:
                    formatted = format_rust_int(actual)
                    full = q_match.group(0)
                    start_bytes = q_match.start(4) - q_match.start(0)
                    end_bytes = q_match.end(4) - q_match.start(0)
                    return full[:start_bytes] + formatted + full[end_bytes:]
            return q_match.group(0)

        def replace_projector(p_match: re.Match[str]) -> str:
            nonlocal checked
            filename = p_match.group(1)
            raw_bytes = p_match.group(2)
            actual = tree.get(filename)
            if actual is None:
                raise RuntimeError(
                    f"{path.relative_to(ROOT)}: projector {filename!r} not found in {repo.repo_id}@{repo.commit}"
                )
            checked += 1
            declared = parse_rust_int(raw_bytes)
            if declared != actual:
                mismatches.append(
                    f"{path.relative_to(ROOT)}: {repo.repo_id} {filename}: declared {declared}, actual {actual}"
                )
                if write:
                    formatted = format_rust_int(actual)
                    full = p_match.group(0)
                    start_bytes = p_match.start(2) - p_match.start(0)
                    end_bytes = p_match.end(2) - p_match.start(0)
                    return full[:start_bytes] + formatted + full[end_bytes:]
            return p_match.group(0)

        block = QUANT_RE.sub(replace_quant, block)
        block = PROJECTOR_RE.sub(replace_projector, block)
        return block

    updated = LLM_ENTRY_RE.sub(replace_entry, text)
    changed = updated != text
    if write and changed:
        path.write_text(updated, encoding="utf-8")
    return checked, mismatches, changed


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Verify or update pinned Hugging Face model file sizes."
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--check",
        action="store_true",
        help="Verify declared sizes match Hugging Face without modifying files.",
    )
    mode.add_argument(
        "--write",
        action="store_true",
        help="Update declared sizes in Rust source files (default).",
    )
    args = parser.parse_args()
    write = not args.check

    targets = collect_repos()
    unique_repos = sorted(
        {repo for _, _, repos in targets for repo in repos},
        key=lambda r: (r.kind, r.repo_id, r.commit),
    )

    token = os.environ.get("HF_TOKEN")
    trees: dict[RepoRef, dict[str, int]] = {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        future_map = {
            pool.submit(fetch_repo_tree, repo, token): repo for repo in unique_repos
        }
        for future in concurrent.futures.as_completed(future_map):
            repo = future_map[future]
            trees[repo] = future.result()

    total_checked = 0
    all_mismatches: list[str] = []
    changed_files: list[str] = []

    for path, text, _ in targets:
        if path == LLM_CATALOG:
            checked, mismatches, changed = process_llm_catalog(path, text, trees, write)
        else:
            checked, mismatches, changed = process_ml_file(path, text, trees, write)
        total_checked += checked
        all_mismatches.extend(mismatches)
        if changed:
            changed_files.append(str(path.relative_to(ROOT)))

    if not write and all_mismatches:
        for line in all_mismatches:
            print(line, file=sys.stderr)
        print(
            f"FAILED: {len(all_mismatches)} mismatch(es) across {total_checked} files ({len(unique_repos)} repos)",
            file=sys.stderr,
        )
        return 1

    if write and changed_files:
        for rel in changed_files:
            print(f"updated {rel}")
    print(
        f"verified {total_checked} files across {len(unique_repos)} repositories"
        + (f" ({len(changed_files)} file(s) updated)" if write else "")
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
