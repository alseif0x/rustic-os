# SPDX-License-Identifier: Apache-2.0
"""Deterministic advisory signatures for offline failure triage."""

from __future__ import annotations

import re
from typing import Any, Iterable

from .schema import CLASSIFICATION_OPTIONS


# More specific causes are considered before generic assertion/timeout words.
_SIGNATURES: tuple[tuple[str, tuple[str, ...]], ...] = (
    (
        "compilation",
        (
            r"(?:error\s*\[E\d+\]|undefined reference|syntax error)",
            r"\b(?:compil(?:e|ation)|compiler|linker|build|rustc|cargo (?:build|check))\b.{0,80}\b(?:fail(?:ed|ure)?|error|cannot|abort|undefined)\b",
            r"\b(?:fail(?:ed|ure)?|error|cannot|abort|undefined)\b.{0,80}\b(?:compil|link|build)\w*\b",
        ),
    ),
    (
        "lint",
        (
            r"\b(?:clippy|lint|rustfmt|static analysis)\b.{0,80}\b(?:fail|error|warning|denied|diff|mismatch)\b",
            r"\b(?:fail|error|warning|denied|diff|mismatch)\b.{0,80}\b(?:clippy|lint|rustfmt|format)\b",
            r"\-D\s*warnings\b",
            r"\b(?:fmt|format(?:ting)?)\b.{0,80}\b(?:diff|mismatch|fail(?:ed|ure)?)\b",
        ),
    ),
    (
        "persistence",
        (
            r"\b(?:disk|storage|replay|durab(?:le|ility)|flush(?:ed|ing)?|fsync|persist(?:ed|ence)?|file (?:write|commit)|volume|readback)\b.{0,80}\b(?:fail(?:ed|ure)?|error|mismatch|lost|corrupt|uncertain|unknown|cannot|rollback)\b",
            r"\b(?:fail(?:ed|ure)?|error|mismatch|lost|corrupt|uncertain|unknown|cannot|rollback)\b.{0,80}\b(?:disk|storage|replay|durab|flush|fsync|persist|volume|readback)\b",
            r"\b(?:reboot|remount|crash recovery|durable receipt|torn (?:metadata|publication)|operation receipt)\b.{0,100}\b(?:fail(?:ed|ure)?|mismatch|differ|mixed|does not|not)\b",
            r"\b(?:volume oracle|disk write|published file bytes differ|bitmap claims)\b",
        ),
    ),
    (
        "environment",
        (
            r"\b(?:host tool|environment|configuration|config|network|dns|credential|dependency|toolchain)\b.{0,80}\b(?:fail(?:ed|ure)?|error|missing|invalid|unavailable|refused|unreachable|not found|denied)\b",
            r"\b(?:fail(?:ed|ure)?|error|missing|invalid|unavailable|refused|unreachable|not found|denied)\b.{0,80}\b(?:host tool|environment|configuration|config|network|dns|credential|dependency|toolchain)\b",
            r"\bcommand not found\b",
            r"\b(?:network|dns|tls|http)\s+(?:error|failure|refused|unreachable)\b",
            r"\b(?:container|docker|daemon)\b.{0,80}\b(?:unavailable|cannot|fail(?:ed|ure)?|error|refused)\b",
        ),
    ),
    (
        "resource_limit",
        (
            r"\b(?:quota|resource limit|out of (?:memory|space)|oom|memory exhausted|disk full|capacity exceeded|too many (?:files|processes))\b",
            r"\bresource temporarily unavailable\b",
            r"\bpids\.max\b",
        ),
    ),
    (
        "executor",
        (
            r"\b(?:executor|runner|worker|process|subprocess|qemu)\b.{0,80}\b(?:fail(?:ed|ure)?|error|crash|spawn|launch|start|exit)\b",
            r"\b(?:fail(?:ed|ure)?|error|crash|spawn|launch|start|exit)\b.{0,80}\b(?:executor|runner|worker|process|subprocess|qemu)\b",
            r"\bexecutor[_ -]?(?:error|cleanup|failure)\b",
            r"\b(?:cleanup|collector|bookkeeping)[_ -]?(?:failed|error|lost|missing)\b",
        ),
    ),
    (
        "assertion",
        (
            r"\bassert(?:ion)?(?:\s+[^\n]{0,80})?\s+(?:failed|error)\b",
            r"\bexpected\b.{0,80}\b(?:got|actual|received)\b",
            r"\b(?:got|actual|received)\b.{0,80}\bexpected\b",
            r"\btest mismatch\b",
            r"\bnot equal\b",
            r"\btest failed\b",
            r"\btest\b.{0,80}\b(?:failed|mismatch)\b",
        ),
    ),
    (
        "timeout",
        (
            r"\b(?:timeout|timed out|deadline(?: exceeded)?|hang(?:ed|ing)?|stalled)\b",
        ),
    ),
)


