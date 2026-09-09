# SPDX-License-Identifier: Apache-2.0
"""Create local unreachable test commits without editing the checkout or a branch."""
import os
from pathlib import Path
import subprocess
import tempfile

from .prepare import ROOT


def candidate(parent, contents):
    with tempfile.TemporaryDirectory(prefix="rusticos-fixture-") as temporary:
        env = {**os.environ, "GIT_INDEX_FILE": str(Path(temporary) / "index")}

        def git(*args, data=None):
            return subprocess.check_output(["git", *args], cwd=ROOT, env=env, input=data, text=True).strip()

        git("read-tree", parent)
        blob = git("hash-object", "-w", "--stdin", data=contents)
        git("update-index", "--add", "--cacheinfo", "100644", blob, "kernel/build.rs")
        tree = git("write-tree")
        return git("-c", "user.name=RusticOS fixture", "-c", "user.email=fixture@invalid",
                   "commit-tree", tree, "-p", parent, "-m", "Local sandbox failure fixture")
