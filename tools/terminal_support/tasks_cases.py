# SPDX-License-Identifier: Apache-2.0
"""Separate native tasks consumer; read-only effects checked on the actual disk."""
from .cases import counters, pid
from .authority_cases import cleanup
from .oracle import snapshot


def write_document(uart, text):
    escaped = text.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\t", "\\t")
    uart.command(f'write tasks-fixture "{escaped}"', f"written {len(text)} bytes")


def observe(uart, data, command, expected, rows=()):
    before, state = snapshot(data)
    baseline = counters(uart)
    value = uart.command(command, expected)
    errors = [line for line in value.splitlines() if line.startswith("error:")]
    assert errors == ([expected] if expected.startswith("error:") else []), value
    actual_rows = [line for line in value.splitlines() if " [open] " in line or " [done] " in line]
    assert actual_rows == list(rows), value
    if errors:
        assert not any(line.endswith(" tasks") for line in value.splitlines()), value
    uart.command("status", "1" if expected.startswith("error:") else "0")
    after, observed = snapshot(data)
    assert before == after, (command, "read-only task changed disk")
    assert counters(uart) == baseline, (command, baseline, counters(uart))
    return {"command": command, "rows": len(rows), "sequence": state["sequence"],
            "before_sha256": state["selected_sha256"], "after_sha256": observed["selected_sha256"]}


def exercise(uart, data):
    baseline = counters(uart)
    _, initial = snapshot(data)
    results = []
    document = "rustic-tasks-v1\n7\topen\tReview kernel\n42\tdone\tBoot the OS\n"
    rows = ("7 [open] Review kernel", "42 [done] Boot the OS")
    write_document(uart, document)
    _, fixture = snapshot(data)
    assert fixture["files"][(4, "tasks-fixture")] == document.encode("ascii")
    for _ in range(3):
        results.append(observe(uart, data, "tasks list tasks-fixture", "2 tasks", rows))
    for command, error in (("tasks", "invalid arguments; type help"),
                           ("tasks add tasks-fixture", "invalid arguments; type help"),
                           ("tasks list tasks-fixture extra", "invalid arguments; type help"),
                           ("tasks list missing-tasks", "NotFound"),
                           ("tasks list /workspaces", "IsDirectory")):
        results.append(observe(uart, data, command, "error: " + error))
    first, second = pid(uart, "run spin"), pid(uart, "run spin")
    results.append(observe(uart, data, "tasks list tasks-fixture", "error: service busy or full"))
    cleanup(uart, first, second)
    results.append(observe(uart, data, "tasks list tasks-fixture", "2 tasks", rows))
    write_document(uart, "rustic-tasks-v1\n")
    results.append(observe(uart, data, "tasks list tasks-fixture", "0 tasks"))
    for malformed in ("rustic-tasks-v2\n", "rustic-tasks-v1\n7\topen\tValid\n7\tdone\tDuplicate\n",
                      "rustic-tasks-v1\n7\topen\tValid\nmalformed tail\n",
                      "rustic-tasks-v1\n0\topen\tZero ID\n",
                      "rustic-tasks-v1\n1\topen\t\n",
                      "rustic-tasks-v1\n1\topen\tNo final newline"):
        write_document(uart, malformed)
        results.append(observe(uart, data, "tasks list tasks-fixture", "error: invalid tasks document"))
    maximum = "rustic-tasks-v1\n" + "".join(f"{i}\topen\t{'x' * 24}\n" for i in range(1, 17))
    write_document(uart, maximum)
    results.append(observe(uart, data, "tasks list tasks-fixture", "16 tasks",
                           tuple(f"{i} [open] {'x' * 24}" for i in range(1, 17))))
    for oversized in (maximum + "17\tdone\tOverflow\n", "rustic-tasks-v1\n1\topen\t" + "x" * 25 + "\n"):
        write_document(uart, oversized)
        results.append(observe(uart, data, "tasks list tasks-fixture", "error: tasks capacity exceeded"))
    write_document(uart, document)
    uart.command("write /config/owner-policy invalid")
    uart.command("restart files", "utility sessions revoked")
    results.append(observe(uart, data, "tasks list tasks-fixture", "error: service denied"))
    uart.command(r'write /config/owner-policy "rustic-owner-v1\nhelpers=explicit\n"')
    uart.command("restart files", "utility sessions revoked")
    results.append(observe(uart, data, "tasks list tasks-fixture", "2 tasks", rows))
    uart.command("rm tasks-fixture")
    _, final = snapshot(data)
    assert final["files"] == initial["files"], "task fixtures changed unrelated files"
    assert counters(uart) == baseline
    return {"verified": True, "read_only": True, "cases": results}


def after_reboot(uart, data):
    write_document(uart, "rustic-tasks-v1\n4294967295\tdone\tAfter reboot\n")
    result = observe(uart, data, "tasks list tasks-fixture", "1 tasks",
                     ("4294967295 [done] After reboot",))
    uart.command("rm tasks-fixture")
    return result
