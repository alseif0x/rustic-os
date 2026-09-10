// SPDX-License-Identifier: Apache-2.0
//! Console-owned progress while a foreground RPC waits. No new thread or shared global.
use rustic_sdk::{Error, rpc::Progress, runtime};
use rustic_shell::input::{Event, Queue};
#[derive(Default)]
pub struct Console {
    input: Queue,
    pub overflow: bool,
}
impl Console {
    pub fn read(&mut self, byte: &mut [u8; 1]) -> Result<usize, runtime::Error> {
        if let Some(b) = self.input.pop() {
            byte[0] = b;
            Ok(1)
        } else {
            let n = runtime::console_read(byte)?;
            if n != 0 && self.input.discarding() {
                self.input.push(byte[0]);
                Err(runtime::Error::WouldBlock)
            } else {
                Ok(n)
            }
        }
    }
}
impl Progress for Console {
    fn wait(&mut self, endpoint: u64) -> Result<(), Error> {
        let mut bytes = [0; 16];
        let mut interrupted = false;
        match runtime::console_read(&mut bytes) {
            Ok(n) => {
                for b in &bytes[..n] {
                    match self.input.push(*b) {
                        Event::Interrupted => interrupted = true,
                        Event::Overflow => {
                            self.overflow = true;
                            interrupted = true;
                        }
                        Event::Buffered => {}
                    }
                }
            }
            Err(runtime::Error::WouldBlock) => {}
            _ => return Err(Error::Protocol),
        }
        if interrupted {
            return Err(Error::Interrupted);
        }
        if endpoint == 0 {
            runtime::clock();
        } else {
            runtime::wait_set(&[endpoint], 1).map_err(|_| Error::Protocol)?;
        }
        Ok(())
    }
}
