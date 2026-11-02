"""Envelope validation against schema/envelope.schema.json, strict.

The schema file is the source of truth; this module validates the JSON-schema
subset it uses (object/array/string/integer/number/boolean, required,
properties, additionalProperties, enum, items, minLength, minimum). An unknown
field, a missing required one or a wrong type rejects the envelope — the graph
then re-asks the agent in the same session, never a cold restart.
"""

import json
import os

SCHEMA_PATH = os.path.join(
    os.path.dirname(__file__), "..", "..", "..", "schema", "envelope.schema.json"
)


class EnvelopeError(ValueError):
    """Why an envelope was rejected, in one human-readable line."""


def load_schema(path=None):
    with open(path or SCHEMA_PATH, encoding="utf-8") as f:
        return json.load(f)


def _type_ok(value, expected):
    if expected == "object":
        return isinstance(value, dict)
    if expected == "array":
        return isinstance(value, list)
    if expected == "string":
        return isinstance(value, str)
    if expected == "integer":
        return isinstance(value, int) and not isinstance(value, bool)
    if expected == "number":
        return isinstance(value, (int, float)) and not isinstance(value, bool)
    if expected == "boolean":
        return isinstance(value, bool)
    return True


def _check(value, schema, path, errors):
    expected = schema.get("type")
    if expected and not _type_ok(value, expected):
        errors.append(f"{path or '<root>'}: expected {expected}")
        return
    if "enum" in schema and value not in schema["enum"]:
        errors.append(f"{path or '<root>'}: not one of {schema['enum']}")
        return
    if expected == "string" and "minLength" in schema:
        if len(value) < schema["minLength"]:
            errors.append(f"{path or '<root>'}: shorter than {schema['minLength']}")
    if expected in ("integer", "number") and "minimum" in schema:
        if value < schema["minimum"]:
            errors.append(f"{path or '<root>'}: below {schema['minimum']}")
    if expected == "object":
        props = schema.get("properties", {})
        for key in schema.get("required", []):
            if key not in value:
                errors.append(f"{path or '<root>'}: missing {key!r}")
        if schema.get("additionalProperties") is False:
            for key in value:
                if key not in props:
                    errors.append(f"{path or '<root>'}: unknown field {key!r}")
        for key, sub in props.items():
            if key in value:
                _check(value[key], sub, f"{path}.{key}" if path else key, errors)
    if expected == "array" and "items" in schema:
        for i, item in enumerate(value):
            _check(item, schema["items"], f"{path}[{i}]", errors)


def validate(envelope, schema=None):
    """Return the envelope unchanged, or raise EnvelopeError naming every defect."""
    schema = schema or load_schema()
    errors = []
    _check(envelope, schema, "", errors)
    if errors:
        raise EnvelopeError("; ".join(errors))
    return envelope
