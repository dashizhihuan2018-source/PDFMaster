#!/usr/bin/env python3
"""Inspect only the publication tree. Never print matching credential values."""
import argparse
import hashlib
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
REVIEWED_UI = json.loads((ROOT / "scripts" / "reviewed-ui-literals.json").read_text())
EXCLUDED = {"dist", "output", "work", ".git", ".swiftpm", "__pycache__", "node_modules"}


def files():
    return sorted(p for p in ROOT.rglob("*") if p.is_file()
                  and not any(part in EXCLUDED or part.startswith(".build") for part in p.relative_to(ROOT).parts)
                  and p.relative_to(ROOT).parts[0] != "tools"
                  and p.relative_to(ROOT).parts[:2] != ("Resources", "CanvasWeb")
                  and not p.name.endswith(".tsbuildinfo")
                  and p.name != "SOURCE-MANIFEST.sha256")


def audit():
    patterns = {
        "provider_credential": re.compile(r"(?i)(?:sk-[a-z0-9_-]{16,}|gh[pousr]_[a-z0-9]{20,}|github_pat_[a-z0-9_]{20,}|AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{30,})"),
        "private_key": re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
        "personal_home": re.compile(r"/(?:Users|home)/[A-Za-z0-9_.-]+/"),
    }
    matches = []
    known_bootstrap = 0
    test_literal_matches = 0
    reviewed_ui_matches = 0
    credential_assignment = re.compile(r'(?i)\b\w*(?:password|passwd|secret|token|apikey|api_key|accesskey|access_key)\w*\b\s*(?:=|:)\s*"([^"\n]+)"')
    candidates = files()
    for path in candidates:
        relative = path.relative_to(ROOT).as_posix()
        if path.is_symlink():
            matches.append({"file": relative, "kind": "symlink"})
            continue
        try:
            content = path.read_text()
        except UnicodeDecodeError:
            if b"/Users/" in path.read_bytes():
                matches.append({"file": relative, "kind": "binary_home_marker"})
            continue
        for line_number, line in enumerate(content.splitlines(), 1):
            for kind, pattern in patterns.items():
                if pattern.search(line):
                    matches.append({"file": relative, "line": line_number, "kind": kind})
            if credential_assignment.search(line):
                if relative == "Sources/PaperDesk/AccountStore.swift" and "let bootstrapPassword" in line:
                    known_bootstrap += 1
                elif relative.startswith("Tests/"):
                    test_literal_matches += 1
                elif hashlib.sha256(line.encode()).hexdigest() in REVIEWED_UI.get(relative, []):
                    reviewed_ui_matches += 1
                else:
                    matches.append({"file": relative, "line": line_number, "kind": "credential_literal_review"})
    print(json.dumps({"files": len(candidates), "bytes": sum(p.stat().st_size for p in candidates),
        "unexpected_matches": matches, "known_public_bootstrap_literals": known_bootstrap,
        "synthetic_test_literal_lines": test_literal_matches,
        "reviewed_ui_literal_lines": reviewed_ui_matches,
        "note": "Heuristic scan, not a security certification; values never printed. Review test literals separately."}, indent=2))
    return bool(matches)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", action="store_true", help="Print SHA-256 manifest, excluding generated build/dist and the manifest itself")
    args = parser.parse_args()
    if args.manifest:
        for path in files():
            print(hashlib.sha256(path.read_bytes()).hexdigest(), "  ", path.relative_to(ROOT).as_posix(), sep="")
    else:
        sys.exit(audit())
