# SPDX-License-Identifier: Apache-2.0
"""Build an ELF and a disposable FAT32 UEFI volume; never mount host disks."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile

import environment

ROOT = environment.ROOT
OUTPUT = ROOT / "artifacts/boot"


def source_id():
    digest = hashlib.sha256()
    paths = sorted((ROOT / "kernel").rglob("*.rs"))
    paths += [ROOT / name for name in ("kernel/linker.ld", "Cargo.toml", "Cargo.lock", "kernel/Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml")]
    for path in paths:
        digest.update(str(path.relative_to(ROOT)).encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
    return digest.hexdigest()[:16]


def build(mode):
    if mode not in ("ok", "panic", "hang", "invalid"):
        raise ValueError("unsupported fixture")
    environment.verify()
    environment.fetch_bootloader()
    build_id = source_id()
    env = os.environ.copy()
    env["RUSTIC_BUILD_ID"] = build_id
    subprocess.run(
        ["cargo", "build", "-p", "rustic-kernel", "--bin", "rustic-os", "--features",
         "boot-image", "--target", "x86_64-unknown-none", "--release", "--locked"],
        cwd=ROOT, env=env, check=True,
    )
    kernel = ROOT / "target/x86_64-unknown-none/release/rustic-os"
    provenance = {
        "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "source_status": subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True),
        "rustc": subprocess.check_output(["rustc", "--version", "--verbose"], text=True),
    }
    return package(kernel, mode, build_id, provenance)


def package(kernel, mode, build_id, provenance):
    """Package a prebuilt ELF using trusted reference files, without compiling."""
    if mode not in ("ok", "panic", "hang", "invalid"):
        raise ValueError("unsupported fixture")
    directory = OUTPUT / mode
    directory.mkdir(parents=True, exist_ok=True)
    header = kernel.read_bytes()[:64]
    if header[:6] != b"\x7fELF\x02\x01" or header[16:20] != b"\x02\x00\x3e\x00":
        raise RuntimeError("expected a static x86_64 ELF executable")
    archive = ROOT / ".cache/limine-12.8.0-binary.tar.gz"
    image = directory / "rustic-os.img"
    # Only fixed, verified members are read; no archive paths are extracted.
    with tempfile.TemporaryDirectory(prefix="rustic-image-") as temporary:
        stage = Path(temporary)
        with tarfile.open(archive, "r:gz") as bundle:
            for name, destination in [("BOOTX64.EFI", "BOOTX64.EFI"), ("LICENSE", "LIMINE.txt")]:
                member = bundle.getmember("limine-binary/" + name)
                if not member.isfile() or member.size > 16 * 1024 * 1024:
                    raise RuntimeError("unexpected bootloader archive member")
                with bundle.extractfile(member) as source:
                    (stage / destination).write_bytes(source.read())
        shutil.copyfile(kernel, stage / "kernel.elf")
        (stage / "limine.conf").write_text(
            "timeout: 0\nserial: yes\n/RusticOS\n    protocol: limine\n"
            f"    path: boot():/kernel.elf\n    cmdline: mode={mode}\n"
        )
        for name in ("limine-rust-MIT.txt", "bitflags-MIT.txt", "rust-MIT.txt"):
            shutil.copyfile(ROOT / "licenses" / name, stage / name)
        shutil.copyfile(ROOT / "LICENSE", stage / "RUSTIC.txt")
        # Create a new temporary regular file; never format an existing user path.
        volume = stage / "volume.img"
        subprocess.run(["mformat", "-i", str(volume), "-C", "-F", "-T", "131072",
                        "-N", "0x52555354", "-v", "RUSTICOS", "::"], check=True)
        subprocess.run(["mmd", "-i", str(volume), "::/EFI", "::/EFI/BOOT", "::/licenses"], check=True)
        for name in ("kernel.elf", "limine.conf"):
            subprocess.run(["mcopy", "-i", str(volume), str(stage / name), "::/" + name], check=True)
        subprocess.run(["mcopy", "-i", str(volume), str(stage / "BOOTX64.EFI"), "::/EFI/BOOT/BOOTX64.EFI"], check=True)
        for license_file in stage.glob("*.txt"):
            subprocess.run(["mcopy", "-i", str(volume), str(license_file), "::/licenses/" + license_file.name], check=True)
        shutil.copyfile(volume, image)
    shutil.copyfile(kernel, directory / "kernel.elf")
    metadata = {
        "mode": mode, "build_id": build_id,
        **provenance,
        "kernel_sha256": environment.digest(kernel),
        "image_sha256": environment.digest(image),
        "environment": environment.CONFIG,
    }
    (directory / "image.json").write_text(json.dumps(metadata, indent=2) + "\n")
    return image
