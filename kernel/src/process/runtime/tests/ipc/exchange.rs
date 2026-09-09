// SPDX-License-Identifier: Apache-2.0
use super::*;

pub(super) fn verify(manager: &mut Manager, memory: &mut Memory) {
    let before = memory.free_frames();
    for _ in 0..16 {
        let receiver = create(manager, memory);
        let sender = create(manager, memory);
        let (rx, tx) = manager.connect(receiver, sender).unwrap();
        manager.bootstrap(receiver, [1, rx, sender.0]);
        manager.bootstrap(sender, [0, tx, receiver.0]);
        drive(manager, memory, sender);
        drive(manager, memory, receiver);
        assert_eq!(manager.wait(memory, sender).unwrap(), Some(Exit::Code(42)));
        assert_eq!(
            manager.wait(memory, receiver).unwrap(),
            Some(Exit::Code(43))
        );
        assert_eq!(manager.broker.counts(), (0, 0));
        assert_eq!(memory.free_frames(), before);
    }
    let info = create(manager, memory);
    manager.bootstrap(info, [14, 0, 0]);
    drive(manager, memory, info);
    assert_eq!(manager.wait(memory, info).unwrap(), Some(Exit::Code(1)));
}
