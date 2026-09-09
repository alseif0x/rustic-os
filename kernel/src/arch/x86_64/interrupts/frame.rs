// SPDX-License-Identifier: Apache-2.0
//! Assembly/CPU frame shared by exception dispatch and the user transition bridge.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub(crate) struct Frame {
    pub(crate) registers: [u64; 15],
    pub(crate) vector: u64,
    pub(crate) error: u64,
    pub(crate) rip: u64,
    pub(crate) cs: u64,
    pub(crate) flags: u64,
    pub(crate) rsp: u64,
    pub(crate) ss: u64,
}

const _: () = assert!(core::mem::size_of::<Frame>() == 176);
const _: () = assert!(core::mem::offset_of!(Frame, vector) == 120);
const _: () = assert!(core::mem::offset_of!(Frame, rip) == 136);

impl Frame {
    pub(crate) fn user(entry: u64, stack: u64, args: [u64; 3]) -> Self {
        let mut frame = Self {
            rip: entry,
            rsp: stack,
            cs: 0x2b,
            ss: 0x33,
            flags: 0x202,
            ..Self::default()
        };
        frame.registers[8] = args[0]; // RDI
        frame.registers[9] = args[1]; // RSI
        frame.registers[11] = args[2]; // RDX
        frame
    }
    pub(crate) fn result(&mut self, value: u64) {
        self.registers[14] = value;
    }
    pub(crate) fn call(&self) -> (u64, u64) {
        (self.registers[14], self.registers[8])
    }
    pub(crate) fn valid_user(&self) -> bool {
        self.cs == 0x2b
            && self.ss == 0x33
            && (4096..1 << 47).contains(&self.rip)
            && (4096..1 << 47).contains(&self.rsp)
    }
    pub(crate) fn sanitize(&mut self) {
        // Arithmetic flags and DF only; always IF=1, IOPL=0, no NT/VM/AC/TF.
        self.flags = (self.flags & 0xcd5) | 0x202;
    }
}
