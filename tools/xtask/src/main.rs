// SPDX-License-Identifier: Apache-2.0
//! Host-only command entry point.
mod checks;
mod command;

fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [action] if action == "check" => checks::run(),
        _ => Err("usage: cargo xtask check".to_owned()),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
