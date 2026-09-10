// SPDX-License-Identifier: Apache-2.0
use rustic_shell::{
    editor::{CAPACITY, Editor, Event},
    parser::{Error, parse},
};
#[test]
fn quoting_and_escaping_preserve_arguments_or_fail_before_execution() {
    let mut bytes = br#"write a "hello world" '' 'a\b' x\ y"#.to_vec();
    let a = parse(&mut bytes).unwrap();
    assert_eq!(
        (0..a.len()).map(|n| a.get(n).unwrap()).collect::<Vec<_>>(),
        ["write", "a", "hello world", "", "a\\b", "x y"]
    );
    for (mut b, error) in [
        (b"write a 'unfinished".to_vec(), Error::Quote),
        (b"echo x\\".to_vec(), Error::Escape),
    ] {
        assert_eq!(parse(&mut b).err(), Some(error));
    }
    assert_eq!(parse(&mut b"a ".repeat(17)).err(), Some(Error::Arguments));
}
#[test]
fn overflow_crlf_cancel_and_escape_sequences_cannot_execute_a_prefix() {
    let mut e = Editor::new();
    for _ in 0..CAPACITY + 1 {
        e.push(b'x');
    }
    e.push(127);
    assert_eq!(e.push(b'\r'), Event::Overflow);
    e.reset();
    assert_eq!(e.push(b'\n'), Event::None);
    assert!(e.line().is_empty());
    for b in b"hello" {
        e.push(*b);
    }
    assert_eq!(e.push(21), Event::Cleared(5));
    for b in b"bad" {
        e.push(*b);
    }
    assert_eq!(e.push(3), Event::Cancelled);
    for b in b"\x1b[Aecho" {
        e.push(*b);
    }
    assert_eq!(e.line(), b"echo");
    assert_eq!(e.push(b'\n'), Event::Line);
    e.reset();
    e.push(0xc3);
    e.push(0xa9);
    assert_eq!(e.push(b'\n'), Event::Invalid);
}
