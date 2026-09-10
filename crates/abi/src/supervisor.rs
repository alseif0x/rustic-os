// SPDX-License-Identifier: Apache-2.0
//! Owner-shell service protocol, on an explicitly provisioned private IPC channel.
// Replies: word 0 status (0 success, 1 invalid, 2 denied, 3 busy, 4 service failure),
// words 1..7 operation-specific values. No caller-selected subject identity.
pub const INFO: u64 = 1;
pub const PROCESS: u64 = 2;
pub const RUN: u64 = 3;
pub const KILL: u64 = 4;
pub const REAP: u64 = 5;
pub const PERMISSIONS: u64 = 6;
pub const REVOKE: u64 = 7;
pub const EXIT: u64 = 8;
pub const SERVICES: u64 = 9;
pub const GRANT: u64 = 10;
pub const RESTART: u64 = 11;
pub const SPIN: u64 = 1;
pub const FAULT: u64 = 2;
pub const READ: u64 = 3;
pub const PROBE: u64 = 4;
pub const FINISH: u64 = 5;
pub const WATCH: u64 = 6;
