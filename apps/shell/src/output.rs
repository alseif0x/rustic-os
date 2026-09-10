// SPDX-License-Identifier: Apache-2.0
use core::fmt::Write;
pub fn text(s: &str) {
    let _ = rustic_sdk::runtime::console_write(s.as_bytes());
}
pub fn format(args: core::fmt::Arguments<'_>) {
    let _ = rustic_sdk::runtime::Console.write_fmt(args);
}
pub fn bytes(bytes: &[u8]) {
    // File data is untrusted terminal content: escape controls, including ANSI sequences.
    for &b in bytes {
        match b {
            b'\n' => text("\r\n"),
            32..=126 => {
                let _ = rustic_sdk::runtime::console_write(&[b]);
            }
            _ => format(format_args!("\\x{b:02x}")),
        }
    }
}
