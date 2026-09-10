// SPDX-License-Identifier: Apache-2.0
#[derive(Default)]
pub(in super::super) struct Session {
    pub supervisor: u64,
    pub console: u64,
    pub shutdown: bool,
}
