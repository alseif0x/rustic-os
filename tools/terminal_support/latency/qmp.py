# SPDX-License-Identifier: Apache-2.0
"""Bounded, correlated QMP on an owned Unix socket; no general monitor interface."""
import json
import socket
import time


class Monitor:
    def __init__(self, path, process, log):
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.socket.settimeout(1)
        self.log, self.buffer, self.ticket = log, bytearray(), 0
        self.started, self.records = time.monotonic(), 0
        try:
            deadline = self.started + 5
            while True:
                try:
                    self.socket.connect(str(path))
                    break
                except (FileNotFoundError, ConnectionRefusedError):
                    if process.poll() is not None or time.monotonic() >= deadline:
                        raise RuntimeError("private QMP did not start")
                    time.sleep(.01)
            if "QMP" not in self._message(time.monotonic() + 3):
                raise RuntimeError("missing QMP greeting")
            self.call("qmp_capabilities")
        except BaseException:
            self.close()
            raise

    def close(self):
        self.socket.close()

    def _record(self, direction, value):
        self.records += 1
        if self.records > 256:
            raise RuntimeError("QMP evidence record budget exceeded")
        with self.log.open("a") as output:
            output.write(json.dumps({"seconds": time.monotonic() - self.started,
                                     "direction": direction, "message": value}) + "\n")

    def _message(self, deadline):
        while b"\n" not in self.buffer:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise RuntimeError("QMP response deadline exceeded")
            self.socket.settimeout(remaining)
            chunk = self.socket.recv(65536)
            if not chunk:
                raise RuntimeError("QMP closed")
            self.buffer.extend(chunk)
            if len(self.buffer) > 65536:
                raise RuntimeError("QMP response size budget exceeded")
        raw, _, remaining = self.buffer.partition(b"\n")
        self.buffer = bytearray(remaining)
        value = json.loads(raw)
        if not isinstance(value, dict):
            raise RuntimeError("invalid QMP response object")
        self._record("received", value)
        return value

    def call(self, name, arguments=None):
        self.ticket += 1
        request = {"execute": name, "id": self.ticket}
        if arguments is not None:
            request["arguments"] = arguments
        self._record("sent", request)
        self.socket.settimeout(3)
        self.socket.sendall(json.dumps(request).encode() + b"\r\n")
        deadline = time.monotonic() + 3
        for _ in range(64):
            reply = self._message(deadline)
            if "event" in reply:
                continue
            if (type(reply.get("id")) is not int or reply["id"] != self.ticket
                    or "error" in reply or "return" not in reply):
                raise RuntimeError(f"QMP command failed: {reply}")
            return reply["return"]
        raise RuntimeError("QMP event budget exceeded")

    def breakpoint(self, resume=False):
        command = "resume rustic_delay" if resume else "break flush_to_disk rustic_delay"
        result = self.call("human-monitor-command", {
            "command-line": f'qemu-io rusticdata "{command}"'})
        if result != "":
            raise RuntimeError(f"unexpected breakpoint response: {result}")
