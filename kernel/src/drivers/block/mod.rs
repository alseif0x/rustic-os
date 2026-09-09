// SPDX-License-Identifier: Apache-2.0
//! Synchronous R0 block device. Transport, DMA queue, requests and fixtures are separate.
mod device;
mod queue;
mod request;
mod tests;
pub(crate) use device::Device;
pub(crate) use tests::verify;
