# SPDX-License-Identifier: Apache-2.0
"""Scheduled failure cuts over the actual native service and independent disk oracle."""
from . import late, loss, restart, device, pressure


def verify(session, owned_disk, temporary, image, mount):
    cases, base = late.verify(session, owned_disk, temporary, image, mount)
    cases.extend(loss.verify(session, owned_disk, temporary, image, mount))
    cases.extend(restart.verify(session, owned_disk, temporary, image, mount))
    cases.extend(device.verify(session, owned_disk, temporary, mount, base))
    cases.extend(pressure.verify(session, owned_disk, temporary, image, mount))
    return cases
