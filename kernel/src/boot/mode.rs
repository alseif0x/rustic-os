// SPDX-License-Identifier: Apache-2.0
/// Explicit fixture selection for the experimental boot image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootMode {
    Ok,
    Panic,
    Hang,
    Exception,
    GeneralProtection,
    DoubleFault,
    TimerStall,
}

impl BootMode {
    /// Reject all unsupported input instead of silently selecting success.
    pub fn parse(input: &[u8]) -> Option<Self> {
        match input {
            b"mode=ok" => Some(Self::Ok),
            b"mode=panic" => Some(Self::Panic),
            b"mode=hang" => Some(Self::Hang),
            b"mode=exception" => Some(Self::Exception),
            b"mode=gp" => Some(Self::GeneralProtection),
            b"mode=doublefault" => Some(Self::DoubleFault),
            b"mode=timer-stall" => Some(Self::TimerStall),
            _ => None,
        }
    }
}
