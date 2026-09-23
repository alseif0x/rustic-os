// SPDX-License-Identifier: Apache-2.0
use super::super::syscall::Action;
use super::sdk::{ELF, MANIFEST};
use super::{Exit, Manager, Memory, State};
use rustic_abi::{application, runtime as abi};
use rustic_kernel::{
    memory::{PAGE_SIZE, PagePermissions},
    process::lifecycle::{CAPACITY, Pid},
};
use sha2::{Digest, Sha256};

const USER_MANIFEST: u64 = 0x20_0000;
const USER_ELF: u64 = USER_MANIFEST + PAGE_SIZE;
const BAD_IDENTITY: u64 = 0x80_0000;
const BAD_DIGEST: u64 = 0x80_1000;
const BAD_ELF_MANIFEST: u64 = 0x80_2000;
const BAD_ELF: u64 = 0x80_3000;
const SHORT_MANIFEST: u64 = 0x80_4000;
const INVALID_COPY: u64 = 0x90_0000;
const CONTROL_BUFFER: u64 = 0xa0_0000;

pub(super) fn verify(manager: &mut Manager, memory: &mut Memory) {
    let free_before = memory.free_frames();
    let supervisor = super::sdk::launch_fixture(manager, memory);
    let attacker = super::sdk::launch_fixture(manager, memory);
    manager.session.supervisor = supervisor.0;
    map_user_bytes(manager, memory, supervisor, USER_MANIFEST, MANIFEST);
    map_user_bytes(manager, memory, supervisor, USER_ELF, ELF);
    map_user_bytes(manager, memory, supervisor, CONTROL_BUFFER, &[0; 64]);

    let mut invalid_identity = [0; application::SIZE];
    invalid_identity.copy_from_slice(MANIFEST);
    invalid_identity[32] = b'1';
    map_user_bytes(manager, memory, supervisor, BAD_IDENTITY, &invalid_identity);
    let mut bad_digest = [0; application::SIZE];
    bad_digest.copy_from_slice(MANIFEST);
    bad_digest[96] ^= 1;
    map_user_bytes(manager, memory, supervisor, BAD_DIGEST, &bad_digest);
    let invalid_elf = [0; 64];
    map_user_bytes(manager, memory, supervisor, BAD_ELF, &invalid_elf);
    let mut invalid_elf_manifest = [0; application::SIZE];
    invalid_elf_manifest.copy_from_slice(MANIFEST);
    invalid_elf_manifest[96..128].copy_from_slice(&Sha256::digest(invalid_elf));
    map_user_bytes(
        manager,
        memory,
        supervisor,
        BAD_ELF_MANIFEST,
        &invalid_elf_manifest,
    );
    map_user_page_at_end(manager, memory, supervisor, SHORT_MANIFEST, MANIFEST);
    let stage_baseline = memory.free_frames();
    assert_eq!(
        begin(
            manager,
            memory,
            supervisor,
            ELF.len(),
            BAD_IDENTITY,
            application::KNOWN
        ),
        Err(abi::Error::Protocol)
    );
    assert_eq!(memory.free_frames(), stage_baseline);

    let requests = application::Manifest::parse(MANIFEST).unwrap().requests;
    assert_ne!(requests, 0);
    assert_eq!(
        begin(manager, memory, supervisor, ELF.len(), USER_MANIFEST, 0),
        Err(abi::Error::Denied)
    );
    assert_eq!(
        begin(
            manager,
            memory,
            supervisor,
            ELF.len(),
            USER_MANIFEST,
            application::KNOWN | (1 << 63)
        ),
        Err(abi::Error::Invalid)
    );

    let short_manifest_address = SHORT_MANIFEST + PAGE_SIZE - 64;
    assert_eq!(
        begin(
            manager,
            memory,
            supervisor,
            ELF.len(),
            short_manifest_address,
            application::KNOWN
        ),
        Err(abi::Error::Address)
    );

    let pids_before: [Option<Pid>; CAPACITY] =
        core::array::from_fn(|slot| manager.table.pid_at(slot));
    memory.verify_frame_budget(1, |memory| {
        assert_eq!(
            begin(
                manager,
                memory,
                supervisor,
                ELF.len(),
                USER_MANIFEST,
                application::KNOWN
            ),
            Err(abi::Error::Full)
        );
    });
    assert_eq!(
        core::array::from_fn::<_, CAPACITY, _>(|slot| manager.table.pid_at(slot)),
        pids_before
    );
    assert_eq!(memory.free_frames(), stage_baseline);

    let first_generation = begin(
        manager,
        memory,
        supervisor,
        ELF.len(),
        USER_MANIFEST,
        application::KNOWN,
    )
    .unwrap();
    let allocated = memory.free_frames();
    assert_eq!(allocated, stage_baseline - image_pages());
    assert_eq!(
        begin(
            manager,
            memory,
            supervisor,
            ELF.len(),
            USER_MANIFEST,
            application::KNOWN
        ),
        Err(abi::Error::Busy)
    );
    assert_eq!(memory.free_frames(), allocated);
    assert_eq!(
        manager.staging_operation(
            attacker.0,
            [abi::STAGE_ABORT, first_generation, 0, 0, 0, 0, 0, 0],
            memory,
        ),
        Err(abi::Error::Denied)
    );
    assert_eq!(memory.free_frames(), allocated);
    assert_eq!(
        manager.staging_operation(
            supervisor.0,
            [abi::STAGE_ABORT, first_generation + 1, 0, 0, 0, 0, 0, 0],
            memory,
        ),
        Err(abi::Error::Invalid)
    );
    assert_eq!(memory.free_frames(), allocated);
    assert_eq!(
        manager.staging_operation(
            supervisor.0,
            [abi::STAGE_COPY, first_generation, 1, USER_ELF, 1, 0, 0, 0],
            memory,
        ),
        Err(abi::Error::Invalid)
    );
    assert_eq!(memory.free_frames(), stage_baseline);
    assert_eq!(
        manager.staging_operation(
            supervisor.0,
            [abi::STAGE_ABORT, first_generation, 0, 0, 0, 0, 0, 0],
            memory,
        ),
        Err(abi::Error::Busy)
    );

    let truncated = begin(
        manager,
        memory,
        supervisor,
        ELF.len(),
        USER_MANIFEST,
        application::KNOWN,
    )
    .unwrap();
    copy_chunk(manager, memory, supervisor, truncated, 0, USER_ELF, 64).unwrap();
    assert_eq!(
        commit(manager, memory, supervisor, truncated),
        Err(abi::Error::Size)
    );
    assert_eq!(memory.free_frames(), stage_baseline);

    let before_bad_commit: [Option<Pid>; CAPACITY] =
        core::array::from_fn(|slot| manager.table.pid_at(slot));
    let mismatch = begin(
        manager,
        memory,
        supervisor,
        ELF.len(),
        BAD_DIGEST,
        application::KNOWN,
    )
    .unwrap();
    copy_all(manager, memory, supervisor, mismatch);
    assert_eq!(
        commit(manager, memory, supervisor, mismatch),
        Err(abi::Error::Protocol)
    );
    assert_eq!(memory.free_frames(), stage_baseline);
    assert_eq!(
        core::array::from_fn::<_, CAPACITY, _>(|slot| manager.table.pid_at(slot)),
        before_bad_commit
    );

    let before_loader_failure: [Option<Pid>; CAPACITY] =
        core::array::from_fn(|slot| manager.table.pid_at(slot));
    let invalid = begin(
        manager,
        memory,
        supervisor,
        invalid_elf.len(),
        BAD_ELF_MANIFEST,
        application::KNOWN,
    )
    .unwrap();
    copy_chunk(
        manager,
        memory,
        supervisor,
        invalid,
        0,
        BAD_ELF,
        invalid_elf.len(),
    )
    .unwrap();
    assert_eq!(
        commit(manager, memory, supervisor, invalid),
        Err(abi::Error::Protocol)
    );
    assert_eq!(memory.free_frames(), stage_baseline);
    assert_eq!(
        core::array::from_fn::<_, CAPACITY, _>(|slot| manager.table.pid_at(slot)),
        before_loader_failure
    );

    let missing_source = begin(
        manager,
        memory,
        supervisor,
        ELF.len(),
        USER_MANIFEST,
        application::KNOWN,
    )
    .unwrap();
    assert_eq!(
        copy_chunk(
            manager,
            memory,
            supervisor,
            missing_source,
            0,
            INVALID_COPY,
            64
        ),
        Err(abi::Error::Address)
    );
    assert_eq!(memory.free_frames(), stage_baseline);

    let abort = begin(
        manager,
        memory,
        supervisor,
        ELF.len(),
        USER_MANIFEST,
        application::KNOWN,
    )
    .unwrap();
    assert_eq!(
        manager.staging_operation(
            supervisor.0,
            [abi::STAGE_ABORT, abort, 0, 0, 0, 0, 0, 0],
            memory,
        ),
        Ok([0; 8])
    );
    assert_eq!(memory.free_frames(), stage_baseline);

    let full = begin(
        manager,
        memory,
        supervisor,
        ELF.len(),
        USER_MANIFEST,
        application::KNOWN,
    )
    .unwrap();
    copy_all(manager, memory, supervisor, full);
    let mut fillers = [None; CAPACITY];
    let mut filler_count = 0;
    while manager.processes.iter().flatten().count() < CAPACITY {
        let child = super::sdk::launch_fixture(manager, memory);
        fillers[filler_count] = Some(child);
        filler_count += 1;
    }
    let before_full_commit = memory.free_frames();
    assert_eq!(
        commit(manager, memory, supervisor, full),
        Err(abi::Error::Full)
    );
    assert_eq!(memory.free_frames(), before_full_commit + image_pages());
    for child in fillers.into_iter().flatten() {
        manager.kill(child).unwrap();
        assert_eq!(manager.wait(memory, child).unwrap(), Some(Exit::Killed));
    }
    assert_eq!(memory.free_frames(), stage_baseline);

    let success = begin(
        manager,
        memory,
        supervisor,
        ELF.len(),
        USER_MANIFEST,
        application::KNOWN,
    )
    .unwrap();
    copy_all(manager, memory, supervisor, success);
    let child = Pid(commit(manager, memory, supervisor, success).unwrap()[0]);
    assert_eq!(manager.state(child), Ok(State::Dormant));
    let process = manager.process(child).unwrap();
    assert_eq!(process.parent, supervisor.0);
    assert_eq!(process.program, abi::DYNAMIC_IMAGE);
    assert_eq!(manager.broker.counts(), (0, 0));
    assert_eq!(manager.block.broker.counts(), (0, 0));
    manager.kill(child).unwrap();
    assert_eq!(manager.wait(memory, child).unwrap(), Some(Exit::Killed));
    assert_eq!(memory.free_frames(), stage_baseline);

    // A non-staging CONTROL refusal must not discard an in-flight image.
    let reap_target = super::sdk::launch_fixture(manager, memory);
    manager.processes[manager.table.slot(reap_target).unwrap()]
        .as_mut()
        .unwrap()
        .parent = supervisor.0;
    let control_generation = control(
        manager,
        memory,
        supervisor,
        [
            abi::STAGE_BEGIN,
            ELF.len() as u64,
            USER_MANIFEST,
            application::KNOWN,
            0,
            0,
            0,
            0,
        ],
    )
    .unwrap()[0];
    assert_eq!(
        control(
            manager,
            memory,
            supervisor,
            [abi::REAP, reap_target.0, 0, 0, 0, 0, 0, 0]
        ),
        Err(abi::Error::Busy)
    );
    let mut offset = 0;
    while offset < ELF.len() {
        let length = (ELF.len() - offset).min(abi::STAGE_CHUNK_BYTES);
        control(
            manager,
            memory,
            supervisor,
            [
                abi::STAGE_COPY,
                control_generation,
                offset as u64,
                USER_ELF + offset as u64,
                length as u64,
                0,
                0,
                0,
            ],
        )
        .unwrap();
        offset += length;
    }
    let control_child = Pid(control(
        manager,
        memory,
        supervisor,
        [abi::STAGE_COMMIT, control_generation, 0, 0, 0, 0, 0, 0],
    )
    .unwrap()[0]);
    assert_eq!(manager.state(control_child), Ok(State::Dormant));
    manager.kill(control_child).unwrap();
    assert_eq!(
        manager.wait(memory, control_child).unwrap(),
        Some(Exit::Killed)
    );
    manager.kill(reap_target).unwrap();
    assert_eq!(
        manager.wait(memory, reap_target).unwrap(),
        Some(Exit::Killed)
    );
    assert_eq!(memory.free_frames(), stage_baseline);

    let shutdown_generation = control(
        manager,
        memory,
        supervisor,
        [
            abi::STAGE_BEGIN,
            ELF.len() as u64,
            USER_MANIFEST,
            application::KNOWN,
            0,
            0,
            0,
            0,
        ],
    )
    .unwrap()[0];
    assert_ne!(shutdown_generation, 0);
    assert_eq!(memory.free_frames(), stage_baseline - image_pages());
    assert_eq!(
        control(
            manager,
            memory,
            supervisor,
            [abi::SHUTDOWN, 0, 0, 0, 0, 0, 0, 0]
        ),
        Ok([0; 8])
    );
    assert_eq!(memory.free_frames(), stage_baseline);
    assert!(manager.session.shutdown);

    manager.session.supervisor = 0;
    manager.kill(attacker).unwrap();
    assert_eq!(manager.wait(memory, attacker).unwrap(), Some(Exit::Killed));
    manager.kill(supervisor).unwrap();
    assert_eq!(
        manager.wait(memory, supervisor).unwrap(),
        Some(Exit::Killed)
    );
    assert_eq!(memory.free_frames(), free_before);
}

