// SPDX-License-Identifier: Apache-2.0
use super::output;
use rustic_sdk::{files::Client, rpc::Rpc, runtime};
use rustic_shell::{
    editor::{Editor, Event},
    parser,
};
pub struct Session {
    pub files: Client<super::progress::Console>,
    pub supervisor: Rpc,
    pub cwd: u32,
    pub status: u64,
    pub path: [u8; 256],
    pub path_length: usize,
    pub(super) binding_job: u64,
}
impl Session {
    fn prompt(&self) {
        output::text("rustic:");
        output::bytes(&self.path[..self.path_length]);
        output::text("> ");
    }
}
pub fn run(files: u64, control: u64, generation: u32) -> u64 {
    let mut state = Session {
        files: Client::with_progress(files, 0, generation, super::progress::Console::default()),
        supervisor: Rpc::new(control, 0),
        cwd: 4,
        status: 0,
        path: [0; 256],
        path_length: 11,
        binding_job: 0,
    };
    state.path[..11].copy_from_slice(b"/workspaces");
    output::text(
        "\r\nRusticOS native terminal 0.1\r\nRust user processes | persistent files | explicit permissions\r\nType help for commands. Ctrl-C cancels a line; exit stops the VM.\r\n",
    );
    if files == 0 {
        output::text("Starting files; Ctrl-C keeps owner control available.\r\n");
        if let Err(e) = state.wait_job(generation as u64) {
            output::format(format_args!("error: {e}\r\n"));
        }
    }
    let mut editor = Editor::new();
    state.prompt();
    loop {
        let mut byte = [0; 1];
        match state.files.progress().read(&mut byte) {
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
                if core::mem::take(&mut state.files.progress().overflow) {
                    output::text("error: typeahead overflow; complete input line discarded\r\n");
                }
                editor.reset();
                state.prompt();
            }
        }
    }
}
