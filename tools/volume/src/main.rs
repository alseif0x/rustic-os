// SPDX-License-Identifier: Apache-2.0
//! Host tool for explicit disposable v7 fixtures: creating, reporting, adding
//! host files to and maintaining an existing disposable v7 image. It never
//! touches the owner's terminal volume and never guesses a lineage.
use std::path::Path;
use std::process::ExitCode;

mod add7;
mod command;
mod disk;
mod maintain7;
#[cfg(test)]
mod testing;

const USAGE: &str = "\
usage: rustic-volume <command>
  seed7 <image> <lineage> <elf> <manifest> [--scratch]
                                create a fresh v7 application fixture; --scratch
                                also creates an empty writable scratch.bin
  report7 <image>               verify a v7 image and print its metadata as JSON
  add7 <v7-image> <workspace> <name> <file>
                                add a new file to an existing v7 image with one
                                tracked commit; <workspace> is a node id, ws_
                                text or /workspaces/... path
  maintain7 <v7-image>          advance the retry epoch of an existing v7 image,
                                dropping its terminal retained records (the host
                                form of the owner's retention maintenance)
A lineage is 32 hex characters. Images are exactly one volume long; a shorter
file is refused so a truncated image cannot be read as a volume. `seed7`
uses an exclusive create and refuses an existing path. `add7` refuses an
existing name and a full retention table before it writes, and never evicts a
record; `maintain7` refuses with `Busy` while an admission is unresolved.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<String, String> {
    match args {
        [command, image, lineage, elf, manifest] if command == "seed7" => command::seed7(
            Path::new(image),
            lineage,
            Path::new(elf),
            Path::new(manifest),
            false,
        ),
        [command, image, lineage, elf, manifest, flag]
            if command == "seed7" && flag == "--scratch" =>
        {
            command::seed7(
                Path::new(image),
                lineage,
                Path::new(elf),
                Path::new(manifest),
                true,
            )
        }
        [command, image] if command == "report7" => command::report7(Path::new(image)),
        [command, image, workspace, name, source] if command == "add7" => {
            add7::add7(Path::new(image), workspace, name, Path::new(source))
        }
        [command, image] if command == "maintain7" => maintain7::maintain7(Path::new(image)),
        _ => Err(USAGE.to_owned()),
    }
}