fn control(
    manager: &mut Manager,
    memory: &mut Memory,
    supervisor: Pid,
    words: [u64; 8],
) -> Result<[u64; 8], abi::Error> {
    let slot = manager.table.slot(supervisor).unwrap();
    let process = manager.processes[slot].as_mut().unwrap();
    memory
        .copy_to_user(&process.space, CONTROL_BUFFER, &abi::encode(words))
        .unwrap();
    process.frame.set_arguments([CONTROL_BUFFER, 64, 0]);
    let result = match manager.control(supervisor, memory) {
        Action::Return(result) => result,
        _ => panic!("control operation returned a scheduler action"),
    };
    if abi::Error::decode(result)? != 64 {
        return Err(abi::Error::Protocol);
    }
    let process = manager.process(supervisor).unwrap();
    let mut bytes = [0; 64];
    memory
        .copy_from_user(&process.space, CONTROL_BUFFER, &mut bytes)
        .map_err(|_| abi::Error::Address)?;
    abi::decode(&bytes)
}

fn begin(
    manager: &mut Manager,
    memory: &mut Memory,
    supervisor: Pid,
    length: usize,
    manifest: u64,
    available: u64,
) -> Result<u64, abi::Error> {
    manager
        .staging_operation(
            supervisor.0,
            [
                abi::STAGE_BEGIN,
                length as u64,
                manifest,
                available,
                0,
                0,
                0,
                0,
            ],
            memory,
        )
        .map(|result| result[0])
}

