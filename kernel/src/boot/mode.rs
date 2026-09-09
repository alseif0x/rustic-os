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
    MemoryReadOnly,
    MemoryNx,
    MemoryUnmapped,
    MemoryTextAlias,
    MemoryGuard,
    BlockPersist,
    BlockReadOnly,
    BlockError,
    BlockTimeout,
    BlockMissing,
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
            b"mode=memory-ro" => Some(Self::MemoryReadOnly),
            b"mode=memory-nx" => Some(Self::MemoryNx),
            b"mode=memory-unmapped" => Some(Self::MemoryUnmapped),
            b"mode=memory-text-alias" => Some(Self::MemoryTextAlias),
            b"mode=memory-guard" => Some(Self::MemoryGuard),
            b"mode=block-persist" => Some(Self::BlockPersist),
            b"mode=block-readonly" => Some(Self::BlockReadOnly),
            b"mode=block-error" => Some(Self::BlockError),
            b"mode=block-timeout" => Some(Self::BlockTimeout),
            b"mode=block-missing" => Some(Self::BlockMissing),
            _ => None,
        }
    }
}
