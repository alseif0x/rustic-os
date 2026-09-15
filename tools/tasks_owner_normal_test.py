# SPDX-License-Identifier: Apache-2.0
"""Prove the second tasks client works, and its failure cuts do not exist, in an
ordinary image."""
import argparse
import environment
from boot_support.image import build
from terminal_support.tasks_owner import verify_normal

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image')
    parser.add_argument('--output', default=str(environment.ROOT / 'artifacts/tasks-owner-normal'))
    args = parser.parse_args()
    # `terminal-init` is an ordinary build (only `terminal-test` carries the
    # acceptance profile) that initializes the fresh disposable volume.
    verify_normal(args.image or build('terminal-init'), args.output)
