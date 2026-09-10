# SPDX-License-Identifier: Apache-2.0
"""Build and verify the native terminal using the same reference driver as CI."""
from boot_support.image import build
from terminal_support.acceptance import verify
import environment
if __name__ == "__main__":
    verify(build("terminal-test"),output=environment.ROOT / "artifacts/terminal-test")
