// SPDX-License-Identifier: Apache-2.0
//! R0 integer-only INT 0x80 ABI; see docs/PROCESS-ABI.md before extending it.
pub const VERSION: u64 = 0x0001_0000;
pub const QUERY: u64 = 0;
pub const EXIT: u64 = 1;
pub const REPORT: u64 = 2;
pub const GET_PID: u64 = 3;
pub const NOT_SUPPORTED: u64 = u64::MAX;
pub const QUOTA: u64 = u64::MAX - 1;
