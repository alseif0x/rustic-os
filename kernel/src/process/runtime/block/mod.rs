// SPDX-License-Identifier: Apache-2.0
//! Device lifetime/dispatch; admission and completion ownership are pure library code.
mod service;
pub(super) use service::Service;
