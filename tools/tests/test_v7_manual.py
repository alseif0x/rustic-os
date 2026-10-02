# SPDX-License-Identifier: Apache-2.0
"""Refuse misleading default-terminal boot provenance before starting a VM."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import environment
from terminal_support.v7_manual import identities


class ManualBootIdentity(unittest.TestCase):
    def test_wrong_mode_acceptance_build_and_different_kernel_are_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            images = []
            reports = []
            for mode in ("terminal-init", "terminal"):
                directory = Path(temporary) / mode
                directory.mkdir()
                image = directory / "boot.img"
                image.write_bytes(mode.encode())
                (directory / "kernel.elf").write_bytes(b"same kernel")
                report = {"mode": mode, "tasks_acceptance": False, "build_id": "0123456789abcdef",
                          "image_sha256": environment.digest(image),
                          "kernel_sha256": environment.digest(directory / "kernel.elf")}
                (directory / "image.json").write_text(json.dumps(report))
                images.append(image)
                reports.append(report)
            self.assertEqual(identities(*images)[0]["build_id"], "0123456789abcdef")
            for field, value in (("mode", "terminal-init"), ("tasks_acceptance", True),
                                 ("build_id", "1123456789abcdef"), ("build_id", "")):
                bad = dict(reports[1], **{field: value})
                (images[1].parent / "image.json").write_text(json.dumps(bad))
                with self.subTest(field=field), self.assertRaises(ValueError):
                    identities(*images)
            kernel = images[1].parent / "kernel.elf"
            kernel.write_bytes(b"different kernel")
            bad = dict(reports[1], kernel_sha256=environment.digest(kernel))
            (images[1].parent / "image.json").write_text(json.dumps(bad))
            with self.assertRaises(ValueError):
                identities(*images)


if __name__ == "__main__":
    unittest.main()
