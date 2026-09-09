<!-- SPDX-License-Identifier: Apache-2.0 -->

# Bounded block storage

Implemented for #35, before a filesystem or storage service. The kernel can discover the dedicated R0 virtual disk, read/write one sector, flush, reject invalid requests and reclaim DMA memory after a confirmed reset. No filesystem, paths, file permissions or user-visible block syscall is included.

## Transport decision and reference device

R0 uses the legacy PCI I/O transport of `virtio-blk-pci`, explicitly configured with `disable-modern=on,disable-legacy=off`. The dedicated persistence device is PCI **00:06.0**, vendor/device 1af4:1001, revision 0. The boot volume is a separate read-only device and is never claimed by this driver.

The initial choice reuses existing x86 port I/O and coherent RAM. Modern PCI would also require a reviewed MMIO/BAR-mapping transport; that is a separate extension, not silently implied by this implementation. This driver does not claim VirtIO 1.2 compliance or general PCI enumeration. Revisit this choice when another platform/device needs modern transport.

The [VirtIO 1.2 specification](https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html) documents the legacy PCI layout, split queues and block operations. The implementation uses its reset-completion requirement: after status becomes zero, the device must stop interacting with queues until reinitialization. Capacity is expressed in 512-byte sectors; legacy capacity is read repeatedly for consistency. Legacy used-length fields are not relied on; receive storage is initialized and bounded.

