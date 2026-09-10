# SPDX-License-Identifier: Apache-2.0
"""Trusted QEMU blkdebug rules. Counts refer to the volume's write/flush boundary."""
EVENTS = ["write_aio"] * 2 + ["flush_to_disk"] + ["write_aio"] * 11 + ["flush_to_disk", "write_aio", "flush_to_disk"]
CUTS = {"data": 0, "receipt": 3, "metadata": 10, "header": 15, "final_flush": 16}

def configuration(cut):
    if type(cut) is not int or not 0 <= cut < len(EVENTS):
        raise ValueError("unsupported filesystem cut")
    rules = []
    for index, event in enumerate(EVENTS[:cut]):
        rules.append(f'[set-state]\nevent = "{event}"\nstate = "{index+1}"\nnew_state = "{index+2}"\n')
    rules.append(f'[inject-error]\nevent = "{EVENTS[cut]}"\nstate = "{cut+1}"\nerrno = "5"\nonce = "on"\n')
    return "\n".join(rules)
