# SPDX-License-Identifier: Apache-2.0
"""Native completed-operation mission, same-key namespaces and actual device failure cuts."""
import base64
import hashlib
import json
import re
from pathlib import Path
from .cases import pid, exited, counters
from .authority_cases import actor
from .oracle import snapshot
from .recovery_cases import stat
from .recovery_faults import CUTS
from .read_cases import read

VECTORS = json.loads((Path(__file__).resolve().parents[2] / "contracts/services/v1/fixtures/replace-cases.json").read_text())["cases"]

def references(uart, directory, file):
    text = uart.command(f"ref {directory} {file}")
    matches = re.findall(r"^workspace=(ws_[0-9a-f]{32}_[0-9a-f]{8}) resource=(rs_[0-9a-f]{32}_[0-9a-f]{8}_[0-9a-f]{8})$", "\n".join(text.splitlines()), re.MULTILINE)
    if len(matches) != 1: raise AssertionError("missing native operation references")
    return matches[0]

def fields(line):
    result = {}
    for part in line.split(" ")[1:]:
        match = re.fullmatch(r"([a-z][a-z0-9_]*)=([^ =]+)", part)
        if not match or match[1] in result:
            raise AssertionError("malformed or duplicate operation field")
        result[match[1]] = match[2]
    return result


def operation(text):
    text = text.replace("\r\n", "\n")
    if len(text) > 8192 or any(c != "\n" and not 32 <= ord(c) <= 126 for c in text):
        raise AssertionError("invalid operation text framing")
    lines = text.splitlines()
    headers = [line for line in lines if line.startswith("operation-v1 ")]
    receipts = [line for line in lines if line.startswith("receipt workspace=")]
    if len(headers) != 1 or len(receipts) != 1: raise AssertionError("missing or ambiguous operation receipt")
    if lines.index(receipts[0]) != lines.index(headers[0]) + 1 or any(line.lstrip().startswith("error") for line in lines):
        raise AssertionError("ambiguous operation success/error framing")
    h = fields(headers[0])
    r = fields(receipts[0])
    if set(h) != {"id", "service_instance", "state", "effect", "cancel_requested"} or h["state"] != "succeeded" or h["effect"] != "committed" or h["cancel_requested"] != "false":
        raise AssertionError("operation is not a confirmed completion")
    if set(r) != {"workspace", "resource", "previous_version", "version", "size", "epoch", "key", "sha256"}: raise AssertionError("invalid receipt fields")
    if not re.fullmatch(r"0|[1-9][0-9]{0,3}", r["size"]) or int(r["size"]) > 1024:
        raise AssertionError("invalid operation receipt size")
    if not re.fullmatch(r"[0-9a-f]{64}", r["sha256"]): raise AssertionError("invalid receipt hash")
    return {"operation_id":h["id"], "service_instance":h["service_instance"], "state":"succeeded", "effect":"committed", "cancel_requested":False,
            "receipt":{k:r[k] for k in ("workspace", "resource", "previous_version", "version", "sha256")} | {"size":int(r["size"]), "retry":{"epoch":r["epoch"], "key":r["key"]}}}

def lookup(uart, workspace, epoch, key=77):
    return operation(uart.command(f"operation {workspace} e_{epoch:016x} k_{key:016x}"))

def replace(uart, ws, rs, version, epoch, text, key=77):
    return operation(uart.command(f'replace-ref {ws} {rs} v_{version:016x} e_{epoch:016x} k_{key:016x} "{text}"'))

