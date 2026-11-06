#!/usr/bin/env python3
"""Initialize, validate, and render JSON changelogs; Python 3.9+, no dependencies."""
import argparse
import json
import re
import sys
from pathlib import Path

SCHEMA = Path(__file__).resolve().parents[1] / "changelog.schema.json"


def validate_schema(value, schema, where="$"):
    # Implements every keyword used by the bundled schema; not a general JSON Schema engine.
    types = {"array": list, "object": dict, "string": str}
    if "type" in schema and not isinstance(value, types[schema["type"]]):
        raise ValueError(f"{where}: expected {schema['type']}")
    if "enum" in schema and value not in schema["enum"]:
        raise ValueError(f"{where}: expected one of {schema['enum']}")
    if isinstance(value, dict):
        missing = set(schema.get("required", [])) - value.keys()
        if missing:
            raise ValueError(f"{where}: missing {sorted(missing)}")
        properties = schema.get("properties", {})
        if schema.get("additionalProperties") is False and value.keys() - properties.keys():
            raise ValueError(f"{where}: unknown fields {sorted(value.keys() - properties.keys())}")
        for key in value:
            if key in properties:
                validate_schema(value[key], properties[key], f"{where}.{key}")
    elif isinstance(value, list):
        if schema.get("uniqueItems") and len({json.dumps(x, sort_keys=True) for x in value}) != len(value):
            raise ValueError(f"{where}: duplicate items")
        for i, item in enumerate(value):
            validate_schema(item, schema.get("items", {}), f"{where}[{i}]")
    elif isinstance(value, str):
        if len(value.strip()) < schema.get("minLength", 0):
            raise ValueError(f"{where}: empty text")
        if "pattern" in schema and not re.fullmatch(schema["pattern"], value):
            raise ValueError(f"{where}: invalid format")


def validate(data):
    validate_schema(data, json.loads(SCHEMA.read_text()))
    versions = [tuple(map(int, r["version"].split("."))) for r in data]
    normalized = [v + (0,) * (3 - len(v)) for v in versions]
    if len(set(normalized)) != len(normalized):
        raise ValueError("versions must be unique (X.X and X.X.0 are equivalent)")
    if normalized != sorted(normalized, reverse=True):
        raise ValueError("releases must be newest-first")
    return data


def safe_data_path(path):
    path = path.resolve()
    if any(part in {"skills", ".skills"} for part in path.parts):
        raise ValueError("changelog data must be outside skills directories")
    return path


def inline(text):
    # Keep entries on one line and escape Markdown syntax from literal user-facing text.
    text = " ".join(text.split())
    return "".join("\\" + char if char in "\\`*_{}[]<>#|" else char for char in text)


def markdown(data):
    lines = ["# Changelog", ""]
    for release in data:
        heading = release["version"]
        if release.get("title"):
            heading += " — " + inline(release["title"])
        lines += ["## " + heading, ""]
        for kind, label in [("feature", "New features"), ("improvement", "Improvements"), ("fix", "Bug fixes")]:
            changes = [c for c in release["changes"] if c["type"] == kind]
            if not changes:
                continue
            lines += ["### " + label, ""]
            for change in changes:
                tags = " (" + ", ".join(map(inline, change["tags"])) + ")" if change.get("tags") else ""
                lines.append(f"- **{inline(change['title'])}** — {inline(change['description'])}{tags}")
            lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    init = commands.add_parser("init", help="Create data and sibling schema; never overwrite")
    init.add_argument("path", type=Path)
    init.add_argument("--version", required=True)
    check = commands.add_parser("validate", help="Validate shape and descending unique versions")
    check.add_argument("path", type=Path)
    render = commands.add_parser("markdown", help="Overwrite generated Markdown after validation")
    render.add_argument("path", type=Path)
    render.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    path = safe_data_path(args.path)
    if args.command == "init":
        data = validate([{"version": args.version, "changes": []}])
        schema_path = path.with_name("changelog.schema.json")
        if path == schema_path or path.exists() or schema_path.exists():
            raise ValueError("data/schema already exists; inspect and reuse it")
        path.parent.mkdir(parents=True, exist_ok=True)
        with schema_path.open("x") as out:
            out.write(SCHEMA.read_text())
        with path.open("x") as out:
            out.write(json.dumps(data, indent=2, ensure_ascii=False) + "\n")
    else:
        data = validate(json.loads(path.read_text()))
        if args.command == "markdown":
            output = args.output.resolve()
            if output == path or output == SCHEMA or output == path.with_name("changelog.schema.json"):
                raise ValueError("output must not overwrite JSON data/schema")
            output.write_text(markdown(data))
    print(f"{args.command}: OK ({path})")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError) as exc:
        sys.exit(f"error: {exc}")
