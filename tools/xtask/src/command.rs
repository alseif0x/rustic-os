// SPDX-License-Identifier: Apache-2.0
use std::{path::Path, process::Command};

/// Inherits diagnostics and fails on spawn errors, nonzero exit, or signals.
pub(super) fn cargo(root: &Path, args: &[&str]) -> Result<(), String> {
    let executable = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    eprintln!("cargo {}", args.join(" "));
    let status = Command::new(executable)
        .args(args)
        .current_dir(root)
        .status()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo {} failed: {status}", args.join(" ")))
    }
}
