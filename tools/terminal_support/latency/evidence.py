# SPDX-License-Identifier: Apache-2.0
"""Admit native latency evidence only when timing, driver and disk observations agree."""
import math
import re

CASES = {"delayed-completion": (.6, False), "expired-completion": (6.0, True)}
SUSPENDED = "blkdebug: Suspended request 'rustic_delay'"
RESUMED = "blkdebug: Resuming request 'rustic_delay'"
NUMBER = r"(0|[1-9][0-9]*)"


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def _line(text, pattern):
    matches = [match for line in text.splitlines() if (match := re.fullmatch(pattern, line))]
    require(len(matches) == 1, "missing or duplicate native evidence line")
    return matches[0].groups()


def stat(text):
    values = _line(text, f"id={NUMBER} parent={NUMBER} kind=file bytes={NUMBER} version={NUMBER}")
    return dict(zip(("id", "parent", "bytes", "version"), map(int, values)))


def _memory(text):
    keys = ("ticks", "free_frames", "process_slots", "processes", "channels", "pending_io")
    values = _line(text, " ".join(f"{key}={NUMBER}" for key in keys))
    return dict(zip(keys, map(int, values)))


def _commit(text):
    values = _line(text, f"committed id={NUMBER} previous={NUMBER} version={NUMBER} bytes={NUMBER}")
    return dict(zip(("id", "previous", "version", "bytes"), map(int, values)))


def _timeout(text):
    keys = ("request", "kind", "started", "now", "elapsed_ticks", "polls", "stalled_polls",
            "expected", "observed", "device_status")
    pattern = (f"RUSTIC BLOCK_FAILURE phase=completion request={NUMBER} kind={NUMBER} reason=Timeout "
               + " ".join(f"{key}={NUMBER}" for key in keys[2:])
               + " descriptor=None status=None")
    values = _line(text, pattern)
    result = dict(zip(keys, map(int, values)))
    require(all(value < 1 << 64 for value in result.values()), "driver diagnostic integer overflow")
    require(result["request"] > 0 and result["kind"] == 4 and result["device_status"] == 7,
            "timeout was not an owned outstanding FLUSH")
    require(result["now"] >= result["started"] and
            result["elapsed_ticks"] == result["now"] - result["started"] >= 500,
            "the real guest deadline did not expire")
    require(0 < result["polls"] and 0 <= result["stalled_polls"] < 5_000_000
            and result["stalled_polls"] <= result["polls"], "timeout used the stalled-clock fallback")
    require(result["expected"] == result["observed"] < 65536, "FLUSH already had a used entry")
    return result


def validate(case, evidence, state, backend, serial):
    hold, expired = CASES[case]
    require(backend.splitlines().count(SUSPENDED) == 1 and backend.splitlines().count(RESUMED) == 1,
            "missing or duplicate real backend suspension/resumption")
    require(backend.index(SUSPENDED) < backend.index(RESUMED), "backend events out of order")
    for field in ("suspended_seconds", "reply_seconds", "suspension_observed_seconds"):
        require(type(evidence[field]) in (float, int) and math.isfinite(evidence[field])
                and evidence[field] >= 0, "invalid host monotonic timing")
    require(evidence["suspended_seconds"] >= hold and evidence["reply_seconds"] >=
            evidence["suspension_observed_seconds"] + evidence["suspended_seconds"],
            "backend did not remain suspended for the declared interval")
    require(evidence["clean_exit"] is True, "guest did not shut down cleanly")
    # One mutation is sent exactly once. Recovery inspects its outcome and does
    # not replay it after a timeout or after the service restarts.
    require(sum(re.match(r"(?:rustic:/workspaces> )?replace ", line) is not None
                for line in serial.splitlines()) == 1,
            "tracked mutation was missing or replayed")
    require(sum(line.startswith("RUSTIC BLOCK_FAILURE") for line in serial.splitlines()) == int(expired),
            "unexpected or missing native block failure")
    before, after = stat(evidence["before"]), stat(evidence["after"])
    require(before["id"] > 0 and before["version"] > 0 and before["parent"] == 4
            and before["bytes"] == 6, "unexpected initial file identity/content size")
    require(after["id"] == before["id"] and after["parent"] == before["parent"], "file identity changed")
    memories = [_memory(evidence[name]) for name in
                ("resources_before", "resources_after_completion", "resources_final")]
    for memory in memories:
        require(memory["pending_io"] == 0 and memory["free_frames"] > 0
                and memory["processes"] == 3 and memory["channels"] == 4
                and memory["process_slots"] == 8, "pending request or unexpected native resource state")
        require(all(memory[key] == memories[0][key] for key in memory if key != "ticks"),
                "native resources were not reclaimed")
    require(memories[0]["ticks"] <= memories[1]["ticks"] <= memories[2]["ticks"], "guest clock reversed")
    reply = evidence["reply"]
    if expired:
        failure = _timeout(reply)
        require(_timeout(evidence["uart_before_resume"]) == failure,
                "timeout was not observed while the backend was still suspended")
        require("error: Uncertain" in reply.splitlines() and "committed id=" not in reply,
                "expired operation was reported as committed")
        require("files restarted; utility sessions revoked" in evidence["restart"].splitlines(),
                "explicit service recovery did not complete")
        require("error: OutcomeUnknown" in evidence["receipt"].splitlines(), "unexpected receipt after timeout")
        require(after == before and not state["records"], "timeout changed committed metadata or receipt")
        expected_bytes = b"before"
    else:
        committed = _commit(reply)
        require("error:" not in reply and "RUSTIC BLOCK_FAILURE" not in evidence["uart_before_resume"],
                "successful completion contained a failure")
        require(committed == _commit(evidence["receipt"]), "returned receipt differs from committed result")
        require(committed["id"] == before["id"] and committed["previous"] == before["version"]
                and committed["version"] == after["version"] > before["version"]
                and committed["bytes"] == after["bytes"] == 5, "committed identity/version mismatch")
        require(len(state["records"]) == 1, "missing or duplicate durable receipt")
        record = state["records"][0]
        require(record["id"] == before["id"] and record["previous"] == before["version"]
                and record["committed"] == after["version"] and record["content"] == b"after"
                and record["epoch"] == state["epoch"] and record["key"] == 991,
                "independent receipt content or identity mismatch")
        expected_bytes = b"after"
    require(expected_bytes.decode() in evidence["content"].splitlines(), "native content differs from outcome")
    require("untouched" in evidence["other"].splitlines(), "native unrelated file changed")
    require(state["files"][(4, "hello")] == expected_bytes and state["files"][(4, "other")] == b"untouched",
            "independent committed disk content mismatch")
    require(state["nodes"][before["id"]] == {"version": after["version"], "content": expected_bytes},
            "independent node differs from native stat")
