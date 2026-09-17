<!-- SPDX-License-Identifier: Apache-2.0 -->

# Application workspaces: measured limits and selected budget (#51)

Stage one of [#51](https://github.com/alseif0x/rustic-os/issues/51): measure the consuming workload, state the structural limits that a larger workspace has to change, and select an explicit acceptance budget. No capacity is implemented by this document.

## Measured consumers

The native application artifacts that a workspace must eventually carry, as built on this machine:

| Artifact | Bytes |
| --- | ---: |
| `file-server.elf` | 215,296 |
| `shell.elf` | 208,824 |
| `block-probe.elf` | 186,024 |
| `utility.elf` | 126,560 |
| `supervisor.elf` | 68,128 |
| `tasks.elf` | 52,808 |
| `sdk-probe.elf` | 25,944 |
| every `*.manifest` | 128 |

The largest shipped executable is **215,296 bytes**, and the manifest that identifies it is 128 bytes. Anything that installs or launches an application from a workspace (#52) therefore needs files of that order, not of the current order.

## Measured limits of the current volume

| Limit | Value | Where |
| --- | --- | --- |
| Objects per volume | 32 | `crates/fs/src/lib.rs:23` |
| File length, policy | 1,024 bytes | `crates/fs/src/lib.rs:24` (`MAX_FILE`) |
| File length, structure | 65,535 bytes | `Node.length: u16`, `crates/fs/src/namespace.rs:15` |
| Volume structures | 174 sectors (about 87 KiB) | `crates/fs/src/lib.rs:25` (`SECTORS`) |
| Data placement | `32 + slot * 4 + bank * 2` sectors | `crates/fs/src/storage.rs:14` |
| Bank header | `8 + bank * 5` | `crates/fs/src/storage.rs:11` |
| Retained operation records | 2 | the issue's provenance note and the transfer slots |
| On-disk format | v5 | `docs/FILES.md` |

Two structural consequences follow, and they are why raising `MAX_FILE` alone is not enough:

1. **A file is one bank.** One bank is two sectors, exactly 1,024 bytes, so the policy limit and the placement geometry agree today. A larger file needs a bank-per-file mapping (an extent list or a chain) that the format does not have.
2. **The length field is 16 bits.** Even with extent mapping, `Node.length: u16` caps any file at 65,535 bytes — 3.3 times below the largest artifact measured above.
3. **The fixed structures are small.** 174 sectors bound the whole volume's payload area; a 64 MiB workspace is not a constants change but a different allocation scheme.

Whole-file buffers are the other bound: the file service holds `Transfer.data: [u8; MAX_FILE]` for two slots and the filesystem layer keeps whole-file buffers, so raising the per-file limit multiplies RAM per slot by the same factor. A 1 MiB file with two slots is 2 MiB of RAM before any copy, which must be measured against the reference profile rather than assumed.

## Selected workload and budget

**Workload:** install and launch one native application artifact from a workspace, the consumer that #52 names. Its measured size is 215,296 bytes plus a 128-byte manifest.

**Budget, pinned to that consumer:**

- **256 KiB per file** — covers the largest shipped executable with about 19% headroom; 1 MiB stays a planning ceiling to be revisited only with a larger measured artifact.
- **256 objects** — eight times today's 32, enough for application generations plus configuration and task records.
- **64 MiB of file data per volume** — the issue's planning target, kept as the total cap because 256 x 256 KiB would otherwise be 64 MiB exactly; the cap is what makes exhaustion decidable.
- **8 retained operation records** — four times today's two, so a client can recover a small window of unresolved outcomes instead of losing the third mutation to `Full`. Retention is bounded and its eviction policy belongs to the next stage; unresolved evidence is never silently dropped.

**What this implies for the format:** a v6 layout with extents (or a bank chain) per file, `Node.length` widened beyond 16 bits, an explicit total-data cap with typed exhaustion, and deliberate upgrade behavior from v5. Those are the implementation stages of #51, not this document.

## Not claimed

No capacity is implemented, no format is selected, and no filesystem is preselected by this document. The numbers above are measurements of one build on one machine; the 64 MiB figure is the issue's planning target and not a product promise. Whole-file RAM cost, interruption behavior, retention maintenance and migration are named here and remain to be measured and implemented in the following stages.
