# SPDX-License-Identifier: Apache-2.0
"""H1 capacity and invalid-path evidence from real UART and independent disk reads."""
from .oracle import snapshot


def unchanged(before, after, case):
    if before != after:
        raise AssertionError(f"{case}: rejected request changed the selected disk bytes")


def rejected(uart, data, command, error):
    before, state = snapshot(data)
    output = uart.command(command, f"error: {error}")
    # Duplicate or conflicting errors cannot satisfy the expected denial.
    errors = [line for line in output.splitlines() if line.startswith("error:")]
    if errors != [f"error: {error}"]:
        raise AssertionError(f"ambiguous storage failure: {output!r}")
    after, observed = snapshot(data)
    unchanged(before, after, command)
    return {"command": command, "error": error, "sequence": state["sequence"],
            "before_sha256": state["selected_sha256"],
            "after_sha256": observed["selected_sha256"]}


def exercise(uart, data):
    # The preceding manual cases have reclaimed their temporary directory.
    _, initial = snapshot(data)
    expected = {(4, "hello"): b"Hello from native Rust",
                (3, "owner-policy"): b"rustic-owner-v1\nhelpers=explicit\n"}
    if initial["files"] != expected:
        raise AssertionError("unexpected H1 storage fixture")
    # Four directory roots and these two files leave 26 free object slots.
    for index in range(26):
        uart.command(f"touch quota-{index}")
    _, full = snapshot(data)
    full_files = expected | {(4, f"quota-{index}"): b"" for index in range(26)}
    if full["files"] != full_files:
        raise AssertionError("capacity fixture did not fill the volume")
    failures = [rejected(uart, data, "touch overflow", "Full")]
    uart.command("cat overflow", "error: NotFound")
    uart.command("cat hello", "Hello from native Rust")
    # Prove a released slot is usable immediately, not only after all cleanup.
    uart.command("rm quota-0")
    uart.command("write reclaimed slot-reused", "written 11 bytes")
    _, reused = snapshot(data)
    full_files.pop((4, "quota-0"))
    full_files[(4, "reclaimed")] = b"slot-reused"
    if reused["files"] != full_files:
        raise AssertionError("released capacity was not safely reusable")
    uart.command("rm reclaimed")
    for index in range(1, 26):
        uart.command(f"rm quota-{index}")
    for command, error in (("touch " + "x" * 32, "Invalid"),
                           ("cat hello/..", "NotDirectory"),
                           ("touch missing/child", "NotFound"),
                           ("touch /system/denied", "ReadOnly")):
        failures.append(rejected(uart, data, command, error))
    _, final = snapshot(data)
    if final["files"] != expected:
        raise AssertionError("failed requests changed existing data or leaked files")
    return {"verified": True, "backend": "native_uart_with_disk_oracle",
            "object_limit": 32, "temporary_files": 26, "slot_reused": True,
            "failures": failures, "final_file_count": len(final["files"]),
            "final_sha256": final["selected_sha256"]}
