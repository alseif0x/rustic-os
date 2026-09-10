# SPDX-License-Identifier: Apache-2.0
"""Build a separate no_std executable and encode its bounded admission manifest."""
import os
from pathlib import Path
import re
import struct
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
FIELDS = {"schema", "identity", "executable", "version", "process_abi", "ipc_version", "requests"}
CAPABILITIES = {"ipc": 1, "diagnostic": 2, "block": 4, "console": 8, "control": 16}


def encode(document):
    if set(document) != FIELDS:
        raise ValueError("manifest fields must match schema 1")
    if any(type(document[key]) is not int for key in ("schema", "process_abi", "ipc_version")) or document["schema"] != 1 or document["process_abi"] != 65536 or document["ipc_version"] != 1:
        raise ValueError("unsupported manifest or ABI version")
    def name(key):
        value = document[key]
        if not isinstance(value, str) or not re.fullmatch(r"[a-z][a-z0-9._-]{0,30}", value):
            raise ValueError("invalid " + key)
        return value.encode().ljust(32, b"\0")
    identity, executable = name("identity"), name("executable")
    if not document["executable"].endswith(".elf"):
        raise ValueError("executable must be a single ELF filename")
    version = document["version"]
    if not isinstance(version, list) or len(version) != 3 or any(type(v) is not int or not 0 <= v <= 65535 for v in version):
        raise ValueError("version must contain three u16 integers")
    requests = document["requests"]
    if not isinstance(requests, list) or any(not isinstance(v, str) or v not in CAPABILITIES for v in requests) or len(set(requests)) != len(requests):
        raise ValueError("invalid capability requests")
    bits = sum(CAPABILITIES[v] for v in requests)
    return struct.pack("<8sHHIHHHHQ32s32s32s", b"RUSTAPP\0", 1, 128, 65536, 1, *version, bits, identity, executable, bytes(32))


def build_one(root, env, offline, name, manifest_name):
    env = (os.environ if env is None else env).copy()
    descriptor = root / ("apps/" + name + "/app.toml")
    if descriptor.stat().st_size > 4096:
        raise ValueError("manifest text exceeds 4096 bytes")
    document = tomllib.loads(descriptor.read_text())
    manifest = encode(document)
    command = ["cargo", "build", "-p", "rustic-" + name, "--features", "native",
               "--target", "x86_64-unknown-none", "--release", "--locked"]
    if offline:
        command.append("--offline")
    subprocess.run(command, cwd=root, env=env, check=True)
    target = Path(env.get("CARGO_TARGET_DIR", root / "target"))
    if not target.is_absolute():
        target = root / target
    elf = target / ("x86_64-unknown-none/release/" + name)
    if not 64 <= elf.stat().st_size <= 1024 * 1024:
        raise ValueError("application ELF exceeds loader budget")
    output = target / "native"
    output.mkdir(exist_ok=True)
    for name, content in ((document["executable"], elf.read_bytes()), (manifest_name, manifest)):
        destination = output / name
        if not destination.is_file() or destination.read_bytes() != content:
            destination.write_bytes(content)
    return output.resolve()


def build(root=ROOT, env=None, offline=False):
    output = build_one(root, env, offline, "sdk-probe", "app.manifest")
    build_one(root, env, offline, "block-probe", "block-probe.manifest")
    for name in ("file-server", "supervisor", "shell", "utility"):
        build_one(root, env, offline, name, name + ".manifest")
    return output


if __name__ == "__main__":
    print(build())
