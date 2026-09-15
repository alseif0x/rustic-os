// SPDX-License-Identifier: Apache-2.0
use rustic_abi::{block, ipc, memory::*, process, runtime};

#[test]
fn heap_numbers_extend_the_used_range_without_colliding() {
    assert_eq!([MAP, UNMAP, QUERY], [21, 22, 23]);
    let used = [
        process::QUERY,
        process::EXIT,
        process::REPORT,
        process::GET_PID,
        ipc::SEND,
        ipc::RECEIVE,
        ipc::WAIT,
        ipc::CLOSE,
        ipc::INFO,
        block::INFO,
        block::SUBMIT,
        block::RESULT,
        block::WAIT,
        block::CANCEL,
        block::CLOSE,
        runtime::CLOCK,
        runtime::CONSOLE_WRITE,
        runtime::CONSOLE_READ,
        runtime::CONSOLE_WAIT,
        runtime::WAIT_SET,
        runtime::CONTROL,
    ];
    for number in used {
        assert!(!(MAP..=QUERY).contains(&number), "reused number {number}");
    }
    // 21 is the first free number; the control opcode 10 is a different namespace.
    assert_eq!(used.iter().copied().max(), Some(MAP - 1));
    assert_eq!(VERSION, 1);
}

#[test]
fn only_documented_flags_and_selectors_are_accepted() {
    assert!(flags_valid(READ));
    assert!(flags_valid(WRITE));
    assert_eq!(READ, 0);
    assert_eq!(WRITE, 1);
    for bit in 1..64 {
        assert!(!flags_valid(1 << bit), "accepted flag bit {bit}");
        assert!(!flags_valid(WRITE | (1 << bit)), "accepted flag bit {bit}");
    }
    for selector in [MAPPED_PAGES, PAGE_LIMIT, WINDOW_BASE, WINDOW_PAGES] {
        assert!(selector_valid(selector));
    }
    for selector in [4, 5, 64, u64::MAX] {
        assert!(!selector_valid(selector), "accepted selector {selector}");
    }
    assert_eq!(
        [MAPPED_PAGES, PAGE_LIMIT, WINDOW_BASE, WINDOW_PAGES],
        [0, 1, 2, 3]
    );
}

#[test]
fn heap_failures_reuse_the_native_error_encoding() {
    for error in [
        runtime::Error::Size,
        runtime::Error::Address,
        runtime::Error::Invalid,
        runtime::Error::Full,
    ] {
        assert_eq!(runtime::Error::decode(error.code()), Err(error));
        // A base address is never confused with an error code.
        assert!(error.code() > u64::MAX - 4096);
    }
    assert_eq!(runtime::Error::decode(0x1000_0000), Ok(0x1000_0000));
}