fn copy_chunk(
    manager: &mut Manager,
    memory: &mut Memory,
    supervisor: Pid,
    generation: u64,
    offset: usize,
    source: u64,
    length: usize,
) -> Result<(), abi::Error> {
    manager
        .staging_operation(
            supervisor.0,
            [
                abi::STAGE_COPY,
                generation,
                offset as u64,
                source,
                length as u64,
                0,
                0,
                0,
            ],
            memory,
        )
        .map(|_| ())
}

fn copy_all(manager: &mut Manager, memory: &mut Memory, supervisor: Pid, generation: u64) {
    let mut offset = 0;
    while offset < ELF.len() {
        let length = (ELF.len() - offset).min(abi::STAGE_CHUNK_BYTES);
        copy_chunk(
            manager,
            memory,
            supervisor,
            generation,
            offset,
            USER_ELF + offset as u64,
            length,
        )
        .unwrap();
        offset += length;
    }
}

fn commit(
    manager: &mut Manager,
    memory: &mut Memory,
    supervisor: Pid,
    generation: u64,
) -> Result<[u64; 8], abi::Error> {
    manager.staging_operation(
        supervisor.0,
        [abi::STAGE_COMMIT, generation, 0, 0, 0, 0, 0, 0],
        memory,
    )
}

fn map_user_bytes(
    manager: &mut Manager,
    memory: &mut Memory,
    owner: Pid,
    address: u64,
    bytes: &[u8],
) {
    let slot = manager.table.slot(owner).unwrap();
    let process = manager.processes[slot].as_mut().unwrap();
    for (index, chunk) in bytes.chunks(PAGE_SIZE as usize).enumerate() {
        memory
            .load_page(
                &mut process.space,
                address + index as u64 * PAGE_SIZE,
                PagePermissions {
                    writable: true,
                    executable: false,
                    user: true,
                },
                0,
                chunk,
            )
            .unwrap();
    }
}

fn map_user_page_at_end(
    manager: &mut Manager,
    memory: &mut Memory,
    owner: Pid,
    address: u64,
    bytes: &[u8],
) {
    let slot = manager.table.slot(owner).unwrap();
    let process = manager.processes[slot].as_mut().unwrap();
    memory
        .load_page(
            &mut process.space,
            address,
            PagePermissions {
                writable: true,
                executable: false,
                user: true,
            },
            PAGE_SIZE as usize - 64,
            &bytes[..64],
        )
        .unwrap();
}

fn image_pages() -> usize {
    ELF.len().div_ceil(PAGE_SIZE as usize)
}
