// SPDX-License-Identifier: Apache-2.0
use super::output;
use rustic_sdk::{files::Client, rpc::Rpc, runtime};
use rustic_shell::{
    editor::{Editor, Event},
    parser,
};
pub struct Session {
    pub files: Client,
    pub supervisor: Rpc,
    pub cwd: u32,
    pub status: u64,
}
impl Session {
    pub fn service(&mut self, w: [u64; 8]) -> Result<[u64; 8], super::commands::Error> {
        let r = self
            .supervisor
            .words(w)
            .map_err(|_| super::commands::Error::Service(4))?;
        if r[0] != 0 {
            Err(super::commands::Error::Service(r[0]))
        } else {
            Ok(r)
        }
    }
    fn prompt(&mut self) {
        let mut bytes = [0; 256];
        let path = self.files.path(self.cwd, &mut bytes);
        output::text("rustic:");
        match path {
            Ok(n) => output::bytes(&bytes[..n]),
            Err(_) => output::text("?"),
        }
        output::text("> ");
    }
}
pub fn run(files: u64, control: u64, generation: u32) -> u64 {
    let mut state = Session {
        files: Client::new(files, 0, generation),
        supervisor: Rpc::new(control, 0),
        cwd: 4,
        status: 0,
    };
    output::text(
        "\r\nRusticOS native terminal 0.1\r\nRust user processes | persistent files | explicit permissions\r\nType help for commands. Ctrl-C cancels a line; exit stops the VM.\r\n",
    );
    let mut editor = Editor::new();
    state.prompt();
    loop {
        let mut byte = [0; 1];
        match runtime::console_read(&mut byte) {
            Ok(1) => {}
            Err(runtime::Error::WouldBlock) => {
                if runtime::console_wait().is_err() {
                    return 1;
                }
                continue;
            }
            _ => return 1,
        }
        match editor.push(byte[0]) {
            Event::None => {}
            Event::Inserted(b) => {
                let _ = runtime::console_write(&[b]);
            }
            Event::Erased => output::text("\x08 \x08"),
            Event::Cleared(n) => {
                for _ in 0..n {
                    output::text("\x08 \x08");
                }
            }
            Event::Cancelled => {
                output::text("^C\r\n");
                state.status = 130;
                state.prompt();
            }
            Event::Invalid => {
                output::text("\r\nerror: unsupported input; ASCII command discarded\r\n");
                state.status = 2;
                editor.reset();
                state.prompt();
            }
            Event::Overflow => {
                output::text("\r\nerror: line exceeds 1024 bytes; command discarded\r\n");
                state.status = 2;
                editor.reset();
                state.prompt();
            }
            Event::Line => {
                output::text("\r\n");
                match parser::parse(editor.line()) {
                    Ok(args) => {
                        if !args.is_empty() {
                            match super::commands::execute(&mut state, &args) {
                                Ok(exit) => {
                                    state.status = 0;
                                    if exit {
                                        return 0;
                                    }
                                }
                                Err(e) => {
                                    state.status = 1;
                                    output::format(format_args!("error: {e}\r\n"));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        state.status = 2;
                        output::format(format_args!("error: {e:?}\r\n"));
                    }
                }
                editor.reset();
                state.prompt();
            }
        }
    }
}
