# SPDX-License-Identifier: Apache-2.0
"""Verify V5 admission limits with a disposable terminal data volume."""
from boot_support.image import build
from terminal_support.capacity_topology import verify
import environment


if __name__ == "__main__":
    initialize = build("terminal-init")
    verify(
        build("terminal"),
        initialize_image=initialize,
        output=environment.ROOT / "artifacts/boot/terminal-capacity-topology",
    )