def check(result, ws, rs, previous, epoch, key, content, state=None):
    receipt = result["receipt"]
    if receipt["workspace"] != ws or receipt["resource"] != rs or receipt["previous_version"] != f"v_{previous:016x}" or receipt["retry"] != {"epoch":f"e_{epoch:016x}","key":f"k_{key:016x}"}:
        raise AssertionError("operation returned a foreign identity or retry tuple")
    version = int(receipt["version"][2:], 16)
    lineage = ws[3:35]
    if version <= previous or result["operation_id"] != f"op_{lineage}_{version:016x}" or not re.fullmatch(f"si_{lineage}_[0-9a-f]{{16}}", result["service_instance"]):
        raise AssertionError("invalid persistent operation/service identity")
    if receipt["size"] != len(content) or receipt["sha256"] != hashlib.sha256(content).hexdigest():
        raise AssertionError("receipt content differs from requested committed bytes")
    if state is not None:
        matching = [r for r in state["records"] if r.get("workspace") == int(ws[36:], 16) and r["key"] == key and r["epoch"] == epoch]
        if len(matching) != 1: raise AssertionError("independent disk lacks one unique operation")
        record = matching[0]
        if record["id"] != int(rs[-8:],16) or record["previous"] != previous or record["committed"] != version or record["content"] != content or result["service_instance"] != f"si_{lineage}_{record['instance']:016x}":
            raise AssertionError("independent operation receipt differs from native result")

def verify_content(uart, data, result, content):
    receipt = result["receipt"]
    refs = {key: receipt[key] for key in ("workspace", "resource")}
    observed = read(uart, refs, version=receipt["version"])["result"]
    if base64.b64decode(observed["data"]) != content or observed["range_sha256"] != receipt["sha256"]:
        raise AssertionError("version-pinned native read differs from committed operation")
    node = snapshot(data)[1]["nodes"].get(int(receipt["resource"][-8:], 16))
    if node != {"version": int(receipt["version"][2:], 16), "content": content}:
        raise AssertionError("independent current file differs from committed operation")


