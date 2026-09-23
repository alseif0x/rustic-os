// SPDX-License-Identifier: Apache-2.0
use rustic_abi::runtime::*;
#[test]
fn control_shapes_reject_unknown_operations_and_reserved_words() {
    for (op, end) in [
        (INFO, 1),
        (SPAWN, 2),
        (START, 5),
        (CONNECT, 3),
        (CLOSE_ENDPOINT, 3),
        (MOVE_ENDPOINT, 5),
        (BLOCK_GRANT, 5),
        (CONSOLE_GRANT, 2),
        (PROCESS, 2),
        (KILL, 2),
        (REAP, 2),
        (SHUTDOWN, 1),
        (DEVICE, 1),
        (STAGE_BEGIN, 4),
        (STAGE_COPY, 5),
        (STAGE_COMMIT, 2),
        (STAGE_ABORT, 2),
    ] {
        let mut w = [0; 8];
        w[0] = op;
        assert_eq!(validate_control(w), Ok(()));
        for i in end..8 {
            w[i] = 1;
            assert_eq!(validate_control(w), Err(Error::Invalid));
            w[i] = 0;
        }
        assert_eq!(decode(&encode(w)), Ok(w));
    }
    assert_eq!(
        validate_control([10, 0, 0, 0, 0, 0, 0, 0]),
        Err(Error::Invalid)
    );
    assert_eq!(
        validate_control([
            STAGE_BEGIN,
            512,
            0x20_0000,
            rustic_abi::application::KNOWN,
            0,
            0,
            0,
            0
        ]),
        Ok(())
    );
    assert_eq!(decode(&[0; 63]), Err(Error::Size));
    for e in [
        Error::Denied,
        Error::Address,
        Error::WouldBlock,
        Error::Protocol,
    ] {
        assert_eq!(Error::decode(e.code()), Err(e));
    }
}