def _strings(value: Any, prefix: str = "facts") -> Iterable[tuple[str, str]]:
    if isinstance(value, str):
        yield prefix, value
    elif isinstance(value, dict):
        for key, child in value.items():
            yield from _strings(child, f"{prefix}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            yield from _strings(child, f"{prefix}[{index}]")


def _expected_negative_match(facts: dict[str, Any]) -> bool:
    expected = facts.get("expected_outcome")
    outcome = facts.get("outcome")
    if not isinstance(expected, str) or not isinstance(outcome, str):
        return False
    if expected.casefold() != outcome.casefold():
        return False
    return expected.casefold() in {
        "failure",
        "failed",
        "error",
        "panic",
        "hang",
        "hung",
        "timeout",
        "timed_out",
        "nonzero",
        "negative",
        "expected_failure",
    }


def missing_evidence(facts: dict[str, Any], excerpts: list[dict[str, Any]]) -> list[str]:
    """Return a stable coverage list without pretending logs are complete."""

    missing: list[str] = []
    if not any(excerpt.get("kind") == "log" for excerpt in excerpts):
        missing.append("no log excerpts supplied")
    for key in ("harness_passed", "outcome", "expected_outcome"):
        if key not in facts:
            missing.append(f"manifest.facts.{key} is missing")
    return missing


def _evidence_rows(excerpts: list[dict[str, Any]], matched_ids: set[str]) -> list[dict[str, Any]]:
    scored: list[tuple[float, int, dict[str, Any]]] = []
    for index, excerpt in enumerate(excerpts):
        score = 1.0 if excerpt["id"] in matched_ids else 0.0
        row = {
            "id": excerpt["id"],
            "kind": excerpt["kind"],
            "path": excerpt["path"],
            "start": excerpt["start"],
            "end": excerpt["end"],
            "sha256": excerpt["sha256"],
            "score": round(score, 6),
        }
        scored.append((score, index, row))
    scored.sort(key=lambda item: (-item[0], item[1]))
    for rank, (_, _, row) in enumerate(scored, start=1):
        row["rank"] = rank
    return [row for _, _, row in scored]


def classify(facts: dict[str, Any], excerpts: list[dict[str, Any]]) -> dict[str, Any]:
    """Return an advisory deterministic hypothesis and evidence ranking."""

    missing = missing_evidence(facts, excerpts)
    rows = _evidence_rows(excerpts, set())
    # A harness that reports pass is authoritative for this advisory layer,
    # including deliberate panic/hang fixtures.  It does not rewrite facts.
    if facts.get("harness_passed") is True:
        return {
            "hypothesis": {"category": "unknown", "confidence": 0.0, "accepted": False},
            "signals": [],
            "evidence": rows,
            "missing_evidence": missing,
            "guard": "harness_passed",
            "method": "deterministic_signature",
        }
    # A matched expected negative is an intentional fixture outcome.  Keep the
    # metadata visible and abstain without inferring a harness pass.
    if _expected_negative_match(facts) and facts.get("harness_passed") is not False:
        return {
            "hypothesis": {"category": "unknown", "confidence": 0.0, "accepted": False},
            "signals": [],
            "evidence": rows,
            "missing_evidence": missing,
            "guard": "expected_negative_outcome",
            "method": "deterministic_signature",
        }
    if str(facts.get("run_association", "")).casefold() in {"unverified", "unknown", "unavailable"}:
        return {
            "hypothesis": {"category": "unknown", "confidence": None, "accepted": False},
            "signals": [],
            "evidence": rows,
            "missing_evidence": missing,
            "guard": "run_association_unverified",
            "method": "deterministic_signature",
        }
    if any(
        re.search(r"\bignore\b.{0,80}\bevidence\b|\bno actual failure\b", str(excerpt["text"]), flags=re.IGNORECASE)
        for excerpt in excerpts
        if excerpt.get("kind") == "log"
    ):
        return {
            "hypothesis": {"category": "unknown", "confidence": None, "accepted": False},
            "signals": [],
            "evidence": rows,
            "missing_evidence": missing,
            "guard": "untrusted_instruction",
            "method": "deterministic_signature",
        }

    # Only explicitly named observed status fields and log excerpts drive the
    # baseline.  Selected diffs are context for JEV and are never signatures.
    known_fact_keys = (
        "failure",
        "failure_kind",
        "failure_category",
        "error",
        "error_type",
        "error_message",
        "message",
        "reason",
        "status",
        "result",
        "outcome",
        "timed_out",
    )
    sources: list[tuple[str, str]] = []
    for key in known_fact_keys:
        value = facts.get(key)
        if isinstance(value, str):
            sources.append((f"facts.{key}", value))
        elif key == "timed_out" and value is True:
            sources.append(("facts.timed_out", "deadline timeout"))
    for excerpt in excerpts:
        if excerpt.get("kind") == "log":
            sources.append((f"excerpt:{excerpt['id']}", str(excerpt["text"])))
    matched: list[dict[str, str]] = []
    matched_ids: set[str] = set()
    category = "unknown"
    explicit = facts.get("failure_category") or facts.get("failure_kind") or facts.get("status")
    explicit_category = explicit if isinstance(explicit, str) and explicit in CLASSIFICATION_OPTIONS and explicit != "unknown" else None
    if explicit_category is not None:
        category = explicit_category
        matched.append({"category": explicit, "source": "facts.explicit_category", "signature": "observed status"})
    candidates = ((candidate, patterns) for candidate, patterns in _SIGNATURES if explicit_category is None or candidate == category)
    if explicit_category is None:
        for candidate, patterns in candidates:
            found = False
            for source, value in sources:
                for pattern in patterns:
                    if re.search(pattern, value, flags=re.IGNORECASE):
                        matched.append({"category": candidate, "source": source, "signature": pattern})
                        if source.startswith("excerpt:"):
                            matched_ids.add(source.split(":", 1)[1])
                        found = True
                        break
            if found:
                category = candidate
                break
    else:
        # An explicit observed status wins over conflicting generic log text,
        # while every matching excerpt for that chosen status is retained.
        for source, value in sources:
            for pattern in next(patterns for candidate, patterns in _SIGNATURES if candidate == category):
                if re.search(pattern, value, flags=re.IGNORECASE):
                    matched.append({"category": category, "source": source, "signature": pattern})
                    if source.startswith("excerpt:"):
                        matched_ids.add(source.split(":", 1)[1])
                    break
    confidence = None
    rows = _evidence_rows(excerpts, matched_ids)
    return {
        "hypothesis": {
            "category": category if category in CLASSIFICATION_OPTIONS else "unknown",
            "confidence": confidence,
            # Baseline acceptance records a deterministic rule hit.  Its
            # confidence remains null because no calibrated probability is
            # claimed; live model acceptance separately requires >=0.8.
            "accepted": category != "unknown",
        },
        "signals": matched,
        "evidence": rows,
        "missing_evidence": missing,
        "guard": None,
        "method": "deterministic_signature",
    }


__all__ = ["classify", "missing_evidence"]
