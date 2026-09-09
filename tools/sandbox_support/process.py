# SPDX-License-Identifier: Apache-2.0
"""Bound subprocess duration and file output, including Docker client output."""
import resource
import subprocess


def command(args, log, timeout=30, stdin=None, limit=8 * 1024 * 1024):
    def limits():
        resource.setrlimit(resource.RLIMIT_FSIZE, (limit, limit))
    with log.open("wb") as output:
        return subprocess.run(args, stdin=stdin, stdout=output, stderr=subprocess.STDOUT,
                              timeout=timeout, preexec_fn=limits).returncode
