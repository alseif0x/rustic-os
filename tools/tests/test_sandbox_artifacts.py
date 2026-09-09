# SPDX-License-Identifier: Apache-2.0
"""An artifact is untrusted data, never a host filesystem instruction."""
import io
from pathlib import Path
import sys
import tarfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from sandbox_support.artifacts import unpack_single


def archive(names, kind=tarfile.REGTYPE):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w") as tar:
        for name in names:
            member = tarfile.TarInfo(name)
            member.type = kind
            member.linkname = "/etc/passwd" if kind != tarfile.REGTYPE else ""
            member.size = 4 if kind == tarfile.REGTYPE else 0
            tar.addfile(member, io.BytesIO(b"data") if member.size else None)
    return output.getvalue()


class ArtifactContract(unittest.TestCase):
    def test_regular_bounded_file(self):
        self.assertEqual(unpack_single(archive(["kernel.elf"]), "kernel.elf", 4), b"data")

    def test_host_paths_and_multiple_members_rejected(self):
        for names in (["../kernel.elf"], ["/kernel.elf"], ["other"], ["kernel.elf", "extra"]):
            with self.subTest(names=names), self.assertRaises(RuntimeError):
                unpack_single(archive(names), "kernel.elf", 4)

    def test_links_and_special_files_rejected(self):
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.DIRTYPE, tarfile.FIFOTYPE):
            with self.subTest(kind=kind), self.assertRaises(RuntimeError):
                unpack_single(archive(["kernel.elf"], kind), "kernel.elf", 4)

    def test_oversize_rejected(self):
        with self.assertRaises(RuntimeError):
            unpack_single(archive(["kernel.elf"]), "kernel.elf", 3)
