# SPDX-License-Identifier: Apache-2.0
"""Run the second native tasks client against a disposable native volume."""
import argparse
import environment
from boot_support.image import build
from terminal_support.tasks_owner import verify

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image')
    parser.add_argument('--output', default=str(environment.ROOT / 'artifacts/tasks-owner-test'))
    args = parser.parse_args()
    verify(args.image or build('terminal-test'), args.output)
