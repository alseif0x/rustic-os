// SPDX-License-Identifier: Apache-2.0
use super::*;

fn blocked(manager: &mut Manager, memory: &mut Memory, pid: Pid) {
    for _ in 0..32 {
        if manager.state(pid).unwrap() == State::Blocked {
            return;
        }
        manager.step(memory).unwrap();
    }
    panic!("WAIT did not block");
}
pub(super) fn verify(manager: &mut Manager, memory: &mut Memory) {
    let waiting = create(manager, memory);
    let closing = create(manager, memory);
    let (a, b) = manager.connect(waiting, closing).unwrap();
    manager.bootstrap(waiting, [2, a, 0]);
    manager.bootstrap(closing, [19, b, 0]);
    blocked(manager, memory, waiting);
    blocked(manager, memory, closing);
    assert_eq!(
        manager.step(memory).unwrap(),
        None,
        "all-blocked returns idle without spinning"
    );
    manager.cancel_wait(closing).unwrap();
    drive(manager, memory, waiting);
    drive(manager, memory, closing);
    assert_eq!(
        manager.wait(memory, waiting).unwrap(),
        Some(Exit::Code(Error::Closed.code()))
    );
    assert_eq!(
        manager.wait(memory, closing).unwrap(),
        Some(Exit::Code(Error::Handle.code()))
    );
    for cancel in [true, false] {
        let waiter = create(manager, memory);
        let peer = manager
            .create(memory, super::super::image(false), [1, 0, 11])
            .unwrap();
        let (handle, _) = manager.connect(waiter, peer).unwrap();
        manager.bootstrap(waiter, [2, handle, 0]);
        blocked(manager, memory, waiter);
        if cancel {
            manager.cancel_wait(waiter).unwrap();
        } else {
            manager.kill(peer).unwrap();
        }
        drive(manager, memory, waiter);
        let error = if cancel {
            Error::Cancelled
        } else {
            Error::Closed
        };
        assert_eq!(
            manager.wait(memory, waiter).unwrap(),
            Some(Exit::Code(error.code()))
        );
        cleanup(manager, memory, peer);
    }
    let owner = create(manager, memory);
    let peer = create(manager, memory);
    let target = create(manager, memory);
    let (old, remote) = manager.connect(owner, peer).unwrap();
    manager.broker.send(peer.0, remote, &packet()).unwrap();
    let moved = manager.transfer(owner, old, target, abi::READ).unwrap();
    assert_ne!(old, moved);
    assert_eq!(
        manager.broker.check(owner.0, old, abi::READ),
        Err(Error::Handle)
    );
    assert_eq!(
        manager.transfer(target, moved, owner, abi::ALL),
        Err(Error::Denied)
    );
    manager.bootstrap(target, [5, moved, 0x600100]);
    // Retire dormant launch fixtures before executing the recipient.
    manager.kill(owner).unwrap();
    manager.kill(peer).unwrap();
    drive(manager, memory, target);
    assert_eq!(manager.wait(memory, target).unwrap(), Some(Exit::Code(32)));
    cleanup(manager, memory, owner);
    cleanup(manager, memory, peer);
}
