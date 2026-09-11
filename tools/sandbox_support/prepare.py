# SPDX-License-Identifier: Apache-2.0
"""Provision only reviewed infrastructure; candidate jobs never build this image."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
STATE = ROOT / ".cache/sandbox-image.json"
BASE = "ubuntu@sha256:224a1869083a311ef3f13648a154ba79832fbef6364d31493642ca03082da254"
PACKAGE_SOURCES = {"default": "", "github": "http://azure.archive.ubuntu.com/ubuntu"}


def prepare(package_source="default"):
    if package_source not in PACKAGE_SOURCES:
        raise ValueError("package source must be default or github")
    mirror = PACKAGE_SOURCES[package_source]
    with tempfile.TemporaryDirectory(prefix="rustic-toolchain-") as temporary:
        context = Path(temporary)
        reference = context / "reference"
        reference.mkdir()
        for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "LICENSE"):
            shutil.copyfile(ROOT / name, reference / name)
        for name in ("kernel", "crates", "apps", "licenses", ".cargo", "tools/xtask", "tools/boot_support", "tools/terminal_support"):
            shutil.copytree(ROOT / name, reference / name, ignore=shutil.ignore_patterns("__pycache__"))
        for name in ("environment.py", "environment.toml", "application.py"):
            shutil.copyfile(ROOT / "tools" / name, reference / "tools" / name)
        # The trusted UART harness shares stdlib read/replacement vectors, not the host validator.
        for name in ("tools/contracts/__init__.py", "tools/contracts/read_vectors.py",
                     "contracts/services/v1/fixtures/read-ranges.json", "contracts/services/v1/fixtures/replace-cases.json"):
            destination = reference / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, destination)
        shutil.copytree(ROOT / "tools/sandbox_support/container", context / "controller",
                        ignore=shutil.ignore_patterns("__pycache__"))
        shutil.copyfile(context / "controller/Dockerfile", context / "Dockerfile")
        fingerprint = hashlib.sha256()
        fingerprint.update(b"package-source\0" + package_source.encode() + b"\0" + mirror.encode() + b"\0")
        for path in sorted(context.rglob("*")):
            if path.is_file():
                fingerprint.update(str(path.relative_to(context)).encode() + b"\0" + path.read_bytes())
        tag = "rusticos-runner:" + fingerprint.hexdigest()[:16]
        subprocess.run(["docker", "build", "--build-arg", "BASE=" + BASE,
                        "--build-arg", "UBUNTU_MIRROR=" + mirror, "-t", tag, str(context)],
                       check=True, timeout=900, stdout=__import__("sys").stderr)
        image = subprocess.check_output(["docker", "image", "inspect", "--format", "{{.Id}}", tag], text=True).strip()
    result = {"status": "prepared", "image": image, "base": BASE, "infrastructure_sha256": fingerprint.hexdigest(),
              "package_source": package_source, "ubuntu_mirror": mirror or None}
    STATE.parent.mkdir(exist_ok=True)
    STATE.write_text(json.dumps(result, indent=2) + "\n")
    return result
