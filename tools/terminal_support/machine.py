# SPDX-License-Identifier: Apache-2.0
"""Own only a dedicated regular data image and a single reference QEMU process."""
import contextlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import environment

SIZE = 4 * 1024 ** 3

@contextlib.contextmanager
def disk(path, initialize=False, upgrade_recovery=False, *, volume_tool=None):
    if initialize and upgrade_recovery:
        raise RuntimeError("initialize and upgrade are mutually exclusive")
    if upgrade_recovery:
        raise RuntimeError("legacy recovery upgrades are unavailable; terminal provisioning is V7 only")
    if volume_tool is None:
        volume_tool = environment.ROOT / "target/debug/rustic-volume"
    from .v7_disk import disk as v7_disk
    with v7_disk(path, initialize, volume_tool=volume_tool) as data:
        yield data

@contextlib.contextmanager
def machine(image, data, serial, log, fault=None, readonly=False, rules=None):
    """`fault` names a v5 replacement cut; `rules` is a prepared blkdebug
    configuration from `recovery_faults.rules` for another event sequence."""
    if fault is not None and rules is not None:
        raise ValueError("a boot takes either a v5 cut or prepared blkdebug rules")
    image = Path(image).resolve()
    metadata = json.loads((image.parent / "image.json").read_text())
    if environment.digest(image) != metadata["image_sha256"]:
        raise RuntimeError("boot image changed after construction")
    with tempfile.TemporaryDirectory(prefix="rustic-terminal-") as temporary:
        variables = Path(temporary) / "OVMF_VARS.fd"
        shutil.copyfile("/usr/share/OVMF/OVMF_VARS_4M.fd", variables)
        config = environment.CONFIG
        drive = str(data)
        if fault is not None:
            from .recovery_faults import configuration
            rules = configuration(fault)
        if rules is not None:
            if not isinstance(rules, str) or "[inject-error]" not in rules:
                raise ValueError("blkdebug rules must arm one injected error")
            path = Path(temporary) / "blkdebug.conf"
            path.write_text(rules)
            drive = f"blkdebug:{path}:{data}"
        data_options = f"if=none,id=rusticdata,format=raw,cache=writeback,file={drive}"
        if readonly:
            data_options += ",readonly=on"
        command = [
            "qemu-system-x86_64", "-machine", config["machine"], "-accel", config["accelerator"],
            "-cpu", config["cpu"], "-smp", "1", "-m", "256M",
            "-drive", "if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd",
            "-drive", f"if=pflash,format=raw,file={variables}",
            "-drive", f"if=virtio,format=raw,readonly=on,file={image}",
            "-drive", data_options,
            "-device", "virtio-blk-pci,drive=rusticdata,addr=0x6,disable-modern=on,disable-legacy=off,queue-size=8,num-queues=1,vectors=0,rerror=report,werror=report",
            "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
            "-display", "none", "-serial", serial, "-monitor", "none", "-nic", "none", "-no-reboot",
        ]
        if serial == "stdio":
            position=command.index("-serial")
            command[position:position+2]=["-chardev","stdio,id=rusticconsole,signal=off,mux=on","-serial","chardev:rusticconsole"]
        with Path(log).open("wb") as errors:
            process = subprocess.Popen(command, stderr=errors)
            try:
                yield process
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
