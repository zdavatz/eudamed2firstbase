#!/usr/bin/env python3
"""
Validate firstbase JSON files against the GS1 Product API Swagger schema.

Downloads the complete GDSN data model (978 schema definitions) from
test-productapi-firstbase.gs1.ch and validates all JSON files in firstbase_json/.

Usage:
    python3 firstbase_validation.py                      # validate all files
    python3 firstbase_validation.py firstbase_json/f.json # validate specific file(s)
    python3 firstbase_validation.py --verbose             # show per-file details
    python3 firstbase_validation.py --dump-schema TradeItem  # dump a schema definition
"""

import json
import os
import re
import sys
import urllib.request
import collections
from pathlib import Path

SWAGGER_URL = "https://test-productapi-firstbase.gs1.ch/docs/v01/productApi"
CACHE_PATH = Path(__file__).parent / ".swagger_cache.json"
FIRSTBASE_DIR = Path(__file__).parent / "firstbase_json"

# ── Schema loading ──────────────────────────────────────────────────────────

def fetch_swagger(use_cache=True):
    """Download and cache the Swagger spec."""
    if use_cache and CACHE_PATH.exists():
        with open(CACHE_PATH) as f:
            spec = json.load(f)
        return spec

    print(f"Downloading Swagger spec from {SWAGGER_URL} ...")
    req = urllib.request.Request(SWAGGER_URL, headers={"Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as resp:
        spec = json.loads(resp.read())

    with open(CACHE_PATH, "w") as f:
        json.dump(spec, f)
    print(f"Cached to {CACHE_PATH} ({len(spec.get('definitions', {}))} definitions)")
    return spec


def build_schema_index(spec):
    """Build short-name -> full-name lookup, preferring Standard entities."""
    defs = spec["definitions"]
    index = {}
    for name in defs:
        short = name.split(".")[-1]
        # Prefer Standard namespace
        if "Standard" in name or short not in index:
            index[short] = name
    return index


# ── Validation engine ───────────────────────────────────────────────────────

class Validator:
    def __init__(self, spec):
        self.defs = spec["definitions"]
        self.index = build_schema_index(spec)

    def resolve(self, ref_or_name):
        """Resolve a $ref string or short name to a full definition name."""
        name = ref_or_name.replace("#/definitions/", "")
        if name in self.defs:
            return name
        short = name.split(".")[-1]
        return self.index.get(short)

    def get_props(self, def_name):
        return self.defs.get(def_name, {}).get("properties", {})

    def get_enum(self, def_name):
        return self.defs.get(def_name, {}).get("enum")

    def validate(self, obj, def_name, path=""):
        """Validate an object against a schema definition. Returns list of issues."""
        issues = []
        resolved = self.resolve(def_name) if def_name not in self.defs else def_name
        if not resolved:
            issues.append(Issue("SCHEMA_NOT_FOUND", path, f"'{def_name}' not in spec"))
            return issues

        props = self.get_props(resolved)
        if not isinstance(obj, dict):
            return issues

        # Check for unknown fields
        allowed = set(props.keys())
        for key in obj:
            full_path = f"{path}.{key}" if path else key
            if key not in allowed:
                issues.append(Issue("UNKNOWN_FIELD", full_path,
                                    f"not in '{resolved.split('.')[-1]}' "
                                    f"(has {len(allowed)} properties)"))
                continue

            val = obj[key]
            prop_spec = props[key]

            # Type checks
            issues.extend(self._check_type(val, prop_spec, full_path))

            # Enum checks (for inline enums)
            if "enum" in prop_spec and val is not None:
                if val not in prop_spec["enum"]:
                    issues.append(Issue("INVALID_ENUM", full_path,
                                        f"'{val}' not in {prop_spec['enum'][:6]}"
                                        f"{'...' if len(prop_spec['enum']) > 6 else ''}"))

            # Recurse into $ref objects
            if "$ref" in prop_spec and isinstance(val, dict):
                child_def = self.resolve(prop_spec["$ref"])
                if child_def:
                    # Check if this is a code-enum type (has "enum" at definition level)
                    child_enum = self.get_enum(child_def)
                    if child_enum:
                        # The object wraps a Value field; check it
                        inner = val.get("Value")
                        if inner is not None and inner not in child_enum:
                            issues.append(Issue("INVALID_ENUM", f"{full_path}.Value",
                                                f"'{inner}' not in "
                                                f"{child_enum[:6]}"
                                                f"{'...' if len(child_enum) > 6 else ''}"))
                    else:
                        issues.extend(self.validate(val, child_def, full_path))

            # Recurse into arrays
            elif prop_spec.get("type") == "array" and isinstance(val, list):
                items_spec = prop_spec.get("items", {})
                if "$ref" in items_spec:
                    child_def = self.resolve(items_spec["$ref"])
                    if child_def:
                        child_enum = self.get_enum(child_def)
                        for i, item in enumerate(val):
                            item_path = f"{full_path}[{i}]"
                            if child_enum:
                                inner = item.get("Value") if isinstance(item, dict) else item
                                if inner is not None and inner not in child_enum:
                                    issues.append(Issue("INVALID_ENUM", item_path,
                                                        f"'{inner}' not in "
                                                        f"{child_enum[:6]}..."))
                            elif isinstance(item, dict):
                                issues.extend(self.validate(item, child_def, item_path))

        return issues

    def _check_type(self, val, prop_spec, path):
        """Check JSON value type matches schema expectation."""
        issues = []
        if val is None:
            return issues

        expected = prop_spec.get("type")
        if not expected:
            return issues

        type_map = {
            "string": str,
            "boolean": bool,
            "integer": int,
            "number": (int, float),
            "array": list,
            "object": dict,
        }

        expected_types = type_map.get(expected)
        if expected_types and not isinstance(val, expected_types):
            # bool is subclass of int in Python, guard against that
            if expected == "integer" and isinstance(val, bool):
                issues.append(Issue("TYPE_MISMATCH", path,
                                    f"expected {expected}, got boolean"))
            elif expected == "number" and isinstance(val, bool):
                issues.append(Issue("TYPE_MISMATCH", path,
                                    f"expected {expected}, got boolean"))
            else:
                issues.append(Issue("TYPE_MISMATCH", path,
                                    f"expected {expected}, got {type(val).__name__}"))

        return issues


# ── Issue tracking ──────────────────────────────────────────────────────────

class Issue:
    def __init__(self, category, path, message):
        self.category = category
        self.path = path
        self.message = message

    def __str__(self):
        return f"  {self.category} {self.path}: {self.message}"

    @property
    def normalized_path(self):
        return re.sub(r"\[\d+\]", "[*]", self.path)


# ── Output formatting ──────────────────────────────────────────────────────

def print_summary(results, verbose=False):
    """Print validation summary."""
    total = len(results)
    valid = sum(1 for r in results.values() if not r)
    invalid = sum(1 for r in results.values() if r)

    print(f"\n{'=' * 60}")
    print(f"FIRSTBASE JSON VALIDATION vs GS1 GDSN SWAGGER SCHEMA")
    print(f"{'=' * 60}")
    print(f"Files validated : {total}")
    print(f"Valid           : {valid}")
    print(f"With issues     : {invalid}")

    if verbose:
        print(f"\n{'─' * 60}")
        for fname, issues in sorted(results.items()):
            status = "PASS" if not issues else "FAIL"
            print(f"  [{status}] {fname}")
            for iss in issues:
                print(f"  {iss}")

    # Aggregate unique issue patterns
    all_patterns = collections.Counter()
    for issues in results.values():
        seen = set()
        for iss in issues:
            key = f"{iss.category} {iss.normalized_path}: {iss.message}"
            if key not in seen:
                all_patterns[key] += 1
                seen.add(key)

    if all_patterns:
        print(f"\n{'─' * 60}")
        print(f"ISSUE PATTERNS (unique path + message, count = files affected):")
        print(f"{'─' * 60}")
        for pattern, count in all_patterns.most_common(50):
            print(f"  {count:4d}x  {pattern}")
    else:
        print(f"\nAll {total} files passed validation.")

    return invalid == 0


def dump_schema(spec, name):
    """Print a schema definition for inspection."""
    index = build_schema_index(spec)
    full = index.get(name) or name
    d = spec["definitions"].get(full)
    if not d:
        # Try substring match
        matches = [n for n in spec["definitions"] if name.lower() in n.lower()]
        if matches:
            print(f"'{name}' not found. Did you mean:")
            for m in matches[:10]:
                print(f"  {m}")
        else:
            print(f"'{name}' not found in {len(spec['definitions'])} definitions.")
        return
    print(f"\n{full}:")
    print(json.dumps(d, indent=2))


# ── Main ────────────────────────────────────────────────────────────────────

def main():
    args = sys.argv[1:]
    verbose = "--verbose" in args or "-v" in args
    args = [a for a in args if a not in ("--verbose", "-v")]

    # Dump schema mode
    if args and args[0] == "--dump-schema":
        spec = fetch_swagger()
        if len(args) > 1:
            dump_schema(spec, args[1])
        else:
            print("Usage: --dump-schema <SchemaName>")
        return

    # Refresh cache
    if "--refresh" in args:
        args.remove("--refresh")
        if CACHE_PATH.exists():
            CACHE_PATH.unlink()

    # Load schema
    spec = fetch_swagger()
    validator = Validator(spec)
    n_defs = len(spec["definitions"])
    print(f"Schema loaded: {n_defs} definitions")

    # Find TradeItem definition
    ti_def = None
    for name in spec["definitions"]:
        if name.endswith(".TradeItem") and "Standard" in name:
            ti_def = name
            break
    if not ti_def:
        print("ERROR: TradeItem definition not found in schema")
        sys.exit(1)
    print(f"TradeItem schema: {ti_def} ({len(validator.get_props(ti_def))} properties)")

    # Collect files to validate
    if args:
        files = [Path(a) for a in args if a.endswith(".json")]
    else:
        if not FIRSTBASE_DIR.exists():
            print(f"ERROR: {FIRSTBASE_DIR} not found. Generate firstbase JSON first.")
            sys.exit(1)
        files = sorted(FIRSTBASE_DIR.glob("*.json"))

    if not files:
        print("No JSON files to validate.")
        sys.exit(1)

    print(f"Validating {len(files)} files...\n")

    # Validate each file
    results = {}
    for fpath in files:
        try:
            with open(fpath) as f:
                doc = json.load(f)
        except json.JSONDecodeError as e:
            results[fpath.name] = [Issue("PARSE_ERROR", "", str(e))]
            continue

        trade_item = doc.get("TradeItem", doc)
        issues = validator.validate(trade_item, ti_def, "TradeItem")

        # Also validate CatalogueItemChildItemLink children
        children = doc.get("CatalogueItemChildItemLink", [])
        child_def = validator.resolve("CatalogueItemChildItemLink")
        if child_def and children:
            for i, child in enumerate(children):
                child_path = f"CatalogueItemChildItemLink[{i}]"
                issues.extend(validator.validate(child, child_def, child_path))
                # Validate nested TradeItem in child
                child_ti = child.get("CatalogueItem", {}).get("TradeItem")
                if child_ti:
                    issues.extend(validator.validate(
                        child_ti, ti_def,
                        f"{child_path}.CatalogueItem.TradeItem"))

        results[fpath.name] = issues

    success = print_summary(results, verbose)
    sys.exit(0 if success else 1)


if __name__ == "__main__":
    main()