Reuse review considered [virtio-drivers](https://github.com/rcore-os/virtio-drivers), an MIT-licensed no_std framework supporting multiple devices and a memory-mapped PCI transport. Its README was reviewed, not a complete source audit or port. For this bounded port-I/O increment, an original small implementation avoids adding that transport/HAL integration at the same time. Reconsider the framework for broader device support. No third-party driver code was copied and Cargo dependencies remain unchanged.

| Parameter | R0 value |
| --- | --- |
| Guest | x86_64, one CPU, 256 MiB, pc-q35-8.2/qemu64/TCG |
| Device | virtio-blk-pci at 00:06.0, one queue, queue-size=8, vectors=0 |
| Disk | Fresh sparse raw file, 4 GiB logical capacity, 8,388,608 sectors |
| Host cache | writeback; real FLUSH command required by driver initialization |
| Negotiated features | FLUSH and the offered read-only property only |
| Completion | Synchronous polling; INTx disabled, MSI-X absent |
| Request limit | One in flight, exactly 512 bytes per read/write |
| DMA allocation | Three contiguous 4 KiB frames in the tested queue configuration |
| Request budget | 25 PIT ticks or 5,000,000 polls, whichever is reached first |
| Reset budget | At most 100,000 status reads; no unbounded loop |

## Module and ownership boundaries

| Module | Responsibility |
| --- | --- |
| `kernel/src/block/` | Pure geometry, length/range validation and queue-layout/completion rules |
| `arch/x86_64/pci.rs` | Exclusive fixed PCI function, bounded I/O BAR access and reset |
| `arch/x86_64/memory/dma.rs` | Owned coherent DMA pages, checked volatile scalar accesses and explicit release |
| `drivers/block/device.rs` | Device initialization, geometry and shutdown |
| `drivers/block/queue.rs` | Descriptor publication, memory ordering, completion and timeout |
| `drivers/block/request.rs` | Internal read/write/flush API |
| `drivers/block/tests.rs` | Guest acceptance and explicitly identified fault injection |
| `tools/boot_support/block_runner.py` | Disposable disk lifetime, independent host oracle and separate VM boots |

`Device::open`, `geometry`, `read`, `write`, `flush` and consuming `shutdown` form the internal API. Geometry reports capacity and read-only state. Errors distinguish missing/unsupported/busy devices, memory shortage, invalid size/range, read-only denial, device I/O error, timeout, protocol violation and failed reset. Unsupported sizes are rejected rather than partially processed.

The frame allocator searches a bounded bitmap for 1–4 contiguous free pages before mutating ownership. Fragmented or exhausted allocation leaves bookkeeping unchanged. The DMA owner zeroes fresh RAM before publishing its physical address. No Rust slice/reference to device-mutated storage escapes; accesses use checked aligned volatile scalars. Descriptor addresses are owned physical RAM, never caller pointers.

One request uses a header, optional 512-byte buffer and status byte. Fences order publication and completion; port instructions retain compiler memory effects. Queue indices wrap as u16, ring slots are bounded and completions must identify the expected head. An invalid completion poisons the device. Caller read buffers are updated only after a successful completion.

Timeout does not return DMA ownership to the allocator. The driver refuses subsequent submissions and shutdown confirms device status zero before release. If reset cannot be confirmed, frames and the PCI claim remain quarantined for the rest of the boot; bus mastering is disabled as additional containment. Merely dropping an open device also does not release DMA pages. Explicit shutdown is required. There is no automatic retry of a timed-out write whose effects may be uncertain.

The R0 device is trusted and DMA-coherent. No IOMMU protection, hostile-device containment, hot-unplug recovery, concurrent drivers, SMP, dynamic capacity changes or scatter/gather API is established. PCI configuration access is serialized by the one-CPU execution model; IRQ handlers never use its selector ports.

## Guest and host acceptance

```sh
cargo xtask check
python3 -m unittest discover -s tools/tests -v
python3 tools/boot.py run --mode block-persist
python3 tools/boot.py test --timeout 30
```

| Scenario | Required evidence |
| --- | --- |
| block-persist | First VM writes sectors 8, 9 and the final sector, flushes and reads back. A separately started VM reads the existing patterns. Repeated reads exercise ring-slot reuse. |
| block-readonly | Read succeeds; write is denied before queue submission; selected disk contents stay zero. |
| block-error | Test-only admission bypass submits an out-of-range write and unsupported opcode to the real device. Correct error statuses return, followed by a successful read. |
| block-timeout | Test-only notification suppression leaves a real request pending. Budget expires, reuse is rejected, confirmed reset reclaims memory, and reopening/read succeeds. |
| block-missing | No device is attached at the assigned function; initialization returns Missing without allocating DMA memory. |

Each attached-device scenario also rejects five malformed/range/size requests and verifies initialization failure with only two available frames. Before/after free counts match. The error and timeout injections are explicit fixtures, not claims of a physical unplug or a malicious-device audit.

The trusted host creates the disk itself; no disk path or QEMU flag is accepted from the candidate. It verifies selected sectors independently after each successful boot, including sector 0 as a guard. A serial success message without the corresponding bytes cannot pass. Persisted data is checked after full QEMU termination and restart, not merely a driver reopen.

`block.json` records logical/allocated bytes, sector selection, selected-content SHA-256 and number of VM boots. `blocks.bin` preserves exactly those 2048 selected bytes. The checksum describes that selection, not the whole 4 GiB disk. Initial direct evidence occupied 8192 physical bytes; the disposable disk is removed after the scenario. This validates controlled restart, not sudden power loss or filesystem crash consistency.

The full suites contain **37 Rust tests, 26 Python tests, 18 direct VM scenarios and 22 isolated scenarios**. Persistence adds a second VM boot within one scenario. Previous IRQ, memory, process, IPC and SDK cases remain required. [#35](https://github.com/alseif0x/rustic-os/issues/35) records the exact accepted revision, CI and measurements.

The isolated worker uses the same trusted disk runner. Its logical per-file limit is 4 GiB to permit a sparse file; actual 1 GiB workspace/128 MiB temporary tmpfs quotas remain unchanged. Only bounded metadata/selected bytes are exported, never the raw persistence disk. Block persistence receives two per-VM timeout budgets plus the existing packaging margin.

## Next boundary

The file service is #12, after #44 provides bounded user-mode access under [ADR-0002](architecture/ADR-0002-authority-and-delegation.md) and #6's contracts. IPC still has a 64-byte payload limit. Define a bounded, authorized service transfer protocol; do not turn a disk sector or numeric PCI address into implicit user authority. Application SDK, filesystem consistency and product permission policy stay above this driver.
