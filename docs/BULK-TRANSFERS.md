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

## Addendum: the transport model decides between two sub-shapes of Candidate A

Reading the actual transport after the first version of this document changes part of the comparison and must be recorded before implementation, because it selects *which* copied-buffer shape is smallest.

- The kernel moves **one message per `send`**, sized `HEADER + PAYLOAD = 24 + 64 = 88` bytes (`crates/abi/src/ipc.rs:10-12`, `kernel/src/ipc/message.rs`), and a channel queue holds **two messages** (`kernel/src/ipc/channel.rs:4`).
- The SDK performs **one logical call as one message** (`begin` sends, `poll` receives one reply, `crates/sdk/src/rpc/client.rs:46,68`), and the file client already splits request bytes into `DATA`-sized chunks, one logical operation each (`crates/sdk/src/files/staging.rs:28`).
- The file `Packet` is exactly one message wide: `SIZE = 64`, `DATA = 40` (`crates/abi/src/files.rs:5,33`).

Two sub-shapes follow, and they are not equally invasive:

| Sub-shape | Change | Cost | Where the risk sits |
| --- | --- | --- | --- |
| **A1 — grow the IPC payload to the file packet size** | `PAYLOAD`/`MAX_MESSAGE` in `crates/abi/src/ipc.rs` and `Packet::SIZE`/`DATA` in the file ABI; a 1 KiB packet still travels as **one** message | `(HEADER + 1024)` bytes per queue entry, and a kernel queue holds two entries per direction across eight channels: roughly **16.8 KiB** of kernel storage. A large fixed message is also paid by small operations such as `STAT` | The kernel trust envelope and every service's queue; the file call model is untouched |
| **A2 — keep the 64-byte message and fragment** | A fragment protocol in the file ABI plus reassembly before dispatch | One extra KiB in the SDK and in the server's two transfer slots; kernel untouched | Call dispatch: the server must accumulate fragments and stay silent until a call is complete, so every operation's request path changes |

`20.4 KiB` is quoted in some notes as the A1 cost including both directions; the per-direction figure is the 8 queues × 2 entries × 1048 bytes ≈ 16.4 KiB used here.

**Selected: A1.** With "one logical call = one message" as the current model, growing the message keeps that model and confines the change to constants and buffers, while A2 changes dispatch for every operation. A1's cost is kernel storage that a `#20`-style measurement can bound; A2's cost is regression risk across twenty logical methods. A1 is therefore the smallest *justified* mechanism, and A2 becomes the fallback only if a measured kernel-storage budget rejects 1 KiB messages.

Consequence for the payload step: with A1 the first step can still be `MAX_FILE = 1024` bytes, and the `#20` measurement must bound the queue storage above rather than assuming it.


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

**Candidate A, in its smallest form A1: one logical call stays one message, and the message grows to carry the file packet.** Rationale, in order of weight:

1. it is the only option whose failure, authority and cleanup behavior is already the tested behavior, so the increment adds capacity without adding trust;
2. Candidate B's cost is not mainly bytes copied but a new ownership and reclamation rule that `#53` asks to review first, and #50 is not the place to front-run that review;
3. the arithmetic above shows the current cost is dominated by operation count, so raising the unit per operation removes most of the cost for the sizes the product can use today (`MAX_FILE = 1024`);
4. between A1 and A2, A1 keeps the one-call-one-message model, so no operation's dispatch path changes and no reassembly state has to be threaded through twenty logical methods. A2's smaller kernel footprint is not worth that regression surface, and the kernel storage A1 costs is a constant a `#20`-style measurement can bound.

Candidate B stays open and is not rejected: when a consumer needs multi-megabyte transfers, or when a measurement shows copy cost dominating control cost, it becomes the right answer and should be taken up together with `#53`. A2 stays documented as the fallback if the measured queue-storage budget rejects 1 KiB messages.

## Open decision and unmeasured items

