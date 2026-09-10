// SPDX-License-Identifier: Apache-2.0
//! Bounded R0 block device with split submission/completion. Transport, DMA queue, requests and fixtures are separate.
mod completion;
mod device;
mod queue;
mod request;
mod tests;
pub(crate) use device::Device;
pub(crate) use tests::verify;
