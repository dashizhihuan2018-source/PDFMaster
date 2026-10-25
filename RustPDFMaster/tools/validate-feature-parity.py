#!/usr/bin/env python3
"""Validate the human-readable, source-backed feature parity inventory.

This validator checks inventory structure and evidence references. It never
executes or marks the legacy XCTest contracts as passed.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path, PurePosixPath
from typing import Any


ORIGINAL_STATES = {"available", "partial", "conditional", "unavailable"}
RUST_STATES = {"partial", "unsupported", "complete"}
PARITY_SCOPES = {"required", "excluded_original_unavailable"}
EVIDENCE_ROOTS = {"original_checkout", "rust_candidate"}
EVIDENCE_KINDS = {"source", "test", "data", "document"}
PLATFORMS = {"macos", "windows", "ios"}
PLATFORM_STATES = {
    "verified",
    "candidate_unaccepted",
    "compiled_only",
    "not_run",
    "unsupported",
    "not_applicable",
}
REQUIRED_DOMAINS = {
    "tool_catalog",
    "shell",
    "reader",
    "document",
    "import_export",
    "local_canvas",
    "canvas_web",
    "ai_account",
    "safety",
}


def _mapping(value: Any, label: str, errors: list[str]) -> dict[str, Any]:
    if not isinstance(value, dict):
        errors.append(f"{label} must be an object")
        return {}
    return value


def _list(value: Any, label: str, errors: list[str]) -> list[Any]:
    if not isinstance(value, list):
        errors.append(f"{label} must be an array")
        return []
    return value


def _unique_ids(items: list[Any], label: str, errors: list[str]) -> set[str]:
    seen: set[str] = set()
    for index, item in enumerate(items):
        if not isinstance(item, dict) or not isinstance(item.get("id"), str) or not item["id"]:
            errors.append(f"{label}[{index}] needs a non-empty id")
            continue
        item_id = item["id"]
        if item_id in seen:
            errors.append(f"duplicate {label[:-1]} id: {item_id}")
        seen.add(item_id)
    return seen


def _validate_refs(
    refs: Any,
    label: str,
    evidence: dict[str, Any],
    errors: list[str],
    *,
    required: bool,
) -> list[str]:
    values = _list(refs, label, errors)
    if required and not values:
        errors.append(f"{label} must include evidence")
    valid: list[str] = []
    for ref in values:
        if not isinstance(ref, str) or ref not in evidence:
            errors.append(f"{label} references unknown evidence: {ref!r}")
        else:
            valid.append(ref)
    return valid


def validate_document(document: Any) -> list[str]:
    errors: list[str] = []
    doc = _mapping(document, "inventory", errors)
    if doc.get("schema_version") != 1:
        errors.append("schema_version must equal 1")

    legacy = _mapping(doc.get("legacy_contracts"), "legacy_contracts", errors)
    if legacy.get("count") != 801:
        errors.append("legacy_contracts.count must equal the observed 801 contracts")
    if legacy.get("execution_claim") != "not_executed_by_this_inventory":
        errors.append("legacy_contracts must deny an execution claim")

    evidence = _mapping(doc.get("evidence_index"), "evidence_index", errors)
    for evidence_id, item in evidence.items():
        entry = _mapping(item, f"evidence_index.{evidence_id}", errors)
        if entry.get("root") not in EVIDENCE_ROOTS:
            errors.append(f"evidence_index.{evidence_id}.root is invalid")
        if entry.get("kind") not in EVIDENCE_KINDS:
            errors.append(f"evidence_index.{evidence_id}.kind is invalid")
        path = entry.get("path")
        if not isinstance(path, str) or not path or PurePosixPath(path).is_absolute() or ".." in PurePosixPath(path).parts:
            errors.append(f"evidence_index.{evidence_id}.path must be safe and relative")

    profiles = _mapping(doc.get("platform_profiles"), "platform_profiles", errors)
    for profile_id, raw in profiles.items():
        profile = _mapping(raw, f"platform_profiles.{profile_id}", errors)
        if set(profile) != PLATFORMS:
            errors.append(f"platform profile {profile_id} must define macos/windows/ios")
        for platform, state in profile.items():
            if platform in PLATFORMS and state not in PLATFORM_STATES:
                errors.append(f"platform profile {profile_id}.{platform} has invalid state")

    features = _list(doc.get("features"), "features", errors)
    feature_ids = _unique_ids(features, "features", errors)
    observed_domains: set[str] = set()
    for index, raw in enumerate(features):
        feature = _mapping(raw, f"features[{index}]", errors)
        feature_id = feature.get("id", f"index-{index}")
        domain = feature.get("domain")
        if not isinstance(domain, str) or not domain:
            errors.append(f"feature {feature_id} needs a domain")
        else:
            observed_domains.add(domain)
        if not isinstance(feature.get("title"), str) or not feature["title"]:
            errors.append(f"feature {feature_id} needs a title")
        scope = feature.get("parity_scope")
        if scope not in PARITY_SCOPES:
            errors.append(f"feature {feature_id} has invalid parity_scope")

        original = _mapping(feature.get("original"), f"feature {feature_id}.original", errors)
        original_status = original.get("status")
        if original_status not in ORIGINAL_STATES:
            errors.append(f"feature {feature_id} has invalid original status")
        _validate_refs(
            original.get("evidence"),
            f"feature {feature_id} original evidence",
            evidence,
            errors,
            required=True,
        )
        if original_status == "unavailable" and scope != "excluded_original_unavailable":
            errors.append(f"feature {feature_id} is unavailable originally and must be excluded")
        if original_status != "unavailable" and scope != "required":
            errors.append(f"feature {feature_id} is original parity scope and must be required")

        rust = _mapping(feature.get("rust"), f"feature {feature_id}.rust", errors)
        rust_status = rust.get("status")
        if rust_status not in RUST_STATES:
            errors.append(f"feature {feature_id} has invalid rust status")
        profile_id = rust.get("platform_profile")
        if profile_id not in profiles:
            errors.append(f"feature {feature_id} references unknown platform profile {profile_id}")
        _validate_refs(
            rust.get("evidence"),
            f"feature {feature_id} rust evidence",
            evidence,
            errors,
            required=True,
        )
        if rust_status == "complete":
            acceptance = _list(rust.get("acceptance"), f"feature {feature_id} acceptance", errors)
            if not acceptance:
                errors.append(f"feature {feature_id} cannot be complete without acceptance")
            if profile_id in profiles and any(state != "verified" for state in profiles[profile_id].values()):
                errors.append(f"feature {feature_id} cannot be complete with an unverified platform profile")

        links = _mapping(feature.get("test_links"), f"feature {feature_id}.test_links", errors)
        original_tests = _validate_refs(
            links.get("original"),
            f"feature {feature_id} original test_links",
            evidence,
            errors,
            required=False,
        )
        rust_tests = _validate_refs(
            links.get("rust"),
            f"feature {feature_id} rust test_links",
            evidence,
            errors,
            required=False,
        )
        for ref in original_tests + rust_tests:
            if evidence[ref].get("kind") != "test":
                errors.append(f"feature {feature_id} test link {ref} is not test evidence")
        if scope == "required" and not isinstance(feature.get("next_slice"), str):
            errors.append(f"feature {feature_id} requires a next_slice statement")
        if not isinstance(feature.get("gap"), str) or not feature["gap"]:
            errors.append(f"feature {feature_id} needs an explicit gap")

    missing_domains = REQUIRED_DOMAINS - observed_domains
    if missing_domains:
        errors.append(f"features missing required domains: {sorted(missing_domains)}")

    tools = _list(doc.get("original_tool_catalog"), "original_tool_catalog", errors)
    _unique_ids(tools, "original_tool_catalog", errors)
    if len(tools) != 28:
        errors.append("original_tool_catalog must contain exactly 28 entries")
    for index, raw in enumerate(tools):
        tool = _mapping(raw, f"original_tool_catalog[{index}]", errors)
        tool_id = tool.get("id", f"index-{index}")
        availability = tool.get("original_availability")
        if availability not in {"available", "partial", "unavailable"}:
            errors.append(f"tool {tool_id} has invalid availability")
        if tool.get("required_for_replica") is not (availability != "unavailable"):
            errors.append(f"tool {tool_id} required_for_replica contradicts original availability")
        linked = tool.get("feature_id")
        if linked not in feature_ids:
            errors.append(f"tool {tool_id} links missing feature {linked}")
        else:
            feature = next(item for item in features if isinstance(item, dict) and item.get("id") == linked)
            expected_scope = "excluded_original_unavailable" if availability == "unavailable" else "required"
            if feature.get("parity_scope") != expected_scope:
                errors.append(f"tool {tool_id} linked feature has wrong parity scope")
        _validate_refs(
            tool.get("evidence"),
            f"tool {tool_id} evidence",
            evidence,
            errors,
            required=True,
        )

    recommendation = _mapping(doc.get("next_slice_recommendation"), "next_slice_recommendation", errors)
    included = _list(recommendation.get("feature_ids"), "next_slice_recommendation.feature_ids", errors)
    if not included:
        errors.append("next_slice_recommendation must name at least one feature")
    for feature_id in included:
        if feature_id not in feature_ids:
            errors.append(f"next slice references missing feature {feature_id}")
    if not _list(recommendation.get("exit_criteria"), "next_slice_recommendation.exit_criteria", errors):
        errors.append("next_slice_recommendation needs exit criteria")

    return errors


def validate_paths(
    document: dict[str, Any],
    repo_root: Path,
    original_root: Path | None,
) -> list[str]:
    errors: list[str] = []
    for evidence_id, item in document.get("evidence_index", {}).items():
        root = item.get("root")
        if root == "rust_candidate":
            base = repo_root
        elif root == "original_checkout":
            if original_root is None:
                continue
            base = original_root
        else:
            continue
        path = base.joinpath(*PurePosixPath(item["path"]).parts)
        if not path.is_file():
            errors.append(f"evidence {evidence_id} does not exist: {path}")
    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "inventory",
        nargs="?",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "cross-platform" / "features.json",
    )
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--original-root", type=Path)
    args = parser.parse_args(argv)
    try:
        document = json.loads(args.inventory.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"feature inventory unreadable: {error}", file=sys.stderr)
        return 2
    errors = validate_document(document)
    if not errors:
        errors.extend(validate_paths(document, args.repo_root, args.original_root))
    if errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
        return 1
    suffix = " including original checkout" if args.original_root else " (original paths not checked)"
    print(f"feature parity inventory valid: {len(document['features'])} features{suffix}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
