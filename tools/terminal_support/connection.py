# SPDX-License-Identifier: Apache-2.0
"""Real UART socket input; no guest command injection through kernel fixtures."""
import json
import socket
import time
class Connection:
    def __init__(self, path, process, transcript, timeout=60, timings=None):
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.process, self.transcript, self.timeout = process, transcript, timeout
        self.commands = 0
        self.attempts = 0
        self.timings = timings
        self.started = time.monotonic()
        if timings is not None:
            timings.write_text("")
        self.data = bytearray()
        self.pending = bytearray()
        deadline = time.monotonic() + timeout
        while True:
            try:
                self.socket.connect(str(path))
                break
            except (FileNotFoundError, ConnectionRefusedError):
                if process.poll() is not None or time.monotonic() > deadline:
                    self.close()
                    raise RuntimeError("UART connection failed")
                time.sleep(.02)
    def until(self, marker=b"> "):
        deadline = time.monotonic() + self.timeout
        while marker not in self.pending:
            if time.monotonic() > deadline:
                self.save()
                raise RuntimeError(f"UART timeout waiting for {marker!r}; tail={bytes(self.data[-4000:])!r}")
            self.socket.settimeout(.25)
            try:
                chunk = self.socket.recv(4096)
            except socket.timeout:
                continue
            if not chunk:
                self.save()
                raise RuntimeError(f"UART closed; tail={bytes(self.data[-4000:])!r}")
            self.data.extend(chunk)
            self.pending.extend(chunk)
            if len(self.data) > 1024 * 1024:
                raise RuntimeError("UART evidence exceeds budget")
        end = self.pending.index(marker) + len(marker)
        result = bytes(self.pending[:end])
        del self.pending[:end]
        self.save()
        return result.decode("ascii", "backslashreplace")
    def send(self, data):
        self.socket.sendall(data)
    def command(self, command, expected=None):
        if self.attempts >= 4096:
            raise RuntimeError("UART command evidence exceeds budget")
        self.attempts += 1
        started = time.monotonic()
        words = command.split(maxsplit=1)
        record = {"attempt": self.attempts, "verb": words[0][:64] if words else "",
                  "sent_seconds": started - self.started, "response_received": False,
                  "accepted": False}
        try:
            self.send(command.encode("ascii") + b"\r")
            output = self.until()
            record["response_received"] = True
            if expected is not None and expected not in output:
                raise AssertionError(f"{command!r}: missing {expected!r} in {output!r}")
            if expected is None and "\r\nerror:" in output:
                raise AssertionError(f"{command!r}: unexpected error in {output!r}")
            self.commands += 1
            record["accepted"] = True
            return output
        except Exception as error:
            record["error_type"] = type(error).__name__
            raise
        finally:
            record["elapsed_seconds"] = time.monotonic() - started
            if self.timings is not None:
                try:
                    with self.timings.open("a") as stream:
                        stream.write(json.dumps(record) + "\n")
                except OSError:
                    # Do not replace the command's actual failure with a log error.
                    if record["accepted"]:
                        raise
    def save(self):
        self.transcript.write_bytes(self.data)
    def close(self):
        self.save()
        self.socket.close()