- **Consumer.** #50 requires "one real application or storage consumer". The selected one is the **file service replacement path** (`REPLACE_OPEN`/`REPLACE_CHUNK`/`REPLACE_COMMIT`), because it is the only transport today that a product already uses to move opaque bytes and it is the one measured here. The native SDK heap (`#49`) is the next candidate and is not served by this increment.
- **Target size.** The first step raises the per-operation data area to `MAX_FILE = 1024`, matching the existing storage bound, so a whole legal object moves in one operation instead of 26 and no new storage limit is implied. The next step (4 KiB, matching the read/block validation limit) needs its own `#20`-style budget; with A1 that is `(24 + 4096)` bytes per queue entry, and "every queue entry becomes a page" is explicitly not the default.
- **Measurements still required before claiming throughput:** bytes per operation, operations per transfer, elapsed time for a bounded bulk transfer, memory cost per slot and queue, and control latency while data flows and while the reader is stopped.
- **Limits to carry:** the estimate above is arithmetic on a measured round trip; no bulk transfer has been executed yet; the `MAX_FILE = 1024` storage bound is a separate limit from the transport bound and is not changed by this decision.

## Implementation plan for A1 (next increment, not yet executed)

1. `crates/abi/src/ipc.rs`: raise `PAYLOAD` to 1024 so `MAX_MESSAGE` is 1048. The kernel send/receive syscall already takes an explicit length and the broker copies a variable-length message, so no syscall change is expected (`kernel/src/process/runtime/syscall/ipc.rs`).
2. `crates/abi/src/files.rs`: raise `SIZE` to `24 + 1024` and `DATA` to 1024. Every logical method that carried 40 inline bytes now carries up to 1024, and the `count` field keeps the exact length.
3. `crates/sdk/src/files/staging.rs`: one chunk per operation replaces `bytes.chunks(DATA)` with 40-byte pieces, so a whole bounded object is one operation.
4. `crates/file-service/src/{transfer,validation}.rs`: keep `MAX_FILE = 1024` as the storage bound, keep the count/offset/zero-padding checks, and confirm that a 1024-byte chunk is accepted rather than rejected by a stale 40-byte assumption.
5. `contracts/services/v1/*.json`: update any schema that encodes the old message or inline length, and keep the JSON and the Rust constants checked against each other.
6. Tests in the owning layers: the file-service rejects a malformed length, a truncated run, a wrong offset and a chunk that exceeds the declared total; the SDK sends one chunk for a 1024-byte object; the kernel IPC tests keep passing with the larger message.
7. Measurements before any throughput claim: bytes per operation, operations per transfer, elapsed time for a 1024-byte replacement and read, kernel queue storage, and control latency while data flows and while the reader is stopped.
8. Known risk to bound first: one `Message` is `MAX_MESSAGE` bytes on the SDK's 64 KiB user stack. A 1 KiB message is bounded and fits, but the increment must state the measured stack headroom rather than assume it.

## A1 measured cost and first results (implemented)

`cargo xtask check` passes with the payload change (378 Rust tests, fmt, Clippy `-D warnings`, kernel and guest builds) and the `ok` boot fixture runs.

- **What one operation carries now:** the file data area is the transport payload minus its 24-byte header. `Packet::count` is one byte, so no single operation may exceed `MAX_CHUNK = 255` bytes; the server enforces it and the SDK sends and requests at most that. A 1 KiB object therefore moves in four operations instead of 26.
- **Receipts stopped fragmenting:** one reply now carries the whole 104-byte receipt, so the SDK reads it in one exchange and the server refuses a nonzero fragment offset. `OPERATION_PART` remains in the ABI but is no longer required for a correct client.
- **Measured kernel cost:** `metadata_bytes` in `RUSTIC PROCESS_MEMORY` is `size_of::<Manager>()` and grew from about 9.6 KiB to **40,536 B**, because the manager holds the channel message buffers (eight channels, two directions, a two-message queue each) and each entry grew from 88 to 1048 bytes. The host oracle now derives its bound from those constants instead of a literal calibrated to the old size. This is the trade the decision predicted, and it is now a measured number rather than an estimate.
- **Latent bugs the larger data area exposed:** five reserved-byte checks were written as open-ended slices (`data[36..] != [0; 4]`), which silently compared a long tail against a short zero array and therefore never rejected anything; the admission activity, identity and status decoders, the read header and the recovery receipt now check exactly their own field range.
- **Still owed before a throughput claim:** elapsed time for a bounded bulk transfer, bytes per operation and per transfer, control latency while data flows and while the reader is stopped, and the stack headroom of a `MAX_MESSAGE`-sized buffer on the SDK's 64 KiB user stack.
