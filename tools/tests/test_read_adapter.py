# SPDX-License-Identifier: Apache-2.0
"""UART adapter failures must never become successful logical read evidence."""
import base64
import hashlib
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_support.read_cases import decode


def output(raw=b"abc", *, offset=0, size=None, **changes):
    fields = {
        "workspace": "ws_" + "12" * 16 + "_00000004",
        "resource": "rs_" + "12" * 16 + "_00000004_00000005",
        "version": "v_0000000000000001",
        "size": offset + len(raw) if size is None else size,
        "offset": offset, "length": len(raw), "eof": "true",
        "retry_epoch": "e_0000000000000001", "range_sha256": hashlib.sha256(raw).hexdigest(),
    }
    fields.update(changes)
    return ("read-v1 " + " ".join(f"{key}={value}" for key, value in fields.items())
            + "\r\ndata=" + raw.hex() + "\r\n")


class ReadAdapter(unittest.TestCase):
    def reject(self, value):
        with self.assertRaises(ValueError):
            decode(value)

    def test_canonical_binary_result_ignores_command_echo_and_prompt(self):
        raw = b"\x00\xff\r\n"
        result = decode("read-ref workspace resource - 0 1024\r\n" + output(raw) + "rustic:/workspaces> ")
        self.assertEqual(result["method"], "files.read")
        self.assertEqual(result["version"], 1)
        self.assertEqual(base64.b64decode(result["result"]["data"]), raw)
        self.assertEqual(result["result"]["size"], 4)
        self.assertIs(result["result"]["eof"], True)
        self.assertNotIn("length", result["result"])

    def test_empty_file_and_exact_eof_return_empty_hash(self):
        for offset in (0, 1024):
            result = decode(output(b"", offset=offset))["result"]
            self.assertEqual(result["offset"], offset)
            self.assertEqual(result["data"], "")
            self.assertEqual(result["range_sha256"], hashlib.sha256(b"").hexdigest())
            self.assertTrue(result["eof"])

    def test_full_inline_range_and_short_non_eof_progress_are_valid(self):
        raw = bytes((i * 37 + 11) % 256 for i in range(1024))
        self.assertEqual(len(base64.b64decode(decode(output(raw))["result"]["data"])), 1024)
        result = decode(output(b"part", offset=10, size=100, eof="false"))["result"]
        self.assertEqual(result["offset"], 10)
        self.assertFalse(result["eof"])

    def test_known_read_failures_map_to_explicit_no_effect_advice(self):
        groups = {
            ("invalid_request", "fix_request"): ("Invalid", "Size", "Offset", "IsDirectory"),
            ("access_denied", "stop"): ("Denied", "Revoked", "Expired"),
            ("version_conflict", "refresh"): ("Version",),
            ("not_found", "refresh"): ("NotFound",),
            ("unsupported_version", "refresh"): ("UnsupportedVersion",),
            ("unavailable", "stop"): ("Unavailable",),
            ("io_error", "stop"): ("Io", "Corrupt"),
        }
        for (code, action), native_names in groups.items():
            for name in native_names:
                with self.subTest(native=name):
                    result = decode("error: " + name + "\r\nrustic:/> ")
                    self.assertEqual(result, {"version": 1, "method": "files.read", "error": {
                        "code": code, "effect": "none", "next_action": action,
                    }})

    def test_transport_and_unmapped_statuses_are_local_failures(self):
        for error in ("Protocol", "Closed", "Interrupted", "Uncertain", "Busy", "NoTransfer",
                      "ReadOnly", "Full", "NotDirectory", "Unexpected", "OutcomeUnknown"):
            with self.subTest(error=error):
                self.reject("error: " + error + "\r\n")

    def test_duplicate_metadata_is_rejected_even_when_values_match(self):
        header, data, _ = output().split("\r\n")
        for field in header.removeprefix("read-v1 ").split():
            with self.subTest(field=field):
                self.reject(header + " " + field + "\r\n" + data + "\r\n")

    def test_missing_unknown_or_malformed_metadata_fields_are_rejected(self):
        for value in (output().replace(" offset=0", ""), output().replace(" offset=0", " count=0"),
                      output().replace(" offset=0", " offset"), output().replace(" offset=0", " offset=0=0")):
            self.reject(value)

    def test_multiple_results_errors_payloads_or_mixed_outcomes_are_rejected(self):
        success = output()
        header, data, _ = success.split("\r\n")
        error = "error: Invalid\r\n"
        for value in (success + success, error + error, success + error, error + success,
                      header + "\r\n" + data + "\r\n" + data + "\r\n",
                      success + " read-v1 malformed\r\n", success + " data malformed\r\n",
                      success + " error Invalid\r\n", success + "read-v2 unknown\r\n"):
            self.reject(value)

    def test_missing_or_out_of_order_result_lines_are_rejected(self):
        header, data, _ = output().split("\r\n")
        for value in (header, data, "", data + "\r\n" + header,
                      header + "\r\nunexpected gap\r\n" + data):
            self.reject(value)

    def test_invalid_or_mismatched_sha256_is_rejected(self):
        for digest in ("0" * 64, "1" * 63, "1" * 65, "A" * 64, "g" * 64):
            with self.subTest(digest=digest):
                self.reject(output(range_sha256=digest))

    def test_malformed_oversized_or_miscounted_data_is_rejected(self):
        for encoded in ("6", "61 62 63", "AABBCC", "aabbcg", "ab" * 1025):
            self.reject(output().replace("data=616263", "data=" + encoded))
        self.reject(output(length=2))
        self.reject(output(length=4))
        self.reject(output(bytes(1025)))

    def test_native_integers_reject_signs_fractional_and_inexact_domain(self):
        for name in ("size", "offset", "length"):
            for value in ("-1", "+1", "01", "1.0", "1e0", "true", str(1 << 53), "1" * 17):
                with self.subTest(name=name, value=value):
                    changes = {name: value}
                    if name == "offset":
                        changes["size"] = 3
                    self.reject(output(**changes))

    def test_range_eof_and_progress_must_agree(self):
        for value in (output(size=2), output(offset=-1, size=2), output(eof="false"),
                      output(eof="True"), output(eof="1"), output(b"", size=1, eof="false"),
                      output(b"", offset=2, size=1), output(b"x", offset=(1 << 53) - 1)):
            self.reject(value)

    def test_references_are_bounded_ascii_tokens(self):
        for name in ("workspace", "resource", "version", "retry_epoch"):
            for token in ("", "x" * 65, "bad/path", "bad.name", "bad\x00name", "caf\u00e9"):
                with self.subTest(name=name, token=token):
                    self.reject(output(**{name: token}))

    def test_malformed_error_or_success_markers_are_rejected(self):
        for value in ("error: Invalid extra\r\n", " error: Invalid\r\n", "error:Invalid\r\n",
                      "error: Invalid \r\n", output().replace("read-v1 ", "read-v2 "),
                      output().replace("read-v1 ", " read-v1 "), output().replace("data=", " data=")):
            self.reject(value)

    def test_nontext_unbounded_or_control_character_output_is_rejected(self):
        for value in (None, b"error: Invalid\r\n", "x" * 8193, output() + "\x1b[0m",
                      output().replace("\r\n", "\r"), output().replace(" offset=", "\toffset=")):
            self.reject(value)


if __name__ == "__main__":
    unittest.main()
