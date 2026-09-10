# SPDX-License-Identifier: Apache-2.0
"""Native read/reference conformance through the manual SDK client and C/H actors."""
import base64
import hashlib
import re
from contracts.read_vectors import load_cases, fixture_bytes, request_for
from .cases import counters, pid
from .authority_cases import actor, cleanup, fence


# This read-only adapter maps known request/service failures, not the whole native
# Error enum. Corrupt/Io stop without a successful observation. Transport failures
# (Protocol/Closed/Interrupted/Uncertain) and unmapped statuses remain local errors.
ERRORS = {"Invalid": ("invalid_request", "fix_request"), "Size": ("invalid_request", "fix_request"),
          "IsDirectory": ("invalid_request", "fix_request"),
          "Offset": ("invalid_request", "fix_request"), "Denied": ("access_denied", "stop"),
          "Revoked": ("access_denied", "stop"), "Expired": ("access_denied", "stop"),
          "Version": ("version_conflict", "refresh"), "NotFound": ("not_found", "refresh"),
          "UnsupportedVersion": ("unsupported_version", "refresh"), "Unavailable": ("unavailable", "stop"),
          "Io": ("io_error", "stop"), "Corrupt": ("io_error", "stop")}


def references(uart, workspace, resource):
    value = uart.command(f"ref {workspace} {resource}")
    match = re.search(r"(?m)^workspace=(ws_[0-9a-f]{32}_[0-9a-f]{8}) resource=(rs_[0-9a-f]{32}_[0-9a-f]{8}_[0-9a-f]{8})\r?$", value)
    assert match, value
    return {"workspace": match[1], "resource": match[2]}


def decode(value):
    """Translate one complete UART result; malformed/transport output is local failure."""
    if not isinstance(value, str) or len(value) > 8192:
        raise ValueError("native read output exceeds its text bound")
    value = value.replace("\r\n", "\n")
    if any(character != "\n" and not 32 <= ord(character) <= 126 for character in value):
        raise ValueError("native read output contains invalid control or non-ASCII bytes")
    lines = value.split("\n")
    headers = [(index, line) for index, line in enumerate(lines) if line.lstrip().startswith("read-v")]
    payloads = [(index, line) for index, line in enumerate(lines) if line.lstrip().startswith("data")]
    failures = [line for line in lines if line.lstrip().startswith("error")]
    envelope = {"version": 1, "method": "files.read"}
    if failures:
        if len(failures) != 1 or headers or payloads:
            raise ValueError("ambiguous native read failure/result framing")
        failure = re.fullmatch(r"error: ([A-Za-z][A-Za-z0-9_]*)", failures[0])
        if not failure or failure[1] not in ERRORS:
            raise ValueError("native read transport, protocol or unrecognized failure")
        code, action = ERRORS[failure[1]]
        return {**envelope, "error": {"code": code, "effect": "none", "next_action": action}}
    if len(headers) != 1 or len(payloads) != 1 or payloads[0][0] != headers[0][0] + 1:
        raise ValueError("incomplete or ambiguous native read result framing")
    header = re.fullmatch(r"read-v1 ([^\n]+)", headers[0][1])
    data = re.fullmatch(r"data=([0-9a-f]*)", payloads[0][1])
    if not header or not data or len(data[1]) > 2048 or len(data[1]) % 2:
        raise ValueError("malformed native read metadata or hex data")
    fields = {}
    for part in header[1].split(" "):
        pair = re.fullmatch(r"([a-z][a-z0-9_]*)=([^ =]+)", part)
        if not pair or pair[1] in fields:
            raise ValueError("malformed or duplicate native read metadata field")
        fields[pair[1]] = pair[2]
    if set(fields) != {"workspace", "resource", "version", "size", "offset", "length", "eof", "retry_epoch", "range_sha256"}:
        raise ValueError("native read metadata fields do not match version 1")
    for name in ("workspace", "resource", "version", "retry_epoch"):
        if not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", fields[name]):
            raise ValueError("invalid native read reference")
    for name in ("size", "offset", "length"):
        if not re.fullmatch(r"0|[1-9][0-9]{0,15}", fields[name]):
            raise ValueError("invalid native read integer")
        fields[name] = int(fields[name])
        if fields[name] > (1 << 53) - 1:
            raise ValueError("native read integer exceeds exact v1 domain")
    raw = bytes.fromhex(data[1])
    if fields.pop("length") != len(raw):
        raise ValueError("native read byte count mismatch")
    if fields["eof"] not in ("true", "false"):
        raise ValueError("invalid native read EOF flag")
    if not re.fullmatch(r"[0-9a-f]{64}", fields["range_sha256"]):
        raise ValueError("malformed native read SHA-256")
    if hashlib.sha256(raw).hexdigest() != fields["range_sha256"]:
        raise ValueError("native read SHA-256 mismatch")
    fields.update(eof=fields["eof"] == "true", data=base64.b64encode(raw).decode("ascii"))
    end = fields["offset"] + len(raw)
    if end > fields["size"] or fields["eof"] != (end == fields["size"]):
        raise ValueError("native read range or EOF contradicts file size")
    if not raw and not fields["eof"]:
        raise ValueError("native read made no progress before EOF")
    return {**envelope, "result": fields}


