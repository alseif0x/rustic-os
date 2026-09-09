# SPDX-License-Identifier: Apache-2.0
"""Install/verify the pinned Ubuntu boot tools; no implicit installation."""
import argparse
import hashlib
from pathlib import Path
import subprocess
import tomllib
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
CONFIG = tomllib.loads((ROOT / "tools/environment.toml").read_text())


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def install():
    packages = [f"{name}={version}" for name, version in CONFIG["packages"].items()]
    subprocess.run(["sudo", "apt-get", "update"], check=True)
    subprocess.run(
        ["sudo", "apt-get", "install", "-y", "--no-install-recommends", *packages],
        check=True,
    )


def verify():
    for name, expected in CONFIG["packages"].items():
        actual = subprocess.check_output(
            ["dpkg-query", "-W", "-f=${Version}", name], text=True
        )
        if actual != expected:
            raise RuntimeError(f"{name}: expected {expected}, got {actual}")
        print(f"{name}={actual}")
    for name, expected in CONFIG["firmware"].items():
        if digest(Path(name)) != expected:
            raise RuntimeError(f"firmware hash mismatch: {name}")
        print(f"verified {name}")
    machines = subprocess.check_output(
        ["qemu-system-x86_64", "-machine", "help"], text=True
    )
    if CONFIG["machine"] not in {line.split()[0] for line in machines.splitlines() if line}:
        raise RuntimeError("reference QEMU machine unavailable")


def fetch_bootloader():
    config = CONFIG["bootloader"]
    directory = ROOT / ".cache"
    directory.mkdir(exist_ok=True)
    archive = directory / f"limine-{config['version']}-binary.tar.gz"
    if not archive.exists():
        temporary = archive.with_suffix(".download")
        try:
            with urllib.request.urlopen(config["url"], timeout=60) as source:
                with temporary.open("wb") as output:
                    while block := source.read(1024 * 1024):
                        output.write(block)
            if digest(temporary) != config["sha256"]:
                raise RuntimeError("bootloader archive hash mismatch")
            temporary.replace(archive)
        finally:
            temporary.unlink(missing_ok=True)
    if digest(archive) != config["sha256"]:
        raise RuntimeError("cached bootloader archive hash mismatch")
    print(f"verified {archive}")
    # Extraction and image creation belong to #8.


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["install", "verify", "fetch-bootloader"])
    options = parser.parse_args()
    {"install": install, "verify": verify, "fetch-bootloader": fetch_bootloader}[options.action]()
