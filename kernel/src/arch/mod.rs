// SPDX-License-Identifier: Apache-2.0
#[cfg(target_arch = "x86_64")]
mod x86_64;

#[cfg(target_arch = "x86_64")]
pub(crate) use x86_64::{Serial, halt, interrupts, memory, test_exit};

#[cfg(not(target_arch = "x86_64"))]
compile_error!("The boot binary currently supports x86_64 only.");