def read(uart, refs, offset=0, length=1024, version=None, expected_code=None):
    command = f"read-ref {refs['workspace']} {refs['resource']} {version or '-'} {offset} {length}"
    response = decode(uart.command(command, "error:" if expected_code else "read-v1 "))
    if expected_code:
        assert response["error"]["code"] == expected_code, response
    else:
        result = response["result"]
        assert all(result[k] == refs[k] for k in ("workspace", "resource"))
        assert result["offset"] == offset and len(base64.b64decode(result["data"])) <= length
        assert version in (None, result["version"])
    return response


def exercise(uart, data):
    baseline = counters(uart)
    uart.command("mkdir read-work")
    uart.command("touch read-work/binary")
    uart.command("touch read-work/empty")
    uart.command("write read-other untouched")
    c = pid(uart, "session read-work/binary read-other")
    h = pid(uart, f"helper {c} read-work/binary read-other")
    actor(uart, h, "fill", 17)
    actor(uart, c, "fill")
    refs = {"binary": references(uart, "read-work", "read-work/binary"),
            "empty": references(uart, "read-work", "read-work/empty"),
            "text": references(uart, "/workspaces", "hello")}
    exchanges = []
    observed = {}
    for case in load_cases():
        response = read(uart, refs[case["fixture"]], case["offset"], case["length"],
                        expected_code=case["expected_code"])
        if case["expected_code"] is None:
            result = response["result"]
            content = fixture_bytes(case["fixture"])
            assert base64.b64decode(result["data"]) == content[case["offset"]:case["offset"] + case["length"]]
            assert result["size"] == len(content)
            observed[case["fixture"]] = result
        exchanges.append({"id": case["id"], "request": request_for(case, **refs[case["fixture"]]), "response": response})
    for child in (c, h):
        result = actor(uart, child, "api-read")
        assert result["value"] == 1024 and result["other"] == 17 and result["control_denied"] == 1, result
        assert actor(uart, child, "read-open")["value"] == 40
    uart.command("write read-work/binary owner-edited")
    for child in (c, h):
        actor(uart, child, "read-next", 13)
    read(uart, refs["binary"], version=observed["binary"]["version"], expected_code="version_conflict")
    actor(uart, c, "fill")
    for child in (c, h):
        actor(uart, child, "read-open")
    fence(uart, h)
    for child in (c, h):
        actor(uart, child, "read-next", 18)
        actor(uart, child, "api-read", 18)
    cleanup(uart, c, h)
    before_restart = read(uart, refs["binary"])["result"]
    c = pid(uart, "session read-work/binary read-other")
    h = pid(uart, f"helper {c} read-work/binary read-other")
    for child in (c, h):
        assert actor(uart, child, "read-open")["value"] == 40
    uart.command("restart files", "utility sessions revoked")
    for child in (c, h):
        uart.command(f"act {child} read-next", "denied")
        uart.command(f"act {child} api-read", "denied")
    assert counters(uart) == baseline
    assert references(uart, "read-work", "read-work/binary") == refs["binary"]
    read(uart, refs["binary"], version=before_restart["version"])
    c = pid(uart, "session read-work/binary read-other")
    h = pid(uart, f"helper {c} read-work/binary read-other")
    for child in (c, h):
        assert actor(uart, child, "api-read")["value"] == 1024
    cleanup(uart, c, h)
    # Explicit workspace ancestry must not be replaced by the owner's broad grant.
    uart.command("ref read-work read-other", "error: Denied")
    for name in ("binary", "empty"):
        uart.command(f"rm read-work/{name}")
    uart.command("rm read-work")
    uart.command("mkdir read-work")
    uart.command("write read-work/binary replacement")
    replacement = references(uart, "read-work", "read-work/binary")
    assert replacement["workspace"] != refs["binary"]["workspace"]
    assert replacement["resource"] != refs["binary"]["resource"]
    read(uart, refs["binary"], expected_code="access_denied")
    assert base64.b64decode(read(uart, replacement)["result"]["data"]) == b"replacement"
    uart.command("rm read-work/binary")
    uart.command("rm read-work")
    uart.command("rm read-other")
    assert counters(uart) == baseline
    from .oracle import snapshot
    _, state = snapshot(data)
    assert state["files"] == {(3, "owner-policy"): b"rustic-owner-v1\nhelpers=explicit\n", (4, "hello"): fixture_bytes("text")}
    return {"verified": True, "backend": "native_sdk_uart", "exchanges": exchanges,
            "readonly_epoch_without_inspect": True, "chunk_conflict": True, "chunk_revocation": True,
            "restart_fresh_authority": True, "workspace_recreation": True, "resources_reclaimed": True,
            "persistent": {"references": refs["text"], "version": observed["text"]["version"]},
            "deleted": refs["binary"], "disk_sha256": state["selected_sha256"]}


def after_reboot(uart, evidence):
    retained = evidence["persistent"]
    assert references(uart, "/workspaces", "hello") == retained["references"]
    result = read(uart, retained["references"], version=retained["version"])["result"]
    assert base64.b64decode(result["data"]) == fixture_bytes("text")
    read(uart, evidence["deleted"], expected_code="access_denied")
    evidence["separate_boot_references"] = True
