#!/usr/bin/env python3
# Extract (method, path, handler) triples from axum Router chains. Author: kejiqing
"""Scan Rust sources for `.route(...)` registrations and emit a stable contract."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


def extract_from_text(text: str) -> list[tuple[str, str, str]]:
    routes: list[tuple[str, str, str]] = []
    i = 0
    while True:
        j = text.find(".route(", i)
        if j < 0:
            break
        k = j + len(".route(")
        depth = 1
        in_str = False
        escape = False
        while k < len(text) and depth > 0:
            c = text[k]
            if in_str:
                if escape:
                    escape = False
                elif c == "\\":
                    escape = True
                elif c == '"':
                    in_str = False
            else:
                if c == '"':
                    in_str = True
                elif c == "(":
                    depth += 1
                elif c == ")":
                    depth -= 1
            k += 1
        body = text[j + len(".route(") : k - 1]
        pm = re.search(r'"([^"]+)"', body)
        if not pm:
            i = k
            continue
        path = pm.group(1)
        handlers_part = body[pm.end() :]
        for hm in re.finditer(
            r"\b(get|post|put|patch|delete)\(([^()]+(?:\([^()]*\)[^()]*)*)\)",
            handlers_part,
        ):
            method = hm.group(1).upper()
            handler = hm.group(2).strip()
            routes.append((method, path, handler))
        i = k
    return routes


def extract_from_paths(paths: list[Path]) -> list[tuple[str, str, str]]:
    routes: list[tuple[str, str, str]] = []
    for path in paths:
        routes.extend(extract_from_text(path.read_text(encoding="utf-8")))
    # Stable sort for set-diff friendliness while remaining deterministic.
    return sorted(set(routes), key=lambda t: (t[1], t[0], t[2]))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "roots",
        nargs="*",
        type=Path,
        default=[Path("src")],
        help="Rust source roots to scan (default: src)",
    )
    parser.add_argument(
        "--baseline",
        type=Path,
        help="Baseline contract file; exit 1 on any diff",
    )
    parser.add_argument(
        "--write",
        type=Path,
        help="Write current contract to this path",
    )
    args = parser.parse_args()

    files: list[Path] = []
    for root in args.roots:
        if root.is_file():
            files.append(root)
        else:
            files.extend(sorted(root.rglob("*.rs")))
    routes = extract_from_paths(files)
    lines = [f"{m}\t{p}\t{h}" for m, p, h in routes]
    text = "\n".join(lines) + ("\n" if lines else "")

    if args.write:
        args.write.parent.mkdir(parents=True, exist_ok=True)
        args.write.write_text(text, encoding="utf-8")
        print(f"wrote {len(routes)} routes -> {args.write}", file=sys.stderr)

    if args.baseline:
        baseline = args.baseline.read_text(encoding="utf-8")
        base_set = set(baseline.splitlines())
        cur_set = set(lines)
        missing = sorted(base_set - cur_set)
        extra = sorted(cur_set - base_set)
        if missing or extra:
            print("route contract DIFF", file=sys.stderr)
            for line in missing:
                print(f"- {line}", file=sys.stderr)
            for line in extra:
                print(f"+ {line}", file=sys.stderr)
            return 1
        print(f"route contract OK ({len(cur_set)} triples)", file=sys.stderr)

    if not args.write and not args.baseline:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
