// SPDX-License-Identifier: Apache-2.0
use std::path::Path;

use crate::command;

pub(super) fn run() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or("cannot locate workspace root")?;
    let commands: &[&[&str]] = &[
        &["fmt", "--all", "--", "--check"],
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
        &["test", "--workspace", "--locked"],
        &[
            "build",
            "-p",
            "rustic-kernel",
            "--target",
            "x86_64-unknown-none",
            "--locked",
        ],
        &[
            "clippy",
            "-p",
            "rustic-kernel",
            "--target",
            "x86_64-unknown-none",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    ];
    for args in commands {
        command::cargo(root, args)?;
    }
    command::cargo(
        root,
        &[
            "clippy",
            "-p",
            "rustic-kernel",
            "--bin",
            "rustic-os",
            "--features",
            "boot-image",
            "--target",
            "x86_64-unknown-none",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    Ok(())
}
