# SPDX-License-Identifier: Apache-2.0
"""Owner revocation during submitted logical publication, with independent disk proof."""
import time
from .cases import pid, exited, counters
from .authority_cases import fence
from .oracle import snapshot
from .operation_cases import lookup, check


def verify(session, owned_disk, temporary, mount, base, workspace, resource, previous):
    cases = []
    content = b"reply deliberately unobserved"
    for name, skip, committed in (("data", 0, False), ("before_header", 14, False),
                                  ("header", 15, True), ("final_flush", 16, True)):
        name = "controlled_operation_" + name
        with owned_disk(temporary / (name + ".raw"), True, evidence_name=name + "-reboot") as data:
            with data.open("r+b") as stream:
                stream.write(base)
            with session(mount, data, name) as uart:
                baseline = counters(uart)
                uart.command(f"hold-io {skip} 400", "diagnostic armed")
                child = pid(uart, "run lost-operation alpha/item other")
                deadline = time.monotonic() + 8
                while "held=1" not in uart.command("io-status"):
                    if time.monotonic() >= deadline:
                        raise AssertionError("missing admitted logical publication command")
                    time.sleep(.02)
                uart.command("mem", "pending_io=1")
                uart.command(f"revoke {child}", "access=requested")
                pending = uart.command(f"revocation {child}", "effects=unknown")
                if "access=fenced" in pending:
                    raise AssertionError("revocation acknowledged settlement while disk was held")
                uart.command("io-status", "held=1")
                uart.command("echo control-during-publication", "control-during-publication")
                fence(uart, child, "access=fenced members=1 discarded_staging=0 effects=settled", request=False)
                exited(uart, child, 1, 0)
                uart.command(f"reap {child}", "code=0")
                uart.command("cat alpha/item", content.decode() if committed else "before")
                if committed:
                    original = lookup(uart, workspace, 1)
                    check(original, workspace, resource, previous, 1, 77, content, snapshot(data)[1])
                else:
                    uart.command(f"operation {workspace} e_0000000000000001 k_000000000000004d", "OutcomeUnknown")
                if counters(uart) != baseline:
                    raise AssertionError("controlled publication leaked native resources")
            with session(mount, data, name + "-reboot") as uart:
                uart.command("cat alpha/item", content.decode() if committed else "before")
                uart.command("cat beta/item", "before")
                uart.command("cat other", "untouched")
                if committed:
                    if lookup(uart, workspace, 1) != original:
                        raise AssertionError("late revocation changed the retained receipt after reboot")
                else:
                    uart.command(f"operation {workspace} e_0000000000000001 k_000000000000004d", "OutcomeUnknown")
            _, state = snapshot(data)
            node = state["nodes"][int(resource[-8:], 16)]
            if node["content"] != (content if committed else b"before") or len(state["records"]) != int(committed):
                raise AssertionError("independent file/operation oracle contradicts cancellation boundary")
            if committed:
                check(original, workspace, resource, previous, 1, 77, content, state)
            elif node["version"] != previous:
                raise AssertionError("early revocation changed the file version")
            cases.append({"case": name, "verified": True, "skip": skip, "committed": committed,
                          "revoked_while_pending": True, "ack_after_settlement": True,
                          "reboot_verified": True, "sha256": state["selected_sha256"]})
    return cases
