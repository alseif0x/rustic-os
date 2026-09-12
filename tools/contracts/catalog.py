# SPDX-License-Identifier: Apache-2.0
"""Load reviewed schemas offline and derive neutral tool descriptors."""
import hashlib
import json
from pathlib import Path

from jsonschema import Draft202012Validator
from referencing import Registry, Resource


ROOT = Path(__file__).resolve().parents[2] / "contracts/services/v1"


class Catalog:
    def __init__(self, root=ROOT):
        self.root = root
        self.manifest = json.loads((root / "catalog.json").read_text())
        self.version = self.manifest['version']
        if type(self.version) is not int or self.version not in (1, 2):
            raise ValueError('unsupported service contract version')
        prefix = f'urn:rusticos:services:v{self.version}:'
        self.schemas = {}
        for path in sorted(root.glob("*.schema.json")):
            schema = json.loads(path.read_text())
            Draft202012Validator.check_schema(schema)
            if schema["$id"] in self.schemas:
                raise ValueError("duplicate schema identity")
            self.schemas[schema["$id"]] = schema
        # No retrieve callback: unknown references never trigger network/file I/O.
        self.registry = Registry().with_resources(
            (uri, Resource.from_contents(schema))
            for uri, schema in self.schemas.items()
        )
        self.entries = {}
        for entry in self.manifest["operations"]:
            name = entry["name"]
            if name in self.entries:
                raise ValueError("duplicate method")
            schema = json.loads((root / entry["schema"]).read_text())
            if schema["$id"] != prefix + name:
                raise ValueError("method/schema identity mismatch")
            self.entries[name] = (entry, schema)
            self.expanded(name)  # Resolve all referenced contract types now.
        declared = self.schemas[prefix + 'types']["$defs"]["method"]["enum"]
        if set(declared) != set(self.entries) or len(declared) != len(self.entries):
            raise ValueError("catalog and method enum diverge")

    def validator(self, method, part):
        schema = self.entries[method][1]
        return Draft202012Validator(
            {"$ref": schema["$id"] + "#/$defs/" + part},
            registry=self.registry,
        )

    def expanded(self, method):
        schema = self.entries[method][1]
        resolver = self.registry.resolver(schema["$id"])

        def expand(value, current, seen=()):
            if isinstance(value, list):
                return [expand(item, current, seen) for item in value]
            if not isinstance(value, dict):
                return value
            if "$ref" in value:
                if len(value) != 1:
                    raise ValueError("descriptor exporter requires standalone refs")
                reference = value["$ref"]
                if reference in seen:
                    raise ValueError("recursive descriptors are outside v1")
                resolved = current.lookup(reference)
                return expand(resolved.contents, resolved.resolver, (*seen, reference))
            return {key: expand(item, current, seen) for key, item in value.items()}

        return {part: expand(schema["$defs"][part], resolver)
                for part in ("input", "output", "request", "response")}

    def digest(self, method):
        bundle = {"entry": self.entries[method][0], "schemas": self.expanded(method)}
        content = json.dumps(bundle, sort_keys=True,
                             separators=(",", ":"), ensure_ascii=True).encode("ascii")
        return hashlib.sha256(content).hexdigest()

    def descriptors(self):
        result = []
        for method, (entry, schema) in self.entries.items():
            expanded = self.expanded(method)
            result.append({
                "name": method, "version": self.manifest["version"],
                "description": entry["description"],
                "inputSchema": expanded["input"], "outputSchema": expanded["output"],
                "responseSchema": expanded["response"],
                "contract_id": schema["$id"], "contract_sha256": self.digest(method),
                "required_actions": entry["actions"], "effect": entry["effect"],
            })
        return result
