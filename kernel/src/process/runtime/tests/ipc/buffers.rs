// SPDX-License-Identifier: Apache-2.0
use super::*;

fn called(manager: &mut Manager, memory: &mut Memory, pid: Pid, count: u64) {
    for _ in 0..32 {
        if manager.process(pid).unwrap().calls >= count {
            return;
        }
        manager.step(memory).unwrap();
    }
    panic!("IPC fixture did not return from syscall");
}
fn peer(manager: &mut Manager, memory: &mut Memory) -> Pid {
    manager
        .create(memory, super::super::image(false), [1, 0, 77])
        .unwrap()
}
pub(super) fn verify(manager: &mut Manager, memory: &mut Memory) -> usize {
    let kernel = verify as *const () as u64;
    let cases = [
        ("send-null", 3, 0, Error::Address),
        ("send-overflow", 3, u64::MAX - 15, Error::Address),
        ("send-kernel", 3, kernel, Error::Address),
        ("send-unmapped", 3, 0x700000, Error::Address),
        ("send-cross-hole", 3, 0x600ff0, Error::Address),
        ("send-noncanonical", 3, 1 << 47, Error::Address),
        ("send-empty", 4, 0, Error::Size),
        ("send-short", 4, 23, Error::Size),
        ("send-large", 4, 89, Error::Size),
        ("send-huge", 4, u64::MAX, Error::Size),
        ("version", 7, 2, Error::Version),
        ("opcode", 8, 2, Error::Message),
        ("sender-spoof", 9, 1, Error::Message),
        ("receive-code", 5, 0x400000, Error::Address),
        ("receive-kernel", 5, kernel, Error::Address),
        ("receive-unmapped", 5, 0x700000, Error::Address),
        ("receive-cross-hole", 5, 0x600ff0, Error::Address),
        ("receive-null", 5, 0, Error::Address),
        ("receive-short", 6, 24, Error::Size),
        ("foreign-handle", 3, 0x600100, Error::Handle),
        ("stale-handle", 10, 0, Error::Handle),
        ("attenuation", 3, 0x600100, Error::Denied),
    ];
    for (name, mode, argument, error) in cases {
        let before = memory.free_frames();
        let actor = create(manager, memory);
        let other = peer(manager, memory);
        let (mut handle, remote) = manager.connect(actor, other).unwrap();
        if name == "foreign-handle" {
            handle = remote;
        }
        if name == "attenuation" {
            handle = manager.transfer(actor, handle, actor, abi::READ).unwrap();
        }
        if mode == 5 || mode == 6 {
            manager.broker.send(other.0, remote, &packet()).unwrap();
        }
        if name == "receive-cross-hole" {
            memory
                .copy_to_user(
                    &manager.process(actor).unwrap().space,
                    0x600ff0,
                    &[0xa5; 16],
                )
                .unwrap();
        }
        manager.bootstrap(actor, [mode, handle, argument]);
        called(manager, memory, actor, if mode == 10 { 2 } else { 1 });
        assert_eq!(
            manager.process(actor).unwrap().frame.call().0,
            error.code(),
            "{name}"
        );
        if mode == 5 || mode == 6 {
            assert_eq!(manager.broker.peek(actor.0, handle).unwrap().length(), 32);
        } else if mode != 10 {
            assert_eq!(manager.broker.peek(other.0, remote), Err(Error::WouldBlock));
        }
        if name == "receive-cross-hole" {
            let mut prefix = [0; 16];
            memory
                .copy_from_user(
                    &manager.process(actor).unwrap().space,
                    0x600ff0,
                    &mut prefix,
                )
                .unwrap();
            assert_eq!(
                prefix, [0xa5; 16],
                "failed copyout modified its valid prefix"
            );
        }
        drive(manager, memory, actor);
        assert_eq!(
            manager.wait(memory, actor).unwrap(),
            Some(Exit::Code(error.code()))
        );
        cleanup(manager, memory, other);
        assert_eq!(manager.broker.counts(), (0, 0));
        assert_eq!(memory.free_frames(), before);
        use core::fmt::Write;
        let mut serial = crate::arch::Serial::take().unwrap();
        writeln!(
            serial,
            "RUSTIC IPC_REJECT case={name} code={:#x} preserved=1",
            error.code()
        )
        .unwrap();
        serial.flush();
    }
    for mode in [18, 5] {
        let actor = create(manager, memory);
        let other = peer(manager, memory);
        let (handle, remote) = manager.connect(actor, other).unwrap();
        if mode == 5 {
            manager.broker.send(other.0, remote, &packet()).unwrap();
        }
        manager.bootstrap(actor, [mode, handle, 0x7fffeff0]);
        called(manager, memory, actor, 1);
        assert_eq!(
            manager.process(actor).unwrap().frame.call().0,
            if mode == 5 { 32 } else { 0 }
        );
        if mode == 5 {
            let mut output = [0; 32];
            memory
                .copy_from_user(
                    &manager.process(actor).unwrap().space,
                    0x7fffeff0,
                    &mut output,
                )
                .unwrap();
            assert_eq!(
                u64::from_le_bytes(output[16..24].try_into().unwrap()),
                other.0
            );
            assert_eq!(output[24], 0x42);
        } else {
            assert_eq!(manager.broker.peek(other.0, remote).unwrap().length(), 32);
        }
        cleanup(manager, memory, actor);
        cleanup(manager, memory, other);
    }
    let actor = create(manager, memory);
    let other = peer(manager, memory);
    let (handle, remote) = manager.connect(actor, other).unwrap();
    manager.bootstrap(actor, [11, handle, 0]);
    drive(manager, memory, actor);
    assert_eq!(
        manager.wait(memory, actor).unwrap(),
        Some(Exit::Code(Error::WouldBlock.code()))
    );
    for correlation in [123u64, 124] {
        let message = manager.broker.peek(other.0, remote).unwrap().encode();
        assert_eq!(&message[8..16], &correlation.to_le_bytes());
        manager.broker.consume(other.0, remote).unwrap();
    }
    assert_eq!(manager.broker.peek(other.0, remote), Err(Error::Closed));
    cleanup(manager, memory, other);
    cases.len()
}
