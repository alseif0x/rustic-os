<!-- SPDX-License-Identifier: Apache-2.0 -->

# H1 file foundation acceptance

This matrix bounds original #12 acceptance separately from #47's H2 asynchronous execution/public cancellation. It maps existing storage behavior to executable evidence rather than adding further product requirements to H1. The tiny format remains experimental; whole H1 still includes separate authority and observation work in #13/#15.

## Requirement and evidence matrix

| Original #12 criterion | Owning evidence | Boundary |
| --- | --- | --- |
| Directory/read/write/metadata with defined errors | Native `terminal-test`; `crates/fs/tests/volume.rs`; typed SDK and file-service tests | 32 objects, 31-byte components, 1 KiB files |
| Distinct system/data/config/workspace roots and real authorization | `volume.rs`, native authority C/H cases, read-only system-root denials | Object ancestry and trusted context; not textual-prefix authority |
| Create/modify/restart/persist | Two-boot terminal oracle; service restarts; scoped operation and admission recovery | QEMU reference disk and documented flush semantics |
| Full volume and recovered capacity | `storage_cases.py` fills 26 remaining slots, verifies Full leaves selected disk bytes unchanged, releases one slot and writes through it | Exhaustion of the bounded format, not consuming the entire 4 GiB virtual device |
| Invalid paths/names and protected roots | Native overlong component, traversal through a file, missing parent and read-only root; independent unchanged-disk comparisons | Symlinks/hard links are unsupported and explicitly outside this initial format; no fabricated link traversal claim |
| Partial writes/interruption | `recovery.rs` tests every write/flush/torn-sector cut; `receipts.rs`, `operations.rs`, publication/admission tests; actual native EIO/reboot scenarios | Old/new complete file under the selected model; unknown submitted effects remain uncertain until recovery |
| Base/service recovery preserves user data | Native terminal service restart, invalid-policy/manual recovery and second VM boot with independent content checks | Does not establish future package activation or arbitrary data migration rollback |

Source paths: [host volume tests](../crates/fs/tests/volume.rs), [host crash-cut tests](../crates/fs/tests/recovery.rs), [native storage cases](../tools/terminal_support/storage_cases.py), [terminal driver](../tools/terminal_support/acceptance.py), [independent disk reader](../tools/terminal_support/oracle.py), [native recovery guide](FILE-RECOVERY.md), [workspace operations](FILE-OPERATIONS.md), [admission API](FILE-ADMISSION-API.md).

## Reproduction

Use the configured environment in [DEVELOPMENT.md](DEVELOPMENT.md):

```sh
source ~/.cargo/env
cargo test -p rustic-fs --locked
python3 -m unittest discover -s tools/tests -v
python3 tools/terminal_test.py
python3 tools/boot.py run --mode recovery-test --timeout 60
```

`terminal.json` includes a `storage` section: five named failures, the observed sequence and before/after hashes, immediate capacity reuse and final file count. The test compares the actual selected 174 sectors while the guest is at a settled command prompt; the two-boot oracle independently verifies final persistent files. Runtime comparisons do not claim that intermediate disks can be reconstructed from their hashes alone. Failures preserve bounded raw disk evidence after the owned VM stops and before the disposable disk is removed.

The host oracle tests deliberately challenge a denial that modified storage and ambiguous errors. Host fixtures do not establish native execution.

## Validation record

The initial strengthened native run on 2026-09-11 completed 945 first-phase commands and two boots, including all five unchanged-disk denials and immediate slot reuse. Its guest is the unchanged `79ae909` implementation: build `fb966407231b818c`, kernel SHA-256 `3f638b35b07b1b0629be4e3d31e133b1a1419670dd306422a9bd39daee62de4f`. Command totals vary with polling and are not a performance metric. Exact publication/CI evidence is recorded in #12.

The earlier [79ae909 CI](https://github.com/alseif0x/rustic-os/actions/runs/34629155402) separately passed 22 direct scenarios, 23 recovery groups/46 boots, selected delayed-device faults and the isolated suite. It predates the strengthened storage evidence above and must not be cited as running the new test. A relocation or a document does not satisfy a missing test.

## Remaining product work

#47 owns responsive public status/cancellation and bounded execution. #22 owns live discovery and complete M1; #43 owns full conformance/adapter comparisons; #13 retains its authority/policy acceptance. Larger files, independent retention, links and a general filesystem require new application-driven scope and a reuse review. Closing #12's bounded foundation does not establish these capabilities.
