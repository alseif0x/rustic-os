// SPDX-License-Identifier: Apache-2.0
/// Supply a function fn(u64, u64, u64) -> u64; returning exits the process.
/// Arguments are the launcher's three integers, not pointers or implicit grants.
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        core::arch::global_asm!(
            ".section .text._start,\"ax\"",
            ".global _start",
            "_start:",
            "cld",
            "call __rustic_main",
            "ud2",
        );
        // SAFETY: One entry invocation per executable defines this unique symbol.
        #[unsafe(no_mangle)]
        extern "C" fn __rustic_main(a: u64, b: u64, c: u64) -> ! {
            if $crate::process::verify_versions().is_err() {
                $crate::process::exit(126);
            }
            $crate::process::exit($main(a, b, c))
        }
    };
}
