<!-- SPDX-License-Identifier: Apache-2.0 -->

# Bounded native file service

Implemented as the bounded storage foundation for the [native terminal](TERMINAL.md), with separate pure format/policy crates, SDK clients and a ring-3 file-server application. #12 remains open for the logical service-v1 durable effect/receipt contract; this bootstrap protocol does not claim that conformance.

## Format decision

Use a small original copy-on-write volume for the first native terminal: 32 objects including four namespace roots, 1 KiB per file, 31-byte component names, fixed extents and two checksummed metadata banks. These are explicit format limits, not claims to use the full 4 GiB disk or a permanent general-purpose format. Revisit reuse before increasing file sizes, concurrency or hardware support.

[Littlefs](https://github.com/littlefs-project/littlefs/blob/master/DESIGN.md) provides bounded metadata-pair and copy-on-write precedents, but its flash-oriented C implementation would add a separate port/FFI boundary here. [rust-fatfs](https://github.com/rafalh/rust-fatfs) is a reuse candidate for FAT compatibility; its I/O/runtime integration and in-place format require additional recovery decisions. [SQLite's atomic-commit explanation](https://www.sqlite.org/atomiccommit.html) motivates explicit persistence ordering and failure testing. These sources inform the design; no source code is copied and no format compatibility is claimed.

The selected tiny volume makes a bounded interruption test possible with the already implemented copied-sector SDK. New file data goes into an inactive extent, followed by flush; new metadata goes into an inactive bank, followed by flush; its checksummed sequence header is published last, followed by flush. A failed submitted write poisons the live mount until recovery and reports an unknown effect. Mount chooses a valid committed bank; malformed data is rejected. Checksums detect corruption, not adversarial tampering or arbitrary hash collisions.

The file server runs as a separate native process. A pure no_std format crate has no kernel/SDK dependency; a native adapter translates its storage trait into block calls. File-client messages remain within the existing 64-byte IPC payload with bounded staging, sequential offsets and commit-time context/version checks. The supervisor's authenticated control endpoint issues/revokes scopes; paths and manifest identities grant no authority.

System base, data, configuration and workspaces have distinct root objects. File identities are monotonic within a volume; authority checks use object ancestry and operations. Symlinks, hard links, renames, sparse files, permissions inferred from textual prefixes, dynamic allocation and host filesystem access are outside this first format.

Fresh initialization requires the trusted launcher to identify a newly created disposable/managed volume; an unknown or damaged existing disk must not be silently formatted. The host launcher never selects a physical disk.

## Native protocol and ownership

The file packet is exactly 64 bytes: version/opcode/status/payload count at offsets 0–3, object ID at 4, argument/offset at 8, context generation at 12, resource version at 16 and up to 40 payload bytes at 24. Integers are little endian. Unused bytes are zero; malformed shapes fail before mutation. The ABI crate owns constants and codecs. Paths are resolved by the SDK through authorized object lookups; the server checks ancestry against an authenticated peer and grant.

LOOKUP/STAT/LIST discover authorized objects. CREATE/MKDIR/REMOVE mutate namespace metadata. READ pins the observed file version across chunks. BEGIN reserves one of two bounded transfers; CHUNK requires the next exact offset; COMMIT checks context, scope, completeness and expected file version before replacing data. ABORT discards staging. Four client slots accommodate owner, shell and two utilities. Revocation/expiry clears staging; a new grant gets a fresh generation. Server and process identities bind each native incarnation even though the in-memory generation counter restarts.

The separate private admin channel accepts bounded grant/revoke/detach/status words, never ordinary client file requests. Queued replies are discarded when replacing a grant; closed endpoints are closed on both sides so channels are reusable. One request per client per dispatch pass and an owned reply slot avoid allowing a full output queue to stop unrelated clients.

The native adapter is the only file-server code that imports block SDK calls. The kernel never imports rustic-fs, rustic-file-service or rustic-sdk. File content travels through immediate user-buffer copies and owned block requests; user pointers/DMA addresses do not escape into service policy.

## Durability boundary and open work

Metadata banks occupy sectors 8–12 and 13–17. Each header identifies sequence/next object ID and checksums. The 32 fixed object slots have two 1 KiB extents each beginning at sector 32. CRCs cover logical file bytes; unused extent padding is zeroed on replacement. Sector 0 and the final device sector are outside the format and checked by the host oracle.

A successful mutation includes the final flush. Submitted-write failure returns Uncertain and poisons that live volume until remount. A transport failure after a possible durable request also maps conservatively to Uncertain in the SDK. There is no automatic mutation replay. Existing-file replacement publishes old or new complete content under the tested flush/tear model. Create-and-write is two commits; there is no multi-file transaction.

Pure tests cover names, directory constraints, quota recovery, stale versions, read-only system roots, malformed metadata, checksum rejection and torn writes/flushes. Native terminal acceptance adds real disk I/O, object saturation, scopes, manual recovery, service restart and two VM boots with an independent on-disk oracle. Durable operation receipts/retry epochs, persistent workspace lineage, generic helper delegation and the complete logical service-v1 schema remain open. See [SERVICE-CONTRACTS.md](SERVICE-CONTRACTS.md), #12/#13/#22/#43.
