// SPDX-License-Identifier: Apache-2.0
use rustic_sdk::{abi::supervisor as s, files::Client};
pub fn run(files: &mut Client, w: [u64; 8]) -> [u64; 8] {
    match w[0] {
        s::SPIN => loop {
            core::hint::spin_loop();
        },
        s::FAULT => {
            // SAFETY: Deliberate native isolation probe; UD2 traps and terminates this process.
            unsafe { core::arch::asm!("ud2", options(noreturn)) }
        }
        s::LOST_ADMISSION => {
            super::recovery::discard_reply(files, w[1] as u32, w[2] as u32, s::LOST_ADMISSION)
        }
        s::LOST_OPERATION => {
            super::recovery::discard_reply(files, w[1] as u32, w[2] as u32, s::LOST_OPERATION)
        }
        s::LOST_REPLY => {
            super::recovery::discard_reply(files, w[1] as u32, w[2] as u32, s::LOST_REPLY)
        }
        s::FINISH => [7, 0, 0, 0, 0, 0, 0, 0],
        s::WATCH => {
            let mut bytes = [0; 1024];
            loop {
                match files.read(w[1] as u32, &mut bytes) {
                    Ok(_) => {
                        rustic_sdk::runtime::clock();
                    }
                    Err(e) => return [e as u64, 0, 0, 0, 0, 0, 0, 0],
                }
            }
        }
        s::READ | s::PROBE => {
            if w[0] == s::PROBE {
                use rustic_sdk::runtime::{self, Error, abi as k};
                if runtime::control([k::SHUTDOWN, 0, 0, 0, 0, 0, 0, 0]) != Err(Error::Denied)
                    || runtime::console_write(b"forbidden") != Err(Error::Denied)
                    || runtime::console_read(&mut [0; 1]) != Err(Error::Denied)
                    || runtime::wait_set(&[0], 1) != Err(Error::Invalid)
                {
                    return [999, 0, 0, 0, 0, 0, 0, 0];
                }
            }
            let mut bytes = [0; 1024];
            let first = files.read(w[1] as u32, &mut bytes);
            let other = if w[0] == s::PROBE {
                files.stat(w[2] as u32).map(|_| 0)
            } else {
                Ok(0)
            };
            [
                first.as_ref().err().map_or(0, |e| *e as u64),
                first.unwrap_or(0) as u64,
                other.err().map_or(0, |e| e as u64),
                0,
                0,
                0,
                0,
                0,
            ]
        }
        _ => [1, 0, 0, 0, 0, 0, 0, 0],
    }
}
