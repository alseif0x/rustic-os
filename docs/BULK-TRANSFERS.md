<!-- SPDX-License-Identifier: Apache-2.0 -->

# Bulk transfers: mechanism decision for #50

This document is the decision gate that [#50](https://github.com/alseif0x/rustic-os/issues/50) requires before any implementation. It states what the current transport costs, compares the two candidate mechanisms against the same workload, ownership and failure requirements, and selects the smallest justified one. It also names what is still unmeasured, so the selection is not read as more than it is.

## Current transport, as implemented

| Path | Unit | Evidence |
| --- | --- | --- |
| IPC message | 24-byte header + **64-byte payload** (`crates/abi/src/ipc.rs:11`) | Source |
| File logical method | 40-byte `Packet.data` (`crates/abi/src/files.rs:5`) plus authenticated header | Source |
| Large replacement | `REPLACE_OPEN` / `REPLACE_CHUNK`* / `REPLACE_COMMIT`, one chunk per operation | `crates/file-service/src/transfer.rs:73` |
| Built-in bound | `MAX_FILE = 1024`, two transfer slots per server (`crates/fs/src/lib.rs:24`, `transfer.rs:18`) | Source |
| Byte validation | block/read buffers are validated up to 4096 bytes, but a message cannot carry a 4096-byte payload plus its header | #50 body |
| Control round trip | native read p50 **19.60 ms**, control call p50 **8.47 ms** (`docs/measurements/r0-initial.json`); 25.27 ms read p50 in the later read calibration (`r0-read-v1.json`) | Measured, five samples |

The load-bearing consequence: **one data-carrying operation moves at most 40 bytes**, so the cost is linear in operations, not bytes.

| Transfer | Operations (40 B each) | Data-path time at the measured p50 |
| --- | ---: | ---: |
| 1 KiB (current `MAX_FILE`) | 26 | ≈ 0.51 s |
| 64 KiB | 1,640 | ≈ 32 s |
| 1 MiB | 26,214 | ≈ 8.6 min |

That estimate is arithmetic on a measured round trip, not a measured bulk transfer: the transfer path adds scheduling, reply and per-chunk validation on top, so the real numbers are worse, not better. The 19.60/25.27 ms figures are whole native read calls including UART transport, scheduling and transcript writes (`docs/MEASUREMENTS.md:62`); they bound the data path from below rather than measuring it. **No throughput measurement exists in this repository today**; `docs/MEASUREMENTS.md` explicitly says the read calibration "does not measure 1 KiB throughput".

## Candidate A — bounded copied buffers/streams

Enlarge what one operation carries, keep the copy, keep the message queue as the only data path.

- Shape: a per-operation data area sized by a reviewed constant (for example 1 KiB or 4 KiB) instead of `Packet.data[40]`, with the chunk protocol unchanged; or a short stream of such operations per scheduling slice.
- Cost: memory is `channels × slots × payload` in the kernel and in every server. Four 1 KiB entries per channel across eight channels is 32 KiB of kernel storage before any copy buffer; 4 KiB entries make it 128 KiB, which needs its own budget and a `#20` measurement.
- Ownership and failure: unchanged and already tested. Authority, version checks, cancellation and reply semantics stay exactly where they are; a stopped reader fills a queue and backpressures for a bounded time; peer death releases the slot through the existing path.
- SMP: unchanged, because the kernel copy stays on one CPU (still a single-CPU assumption, `#53`).
- Teardown: unchanged, no new lifetime concept.

## Candidate B — shared-memory grant

Map a bounded window into both processes; control messages carry only the descriptor, offsets and acknowledgements.

- Shape: per-transfer mapping with explicit owner, permissions (say read-only for the receiver, never executable), lifetime tied to an operation, and revocation that unmaps before reuse.
- Cost: near-zero copying and near-zero per-byte protocol, at the price of a new kernel mechanism (map/unmap with a second owner), plus per-transfer page accounting.
- Ownership and failure: new invariants — who may mutate, what happens when the peer dies mid-transfer, when the mapping is revoked, and how a stale translation is prevented after frame reuse. `#53` explicitly asks for the current single-CPU invariants to be inventoried "before expanding mapping, shared-buffer or runtime interfaces"; the shared-mapping design is the change that issue is guarding, so doing it here would front-run it.
- Reclamation: a mapped frame cannot be freed until both owners are done; that is a new reclamation rule for the frame allocator, not a change to file policy.

## Comparison against the same requirements

| Requirement | Candidate A | Candidate B |
| --- | --- | --- |
| Exact bytes at/across boundaries | Same chunk contract, larger unit | New byte-range contract |
| Malformed length, truncation | Existing checks extended | Descriptor validation is new |
| Backpressure, stopped reader | Queue fills, bounded | Mapped frames stay held; needs its own bound |
| Peer death mid-transfer | Existing slot cleanup | New owner + reclamation rule |
| Sender authority | Unchanged | Must be re-established per mapping |
| Cancellation / revocation | Unchanged path | Must unmap before reuse |
| Bounded memory | Constant per slot, measurable | Pages per live transfer, measurable |
| SMP / stale translations | Not affected | Explicitly affected |
| Kernel mechanism new `unsafe` | None | New mapping owner and lifetime |

## Selection

**Candidate A, in its smallest form**, for this increment:

1. it is the only option whose failure, authority and cleanup behavior is already the tested behavior, so the increment adds capacity without adding trust;
2. Candidate B's cost is not mainly bytes copied but a new ownership and reclamation rule that `#53` asks to review first, and #50 is not the place to front-run that review;
3. the arithmetic above shows the current cost is dominated by operation count, so raising the unit per operation already removes most of the cost for the sizes the product can use today (`MAX_FILE = 1024`, workspace files measured in single-digit KiB).

Candidate B stays open and is not rejected: when a consumer needs multi-megabyte transfers, or when a measurement shows copy cost dominating control cost, it becomes the right answer and should be taken up together with `#53`.

## Open decision and unmeasured items

- **Consumer.** #50 requires "one real application or storage consumer". The selected one is the **file service replacement path** (`REPLACE_OPEN`/`REPLACE_CHUNK`/`REPLACE_COMMIT`), because it is the only transport today that a product already uses to move opaque bytes and it is the one measured here. The native SDK heap (`#49`) is the next candidate and is not served by this increment.
- **Target size.** The first step raises the per-operation data area to `MAX_FILE = 1024`, matching the existing storage bound, so a whole legal object moves in one operation instead of 26 and no new storage limit is implied. The next step (4 KiB, matching the read/block validation limit) needs its own `#20`-style budget: four 4 KiB entries per channel across eight channels is 128 KiB of kernel storage, which is real and must be measured before it is claimed. "Every queue entry becomes a page" is explicitly not the default.
- **Measurements still required before claiming throughput:** bytes per operation, operations per transfer, elapsed time for a bounded bulk transfer, memory cost per slot and queue, and control latency while data flows and while the reader is stopped.
- **Limits to carry:** the estimate above is arithmetic on a measured round trip; no bulk transfer has been executed yet; the `MAX_FILE = 1024` storage bound is a separate limit from the transport bound and is not changed by this decision.
