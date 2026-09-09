// SPDX-License-Identifier: Apache-2.0
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod endpoint;
mod message;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use endpoint::Endpoint;
pub use message::Message;
