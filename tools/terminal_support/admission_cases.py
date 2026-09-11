# SPDX-License-Identifier: Apache-2.0
"""Public SDK/IPC admission mission on disposable disks; no screenshots or model."""
import re
from .cases import pid, exited
from .operation_cases import references, operation, verify_content
from .recovery_cases import stat
from .oracle import snapshot


def status(text):
    lines = text.replace("\r\n", "\n").splitlines()
    headers = [line for line in lines if line.startswith("admission-v1 ")]
    if len(headers) != 1 or any(line.startswith("error:") for line in lines):
        raise AssertionError("missing or ambiguous admission status")
    match = re.fullmatch(r"admission-v1 id=(ad_([0-9a-f]{32})_([0-9a-f]{16})) service_instance=(si_[0-9a-f]{32}_[0-9a-f]{16}) state=(admitted|cancelled|committed) terminal=(0|[1-9][0-9]*)", headers[0])
    if not match:
        raise AssertionError("invalid admission status")
    result = dict(id=match[1], lineage=match[2], number=int(match[3], 16), instance=match[4], state=match[5], terminal=int(match[6]))
    completions = [line for line in lines if line.startswith("completion=")]
    expected = [f"completion=op_{result['lineage']}_{result['terminal']:016x}"] if result["state"] == "committed" else []
    if completions != expected:
        raise AssertionError("admission/completion namespace mismatch")
    return result


def check(data, result, content, node):
    _, state = snapshot(data)
    records = [r for r in state["records"] if r.get("admission") == result["number"]]
    if len(records) != 1:
        raise AssertionError("independent disk lacks unique admission")
    record = records[0]
    if (record["state"], record["terminal"], record["content"], record["id"]) != (result["state"], result["terminal"], content, node):
        raise AssertionError("wire status differs from independently decoded disk")
    if result["instance"] != f"si_{state['lineage']}_{record['instance']:016x}":
        raise AssertionError("admission lost originating incarnation")
    return state


def verify(session, owned_disk, temporary, image, mount):
    cases = []
    with owned_disk(temporary / "public-admissions.raw", True, evidence_name="public-admissions") as data:
        with session(image, data, "public-admissions-initial") as uart:
            uart.command("write hello before"); uart.command("write other untouched")
            node = stat(uart, "hello")
            ws, rs = references(uart, ".", "hello")
            uart.command("enable-operations", "persistent format v3")
            uart.command("enable-admissions", "persistent format v4")
            empty = snapshot(data)[0]
            child = pid(uart, "run lost-admission hello other")
            exited(uart, child, 1, 0); uart.command(f"reap {child}", "code=0")
            query = f"admission {ws} e_0000000000000001 k_000000000000004d"
            accepted = status(uart.command(query))
            if accepted["state"] != "admitted" or stat(uart, "hello") != node:
                raise AssertionError("lost acceptance reply executed file effect")
            pending = snapshot(data)[0]
            admission = f'admit-ref {ws} {rs} v_{node["version"]:016x} e_0000000000000001 k_000000000000004d "reply deliberately unobserved"'
            if status(uart.command(admission)) != accepted or snapshot(data)[0] != pending:
                raise AssertionError("retry changed retained admission")
            uart.command("restart files", "utility sessions revoked")
            if status(uart.command(query)) != accepted or snapshot(data)[0] != pending:
                raise AssertionError("restart resumed pending admission")
        with session(mount, data, "public-admissions-reboot") as uart:
            if status(uart.command(query)) != accepted or snapshot(data)[0] != pending:
                raise AssertionError("reboot changed pending admission")
            committed = status(uart.command(f"execute-admission {accepted['id']}"))
            if committed["state"] != "committed" or committed["id"] != accepted["id"]:
                raise AssertionError("explicit execution failed")
            check(data, committed, b"reply deliberately unobserved", node["id"])
            receipt = operation(uart.command(f"operation op_{committed['lineage']}_{committed['terminal']:016x}"))
            verify_content(uart, data, receipt, b"reply deliberately unobserved")
            settled = snapshot(data)[0]
            for action in ("execute-admission", "cancel-admission"):
                if status(uart.command(f"{action} {committed['id']}")) != committed or snapshot(data)[0] != settled:
                    raise AssertionError("terminal replay changed committed effect")
            uart.command("rotate-receipts", "epoch=2")
            current = stat(uart, "hello")
            planned = status(uart.command(f'admit-ref {ws} {rs} v_{current["version"]:016x} e_0000000000000002 k_000000000000004e "never"'))
            uart.command("write hello concurrent")
            uart.command(f"execute-admission {planned['id']}", "Version")
            uart.command("rotate-receipts", "Busy")
            cancelled = status(uart.command(f"cancel-admission {planned['id']}"))
            if cancelled["state"] != "cancelled": raise AssertionError("cancel failed")
            check(data, cancelled, b"never", node["id"])
            settled = snapshot(data)[0]
            for action in ("execute-admission", "cancel-admission"):
                if status(uart.command(f"{action} {cancelled['id']}")) != cancelled or snapshot(data)[0] != settled:
                    raise AssertionError("terminal replay changed cancelled effect")
            uart.command("cat hello", "concurrent"); uart.command("cat other", "untouched")
        cases.append({"case":"public_admission_mission", "verified":True, "lost_reply":True, "reboot_pending":True, "fresh_execution":True, "cancelled":True, "sha256":snapshot(data)[1]["selected_sha256"]})

    for name, base, command, cut in (
        ("accept_io", empty, admission, 0),
        ("cancel_io", pending, f"cancel-admission {accepted['id']}", 0),
        ("execute_flush", pending, f"execute-admission {accepted['id']}", 16),
    ):
        with owned_disk(temporary / (name + ".raw"), True, evidence_name=name) as data:
            with data.open("r+b") as stream: stream.write(base)
            with session(mount, data, "public-" + name + "-fault", cut) as uart:
                uart.command(command, "Uncertain")
                uart.command("restart files", "utility sessions revoked")
                if name == "accept_io":
                    uart.command(query, "OutcomeUnknown"); observed = None
                else:
                    observed = status(uart.command(query))
                    check(data, observed, b"reply deliberately unobserved", node["id"])
                    if observed["state"] != ("admitted" if name == "cancel_io" else "committed"):
                        raise AssertionError("native EIO recovery contradicted the selected publication boundary")
            settled = snapshot(data)[0]
            with session(mount, data, "public-" + name + "-reboot") as uart:
                if observed is None: uart.command(query, "OutcomeUnknown")
                elif status(uart.command(query)) != observed: raise AssertionError("reboot changed fault outcome")
                uart.command("cat hello", "reply deliberately unobserved" if observed and observed["state"] == "committed" else "before")
                uart.command("cat other", "untouched")
                if snapshot(data)[0] != settled: raise AssertionError("fault recovery replayed work")
            cases.append({"case":"public_admission_" + name, "verified":True, "state":observed["state"] if observed else "unknown", "reboot_verified":True, "sha256":snapshot(data)[1]["selected_sha256"]})
    return cases
