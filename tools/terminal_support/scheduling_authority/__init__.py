# SPDX-License-Identifier: Apache-2.0
"""Native scheduling guards: human edits and independently bound client authority."""
from . import human, denials


def verify(session, owned_disk, temporary, image, mount):
    return [human.verify(session, owned_disk, temporary, image, mount),
            *denials.verify(session, owned_disk, temporary, image, mount)]
