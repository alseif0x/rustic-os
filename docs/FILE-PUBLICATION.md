<!-- SPDX-License-Identifier: Apache-2.0 -->

# Replacement publication and cancellation boundaries

This is the filesystem prerequisite for asynchronous cancellation in #12/#13/#43. `rustic-fs` can prepare a replacement and advance its publication one disk command at a time. Both synchronous and nonblocking adapters use the same state machine and sector encoder. The [service control integration](FILE-CONTROL.md) now receives owner revocation while a logical replacement has outstanding I/O. Neither increment exposes `operations.cancel` or acknowledges durable queued/running work.

The distinction matters: a service that waits inside a complete replacement cannot handle a competing cancellation request. Separating the storage steps establishes the exact point at which a future service controller must stop submitting commands, and the point after which it must settle an uncertain effect instead. Merely adding a cancel opcode would not establish either property.

## Mechanics and ownership

`Volume::prepare_replace` returns a `Publication` producing a file node. `Volume::prepare_scoped` produces a retained receipt, or an already committed replay. Preparation validates arguments and prepares bounded memory without issuing a disk command. The publication exclusively borrows both the volume and its disk, preventing a second mutation through those owners while it exists. Disk implementations must also exclude aliases or external writers to the same volume.

`advance()` completes exactly one synchronous write or flush, then returns control. There is no outstanding command between synchronous calls. `poll_advance()` instead uses `PollDisk`, returning `Pending` while its adapter owns one command. Polling repeats that command's completion query; it never admits a second command. `pending()` identifies this unsettled interval. The synchronous entry point refuses to submit while an asynchronous command remains pending. `result()` returns no speculative version or receipt; it exposes the result only after the final flush succeeds, or for an existing committed replay. Repeated terminal calls issue no I/O.

The ordering and on-disk encoding remain compatible with formats 1–3:

1. Write the two inactive data sectors and flush them.
2. Write the inactive recovery bank when enabled, then the four metadata sectors, and flush them.
3. Write the checksummed publication header.
4. Flush the header, then adopt the new metadata in the live volume.

All metadata mutations share the same sector encoder and ordered metadata steps. Replacement staging, its retained operation record and file bytes still publish under one header; no second best-effort log is introduced. A recovery-capable replacement uses 17 write/flush commands; a legacy replacement uses 10.

## Outcomes

These are local mechanics states, not service-v1 operation states or transport dispositions.

| Publication phase | Cancellation result | Live writer behavior |
| --- | --- | --- |
| `Preparing` | `Cancelled` | Earlier commands have settled; scratch writes are abandoned. |
| `ReadyToPublish` | `Cancelled` | Data/metadata flushes succeeded, but no publication header was submitted. |
| `Preparing` with pending scratch I/O | `Draining` | Latch the stop request; wait for the submitted command to settle before reporting `Cancelled`. |
| `Settling` | `TooLate` | The header may have been submitted, including a pending completion; continue through the final flush. |
| `Committed` | `TooLate` | The original result is immutable; repeated calls perform no I/O. |
| `Cancelled` | `Cancelled` | Repeated cancellation/advance calls perform no I/O. |
| `Uncertain` | `Error::Uncertain` | Further commands are forbidden until explicit recovery/remount. |

A successful early cancellation leaves the previous file version and bytes selected. It stores **no durable cancellation record**, reserves no retry key and allocates no operation ID. After restart, lookup of that uncommitted preparation still returns `OutcomeUnknown`. A later explicit submission can therefore use that key. This is not sufficient for a public promise that an accepted operation remains cancelled across restart.

Any failed write/flush is conservatively uncertain, including a failure before the header. Failure is not proof that the device made no change. Cancellation cannot relabel that failure as rollback. Dropping a healthy preparation before publication releases the live writer only if no command remains pending; dropping during I/O or final settlement leaves it fenced. A forgotten publication guard also leaves the writer fenced. Fencing is established before I/O, including an adapter panic that unwinds on the host; successful settlement or known safe cancellation releases it.

## Validation

Run from the pinned environment in [development](DEVELOPMENT.md):

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py run --mode block-user --timeout 45
```

The dedicated filesystem tests cover every pre-publication cancellation boundary, late cancellation, repeated terminal calls, drop/forget, adapter unwind, failed/torn disk commands, historical replay after deletion, and the legacy boundary. The existing replacement/receipt/recovery tests continue to use the shared production path. Host visible/durable disk views are a model of flush ordering, not evidence of physical power-loss behavior.

The `block-user` fixture executes the library in a ring-3 native process using the actual copied block syscalls and VirtIO device. In a disjoint disposable volume, the first VM cancels at 16 boundaries, then tests late cancellation followed by successful settlement. A second VM recovers and replays the original receipt without writing. The host independently reads the file, receipt and checksums after each VM and requires the complete volume bytes to remain unchanged during replay. UART counts alone cannot pass. This proves the mechanics in RusticOS; it does not prove an IPC cancellation race or event-loop responsiveness.

## Next service increment

The native logical replacement path now owns and polls submitted requests while servicing a bounded subset of private owner control. It revalidates the live grant before each command, including the header, and drains outstanding I/O on revocation. Ordinary reads, legacy writes and storage administration remain synchronous; see the [precise supported scope](FILE-CONTROL.md). Public operation cancellation authority still needs to remain distinct from read/inspect authority.

Before acknowledging queued/running work, introduce a bounded durable admission identity and retained terminal cancellation evidence. Commit-sequence IDs from the current completed-only profile cannot identify an uncommitted cancellation: that sequence may later belong to another write. Add retry/restart conformance for the admission, cancellation, publication and reply-loss races before advertising the general `operations.cancel` contract. This preserves the completed-operation profile while giving its extension a testable storage boundary.
