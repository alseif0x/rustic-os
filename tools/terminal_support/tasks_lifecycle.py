# SPDX-License-Identifier: Apache-2.0
"""Native interleavings, checked against counters and an independent disk view."""
from .cases import counters
from .oracle import snapshot
from .tasks_cases import write_document

PREFIX = "RUSTIC TASKS_LIFECYCLE "
FLAGS = {
    "cancel": "held dormant adminpending aborted drained stale_denied released reset fresh",
    "concurrent": "held dormant adminpending reserved spin_started distinct_pids ownership perms released listed",
    "expiry": "completed abandoned expired stale_denied fresh",
}
FACTS = {
    "cancel": "job task_pid slot",
    "concurrent": "rows task_pid spin_pid job",
    "expiry": "rows start_tick complete_tick expiry_tick query_sent_tick",
}
COUNTERS = ("frames", "processes", "channels", "pending")


def parse(output):
    records = {}
    for line in output.splitlines():
        if not line.startswith(PREFIX):
            continue
        pairs = [field.split("=") for field in line[len(PREFIX):].split()]
        assert all(len(pair) == 2 for pair in pairs), line
        values = dict(pairs)
        assert len(values) == len(pairs), "duplicate lifecycle field"
        case = values.pop("case", None)
        assert case in FLAGS and case not in records, "unknown or duplicate lifecycle case"
        expected = set(FLAGS[case].split() + FACTS[case].split())
        expected.update(f"{when}_{name}" for when in ("before", "after") for name in COUNTERS)
        assert set(values) == expected, (case, set(values) ^ expected)
        assert all(value.isascii() and value.isdecimal() and str(int(value)) == value
                   for value in values.values()), "noncanonical lifecycle integer"
        record = {key: int(value) for key, value in values.items()}
        assert all(record[key] == 1 for key in FLAGS[case].split()), (case, record)
        for name in COUNTERS:
            assert record[f"before_{name}"] == record[f"after_{name}"], (case, name)
        assert record["before_processes"] == 3 and record["before_channels"] == 4, record
        assert record["before_pending"] == 0 and record["before_frames"] > 0, record
        if case == "cancel":
            assert record["job"] > 0 and record["task_pid"] > 0 and record["slot"] in (0, 1), record
        elif case == "concurrent":
            assert record["job"] > 0 and record["task_pid"] > 0 and record["spin_pid"] > 0, record
            assert record["task_pid"] != record["spin_pid"] and record["rows"] == 2, record
        else:
            assert 0 < record["start_tick"] <= record["complete_tick"] < record["expiry_tick"] < record["query_sent_tick"], record
            assert record["rows"] == 2, record
        records[case] = record
    assert set(records) == set(FLAGS), "missing lifecycle cases"
    return records


def exercise(uart, data):
    baseline = counters(uart)
    _, initial = snapshot(data)
    document = "rustic-tasks-v1\n7\topen\tReview kernel\n42\tdone\tBoot the OS\n"
    write_document(uart, document)
    before, state = snapshot(data)
    output = uart.command("tasks-acceptance tasks-fixture")
    records = parse(output)
    uart.command("status", "0")
    after, observed = snapshot(data)
    assert before == after, "task lifecycle fixture changed committed data"
    assert counters(uart) == baseline, "task lifecycle fixture leaked resources"
    names = {"frames": "free_frames", "processes": "processes", "channels": "channels", "pending": "pending_io"}
    for case, record in records.items():
        assert all(record[f"before_{name}"] == baseline[key] for name, key in names.items()), case
    uart.command("rm tasks-fixture")
    _, final = snapshot(data)
    assert final["files"] == initial["files"], "task lifecycle fixture changed unrelated files"
    return {"verified": True, "cases": records, "before_sha256": state["selected_sha256"],
            "after_sha256": observed["selected_sha256"]}
