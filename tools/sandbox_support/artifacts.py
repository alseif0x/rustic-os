# SPDX-License-Identifier: Apache-2.0
"""Never extract container-controlled archive paths on the host."""
import hashlib
import io
import tarfile
from .process import command


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def unpack_single(data, expected_name, maximum):
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:") as archive:
        members = archive.getmembers()
        if len(members) != 1:
            raise RuntimeError("expected one artifact")
        member = members[0]
        if member.name not in (expected_name, "./" + expected_name) or not member.isfile():
            raise RuntimeError("invalid artifact path or type")
        if not 0 <= member.size <= maximum:
            raise RuntimeError(f"artifact size limit exceeded: {expected_name} "
                               f"({member.size} bytes; maximum {maximum})")
        return archive.extractfile(member).read()


def unpack_bundle(data, allowed, maximum):
    """Validate every member before the caller writes any fixed-name destination."""
    if len(data) > maximum:
        raise RuntimeError("artifact bundle size limit exceeded")
    result = {}
    total = 0
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:") as archive:
        for member in archive:
            if (member.name not in allowed or member.name in result
                    or not member.isfile() or member.issparse()):
                raise RuntimeError("invalid bundle artifact path or type")
            if not 0 <= member.size <= allowed[member.name]:
                raise RuntimeError("bundle artifact size limit exceeded")
            total += member.size
            if total > maximum:
                raise RuntimeError("artifact bundle payload limit exceeded")
            payload = archive.extractfile(member).read()
            if len(payload) != member.size:
                raise RuntimeError("truncated bundle artifact")
            result[member.name] = payload
    return result


def collect(container, remote, destination, maximum):
    transfer = destination.with_suffix(destination.suffix + ".transfer")
    try:
        parent, name = remote.rsplit("/", 1)
        code = command(["docker", "exec", container, "/bin/tar", "-C", parent, "-cf", "-", "--", name],
                       transfer, timeout=30, limit=maximum + 65536)
        if code:
            raise RuntimeError("artifact transfer failed: " + transfer.read_text(errors="replace")[:2000])
        destination.write_bytes(unpack_single(transfer.read_bytes(), remote.rsplit("/", 1)[-1], maximum))
    finally:
        transfer.unlink(missing_ok=True)
    return {"path": destination.name, "bytes": destination.stat().st_size, "sha256": sha256(destination)}