def verify(session, owned_disk, temporary, image, mount):
    cases, exchanges = [], []
    with owned_disk(temporary / "operations.raw", True, evidence_name="operations-reboot") as data:
        with session(image, data, "operations-initial") as uart:
            uart.command("mkdir alpha"); uart.command("mkdir beta")
            uart.command("write alpha/item before"); uart.command("write beta/item before")
            uart.command("write other untouched")
            a = stat(uart, "alpha/item"); b = stat(uart, "beta/item")
            wa, ra = references(uart, "alpha", "alpha/item"); wb, rb = references(uart, "beta", "beta/item")
            uart.command(f"operation {wa} e_0000000000000001 k_000000000000004d", "Unsupported")
            uart.command("enable-operations", "persistent format v3")
            baseline = counters(uart)
            base = snapshot(data)[0]
            child = pid(uart, "run lost-operation alpha/item other")
            exited(uart, child, 1, 0); uart.command(f"permissions {child}", "other=17"); uart.command(f"reap {child}", "code=0")
            original = lookup(uart, wa, 1)
            check(original, wa, ra, a["version"], 1, 77, b"reply deliberately unobserved")
            verify_content(uart, data, original, b"reply deliberately unobserved")
            observer = pid(uart, "session alpha/item other")
            actor(uart, observer, "operation-get", 17)
            uart.command(f"kill {observer}"); exited(uart, observer, 3, 0); uart.command(f"reap {observer}")
            uart.command("write alpha/item later-edit")
            before = stat(uart, "alpha/item")
            if replace(uart, wa, ra, a["version"], 1, "reply deliberately unobserved") != original or stat(uart, "alpha/item") != before:
                raise AssertionError("identical retry duplicated the effect")
            uart.command(f'replace-ref {wa} {ra} v_{a["version"]:016x} e_0000000000000001 k_000000000000004d "changed"', "IdempotencyConflict")
            uart.command(f'replace-ref {wa} {rb} v_{b["version"]:016x} e_0000000000000001 k_000000000000004d "foreign"', "Denied")
            second = replace(uart, wb, rb, b["version"], 1, "other workspace")
            if second["operation_id"] == original["operation_id"] or second["service_instance"] != original["service_instance"]:
                raise AssertionError("workspace key collision or unstable service incarnation")
            check(original, wa, ra, a["version"], 1, 77, b"reply deliberately unobserved", snapshot(data)[1])
            check(second, wb, rb, b["version"], 1, 77, b"other workspace", snapshot(data)[1])
            uart.command(f'replace-ref {wa} {ra} v_{before["version"]:016x} e_0000000000000001 k_000000000000004e "full"', "Full")
            uart.command("rm alpha/item")
            if operation(uart.command(f'operation {original["operation_id"]}')) != original: raise AssertionError("deletion erased retained result")
            uart.command("write alpha/item recreated")
            _, fresh = references(uart, "alpha", "alpha/item")
            if fresh == ra: raise AssertionError("recreation reused resource identity")
            uart.command("restart files", "utility sessions revoked")
            if lookup(uart, wa, 1) != original or lookup(uart, wb, 1) != second: raise AssertionError("restart changed retained results")
            if counters(uart) != baseline: raise AssertionError("operation mission leaked resources")
        with session(mount, data, "operations-reboot") as uart:
            if lookup(uart, wa, 1) != original or operation(uart.command(f'operation {second["operation_id"]}')) != second:
                raise AssertionError("VM reboot changed retained results")
            epoch = 1
            for vector in VECTORS:
                epoch += 1; uart.command("rotate-receipts", f"epoch={epoch}")
                uart.command(f"operation {wa} e_0000000000000001 k_000000000000004d", "ExpiredEpoch")
                before = stat(uart, "beta/item")
                command = f'replace-fill-ref {wb} {rb} v_{before["version"]:016x} e_{epoch:016x} k_0000000000000063 {vector["byte"]} {vector["length"]}'
                result = operation(uart.command(command))
                content = bytes([vector["byte"]])*vector["length"]
                check(result, wb, rb, before["version"], epoch, 99, content, snapshot(data)[1])
                verify_content(uart, data, result, content)
                by_retry = lookup(uart, wb, epoch, 99); by_id = operation(uart.command(f'operation {result["operation_id"]}'))
                if by_retry != result or by_id != result: raise AssertionError("lookup differs from confirmed replacement")
                if result["service_instance"] == original["service_instance"]: raise AssertionError("reboot reused the old service incarnation")
                exchanges.append({"case":vector["id"],"workspace":wb,"resource":rb,"previous_version":f'v_{before["version"]:016x}',"epoch":f'e_{epoch:016x}',"key":"k_0000000000000063","result":result,"by_retry":by_retry,"by_id":by_id})
            uart.command("cat other", "untouched")
        selected, state = snapshot(data)
        cases.append({"case":"scoped_operations_lost_reply_namespaces_restart_reboot", "verified":True,"sha256":state["selected_sha256"],"exchanges":exchanges})
    for name, cut in CUTS.items():
        with owned_disk(temporary / ("operations-"+name+".raw"), True, evidence_name="operations-"+name+"-reboot") as data:
            with data.open("r+b") as stream: stream.write(base)
            with session(mount, data, "operations-"+name+"-fault", cut) as uart:
                uart.command(f'replace-ref {wa} {ra} v_{a["version"]:016x} e_0000000000000001 k_000000000000004d "after"', "Uncertain")
                uart.command("restart files", "utility sessions revoked")
                _, state = snapshot(data); committed = bool(state["records"])
                uart.command("cat alpha/item", "after" if committed else "before")
                if committed:
                    result = lookup(uart, wa, 1); check(result, wa, ra, a["version"], 1, 77, b"after", state)
                else: uart.command(f"operation {wa} e_0000000000000001 k_000000000000004d", "OutcomeUnknown")
            with session(mount, data, "operations-"+name+"-reboot") as uart:
                if committed:
                    if lookup(uart, wa, 1) != result: raise AssertionError("fault recovery changed operation identity")
                else: uart.command(f"operation {wa} e_0000000000000001 k_000000000000004d", "OutcomeUnknown")
                uart.command("cat alpha/item", "after" if committed else "before")
                uart.command("cat beta/item", "before"); uart.command("cat other", "untouched")
            selected, state = snapshot(data)
            if committed: check(result, wa, ra, a["version"], 1, 77, b"after", state)
            elif state["records"]: raise AssertionError("unexpected operation after unsuccessful publication")
            cases.append({"case":"scoped_operation_"+name,"verified":True,"committed":committed,"sha256":state["selected_sha256"]})
    from .control_cases import verify as verify_control
    cases.extend(verify_control(session, owned_disk, temporary, mount, base, wa, ra, a["version"]))
    return cases, selected
