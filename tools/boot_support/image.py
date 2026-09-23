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
import application
from .scenarios import MODES

ROOT = environment.ROOT
OUTPUT = ROOT / "artifacts/boot"
# RAM profile sizes the reference suite and the `--memory` flag accept. The
# address budget in kernel/src/arch/x86_64/memory/physical.rs is larger, and the
# manual `run` action plus the memory profile suite can use anything up to it.
MEMORY_PROFILES = (256, 512, 2048)
DEFAULT_MEMORY_MIB = 256
# The bitmap budget is 16 GiB (64 KiB of metadata per GiB).
MAX_MEMORY_MIB = 16384
# Fixtures whose harness owns its own QEMU invocation and pins the reference
# size. A profile other than the reference cannot be honored for these, so the
# builder refuses it instead of recording a memory_mib the boot never used.
V7_READ_MODE = "terminal-v7"
IMAGE_MODES = MODES + (V7_READ_MODE,)
PINNED_MEMORY_MODES = ("terminal-test", "recovery-test", V7_READ_MODE)


def memory_supported(mode, memory):
    """MiB profiles from the reference up to the bitmap budget, in 256 MiB steps."""
    declared = memory in MEMORY_PROFILES or (
        DEFAULT_MEMORY_MIB < memory <= MAX_MEMORY_MIB and memory % DEFAULT_MEMORY_MIB == 0
    )
    return declared and (memory == DEFAULT_MEMORY_MIB or mode not in PINNED_MEMORY_MODES)


def source_id(*, tasks_acceptance=False, memory=DEFAULT_MEMORY_MIB):
    digest = hashlib.sha256()
    digest.update(b"tasks-acceptance=1\0" if tasks_acceptance else b"tasks-acceptance=0\0")
    digest.update(f"memory-mib={memory}\0".encode())
    paths = sorted((ROOT / "kernel").rglob("*.rs"))
    paths += sorted((ROOT / "kernel").rglob("*.S"))
    paths += sorted((ROOT / "crates").rglob("*.rs"))
    paths += sorted((ROOT / "crates").rglob("Cargo.toml"))
    paths += sorted(path for path in (ROOT / "apps").rglob("*") if path.is_file())
    paths += [ROOT / "tools/application.py"]
    paths += [ROOT / name for name in ("kernel/linker.ld", "Cargo.toml", "Cargo.lock", "kernel/Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml")]
    for path in paths:
        digest.update(str(path.relative_to(ROOT)).encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
    return digest.hexdigest()[:16]


def build(mode, memory=DEFAULT_MEMORY_MIB):
    if mode not in IMAGE_MODES:
        raise ValueError("unsupported fixture")
    if not memory_supported(mode, memory):
        raise ValueError(
            f"{mode} pins the {DEFAULT_MEMORY_MIB} MiB reference size; "
            f"the {memory} MiB profile is not available for it"
        )
    environment.verify()
    environment.fetch_bootloader()
    tasks_acceptance = mode == "terminal-test"
    build_id = source_id(tasks_acceptance=tasks_acceptance, memory=memory)
    env = os.environ.copy()
    env["RUSTIC_BUILD_ID"] = build_id
    env["RUSTIC_APPLICATION_DIRECTORY"] = str(application.build(ROOT, env, tasks_acceptance=tasks_acceptance))
    subprocess.run(
        ["cargo", "build", "-p", "rustic-kernel", "--bin", "rustic-os", "--features",
         "sdk-test", "--target", "x86_64-unknown-none", "--release", "--locked"],
        cwd=ROOT, env=env, check=True,
    )
    kernel = ROOT / "target/x86_64-unknown-none/release/rustic-os"
    provenance = {
        "tasks_acceptance": tasks_acceptance,
        "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "source_status": subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True),
        "application_elf_sha256": environment.digest(Path(env["RUSTIC_APPLICATION_DIRECTORY"]) / "sdk-probe.elf"),
        "application_manifest_sha256": environment.digest(Path(env["RUSTIC_APPLICATION_DIRECTORY"]) / "app.manifest"),
        "block_application_elf_sha256": environment.digest(Path(env["RUSTIC_APPLICATION_DIRECTORY"]) / "block-probe.elf"),
        "block_application_manifest_sha256": environment.digest(Path(env["RUSTIC_APPLICATION_DIRECTORY"]) / "block-probe.manifest"),
        "native_applications": {name: {suffix: environment.digest(Path(env["RUSTIC_APPLICATION_DIRECTORY"]) / (name + suffix)) for suffix in (".elf", ".manifest")} for name in ("file-server", "supervisor", "shell", "utility", "tasks")},
        "rustc": subprocess.check_output(["rustc", "--version", "--verbose"], text=True),
        "memory_mib": memory,
    }
    return package(kernel, mode, build_id, provenance)


def image_directory(mode, memory=DEFAULT_MEMORY_MIB):
    """One directory per fixture and RAM profile: a non-reference profile never
    overwrites the reference `artifacts/boot/<mode>/` image and metadata."""
    name = mode if memory == DEFAULT_MEMORY_MIB else f"{mode}-{memory}"
    return OUTPUT / name


def package(kernel, mode, build_id, provenance):
    """Package a prebuilt ELF using trusted reference files, without compiling."""
    if mode not in IMAGE_MODES:
        raise ValueError("unsupported fixture")
    memory = provenance.get("memory_mib", DEFAULT_MEMORY_MIB)
    directory = image_directory(mode, memory)
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
        for name in ("limine-rust-MIT.txt", "bitflags-MIT.txt", "rust-MIT.txt",
                     "sha2-MIT.txt", "digest-MIT.txt", "block-buffer-MIT.txt",
                     "crypto-common-MIT.txt", "hybrid-array-MIT.txt", "typenum-MIT.txt",
                     "cpufeatures-MIT.txt", "cfg-if-MIT.txt"):
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
