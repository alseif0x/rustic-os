// SPDX-License-Identifier: Apache-2.0
pub const PAGE_SIZE: u64 = 4096;

pub const fn canonical(address: u64) -> bool {
    address <= 0x0000_7fff_ffff_ffff || address >= 0xffff_8000_0000_0000
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualPage(u64);

impl VirtualPage {
    /// Runtime allocations are restricted to the non-null lower canonical half.
    pub const fn new(address: u64) -> Option<Self> {
        if address >= PAGE_SIZE && address < (1 << 47) && address.is_multiple_of(PAGE_SIZE) {
            Some(Self(address))
        } else {
            None
        }
    }

    pub const fn address(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PagePermissions {
    pub writable: bool,
    pub executable: bool,
    pub user: bool,
}

impl PagePermissions {
    pub const READ_ONLY: Self = Self {
        writable: false,
        executable: false,
        user: false,
    };
    pub const READ_WRITE: Self = Self {
        writable: true,
        executable: false,
        user: false,
    };
    pub const CODE: Self = Self {
        writable: false,
        executable: true,
        user: false,
    };

    pub const fn valid(self) -> bool {
        !(self.writable && self.executable)
    }
}
