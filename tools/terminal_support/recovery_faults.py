# SPDX-License-Identifier: Apache-2.0
"""Trusted QEMU blkdebug rules. Counts refer to the volume's write/flush boundary.

`rules(events, cut)` arms one EIO at the `cut`-th event of an expected write/flush
sequence. The `set-state` chain is a subsequence matcher: each state advances
only on the next expected event kind, so reads and events of the other kind in
between are ignored. The error is injected once and the request never reaches
the image; every earlier write already sits in the host page cache the host
reader sees. With a caller that fences on the error, the image is "fail-stop
after event cut-1". `EVENTS`/`CUTS` are the v5 terminal replacement's sequence.
"""
EVENTS = ["write_aio"] * 2 + ["flush_to_disk"] + ["write_aio"] * 11 + ["flush_to_disk", "write_aio", "flush_to_disk"]
CUTS = {"data": 0, "receipt": 3, "metadata": 10, "header": 15, "final_flush": 16}
KINDS = ("write_aio", "flush_to_disk")


def rules(events, cut):
    """blkdebug configuration that fails event `cut` of `events` with EIO once."""
    events = list(events)
    if not events or any(event not in KINDS for event in events):
        raise ValueError("unsupported blkdebug event sequence")
    if type(cut) is not int or not 0 <= cut < len(events):
        raise ValueError("unsupported filesystem cut")
    chain = []
    for index, event in enumerate(events[:cut]):
        chain.append(f'[set-state]\nevent = "{event}"\nstate = "{index+1}"\nnew_state = "{index+2}"\n')
    chain.append(f'[inject-error]\nevent = "{events[cut]}"\nstate = "{cut+1}"\nerrno = "5"\nonce = "on"\n')
    return "\n".join(chain)


def configuration(cut):
    """The v5 replacement's rules; unchanged output for every cut in `CUTS`."""
    return rules(EVENTS, cut)
