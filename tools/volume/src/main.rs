// SPDX-License-Identifier: Apache-2.0
//! Host tool for v6 volume images: provision, seed a v5 experiment volume,
//! migrate it in place and report what an image contains. It never touches the
//! owner's terminal volume and never guesses a lineage.
use std::path::Path;
use std::process::ExitCode;

mod command;
mod disk;

const USAGE: &str = "\
usage: rustic-volume <command>
  provision <image> <lineage>   write a fresh v6 volume image
  seed <image>                  write a small v5 experiment volume image
  write <image> <parent> <name> <source>   place a host file in a v6 image
  migrate <image> <lineage>     migrate a v5 image to v6 in place
  report <image>                print a v6 image as JSON
A lineage is 32 hex characters. Images are exactly one volume long; a shorter
file is refused so a truncated image cannot be read as a volume.";

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
        [command, image, lineage] if command == "provision" => {
            command::provision(Path::new(image), lineage)
        }
        [command, image, lineage] if command == "migrate" => {
            command::migrate(Path::new(image), lineage)
        }
        [command, image, parent, name, source] if command == "write" => {
            command::write(Path::new(image), parent, name, Path::new(source))
        }
        [command, image] if command == "seed" => command::seed(Path::new(image)),
        [command, image] if command == "report" => command::report(Path::new(image)),
        _ => Err(USAGE.to_owned()),
    }
}
