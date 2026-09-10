# SPDX-License-Identifier: Apache-2.0
"""Exercise real delayed FLUSH completion and timeout through private Unix NBD."""
import argparse
from datetime import datetime, timezone
from pathlib import Path
import uuid

import environment
from boot_support.image import build
from terminal_support.latency import verify


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", type=Path, help="existing recovery-test image; otherwise build it")
    parser.add_argument("--output", type=Path, help="new evidence directory, never overwrite an earlier run")
    parser.add_argument("--timeout", type=float, default=60, help="bounded UART wait in seconds")
    args = parser.parse_args()
    if not 1 <= args.timeout <= 120:
        parser.error("timeout must be between 1 and 120 seconds")
    output = args.output or environment.ROOT / "artifacts/latency" / (
        datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-") + uuid.uuid4().hex[:8])
    image = args.image or build("recovery-test")
    verify(image, output, args.timeout)


if __name__ == "__main__":
    main()
