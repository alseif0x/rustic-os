# SPDX-License-Identifier: Apache-2.0
"""Install/verify the pinned Ubuntu boot tools; no implicit installation."""
import argparse
import hashlib
from pathlib import Path
import shlex
import subprocess
import tomllib
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
OS_RELEASE = Path("/etc/os-release")


def os_release(path=OS_RELEASE):
    """The declared fields of an os-release(5) file.

    Those values use that format's shell-like quoting, so the standard shlex
    parser reads them instead of ad-hoc quote stripping.
    """
    try:
        text = path.read_text()
    except OSError as error:
        raise RuntimeError(f"cannot read {path}: {error}") from error
    values = {}
    for line in text.splitlines():
        name, separator, value = line.partition("=")
        name = name.strip()
        if not separator or not name or name.startswith("#"):
            continue
        try:
            values[name] = " ".join(shlex.split(value))
        except ValueError as error:
            raise RuntimeError(f"unreadable value for {name} in {path}") from error
    return values


def host_release(path=OS_RELEASE):
    """The Ubuntu release the reviewed baselines are keyed by."""
    values = os_release(path)
    if values.get("ID") != "ubuntu":
        raise RuntimeError(
            f"the reference environment is Ubuntu; this host reports {values.get('ID', 'no ID')}"
        )
    if not values.get("VERSION_ID"):
        raise RuntimeError(f"{path} does not declare VERSION_ID")
    return values["VERSION_ID"]


def resolve(document, release):
    """Flatten one reviewed baseline into the shared configuration.

    Package versions and firmware hashes differ per Ubuntu release, so they are
    stored per release and selected here. An unreviewed release is refused: the
    caller must add its measured versions rather than accept whatever apt offers.
    """
    baselines = document.get("baselines")
    if not isinstance(baselines, dict) or not baselines:
        raise RuntimeError("the environment document declares no reviewed baseline")
    if release not in baselines:
        reviewed = ", ".join(sorted(baselines))
        raise RuntimeError(
            f"no reviewed baseline for Ubuntu {release}; reviewed releases: {reviewed}"
        )
    shared = {name: value for name, value in document.items() if name != "baselines"}
    return {**shared, "release": release, **baselines[release]}


# The pinned versions are needed by every consumer, so the baseline is selected
# when this module loads: an unreviewed host release refuses here, not later.
CONFIG = resolve(
    tomllib.loads((ROOT / "tools/environment.toml").read_text()), host_release()
)


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
    print(f"reviewed baseline: Ubuntu {CONFIG['release']}")
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
