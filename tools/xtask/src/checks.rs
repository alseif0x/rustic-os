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
    ];
    for args in commands {
        command::cargo(root, args)?;
    }
    let application = std::process::Command::new("python3")
        .args(["tools/application.py"])
        .current_dir(root)
        .status()
        .map_err(|error| error.to_string())?;
    if !application.success() {
        return Err("native application build failed".to_owned());
    }
    command::cargo(
        root,
        &[
            "clippy",
            "-p",
            "rustic-sdk-probe",
            "-p",
            "rustic-block-probe",
            "--features",
            "native",
            "--target",
            "x86_64-unknown-none",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let native = root
        .join(target)
        .join("native")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    command::cargo_env(
        root,
        &[
            "clippy",
            "-p",
            "rustic-kernel",
            "--bin",
            "rustic-os",
            "--features",
            "sdk-test",
            "--target",
            "x86_64-unknown-none",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
        &[("RUSTIC_APPLICATION_DIRECTORY", &native)],
    )?;
    Ok(())
}
