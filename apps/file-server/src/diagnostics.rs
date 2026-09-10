// SPDX-License-Identifier: Apache-2.0
//! Explicit owner-only failure fixture on the authenticated private admin channel.
use rustic_sdk::{
    abi::runtime as k,
    ipc::{Endpoint, Message},
    runtime,
};
pub(super) fn stall(admin: &Endpoint, correlation: u64, w: [u64; 8]) -> bool {
    if w[0] != 39 {
        return false;
    }
    let valid = w[1] <= 1000 && w[2..].iter().all(|v| *v == 0);
    let reply = [u64::from(!valid), 0, 0, 0, 0, 0, 0, 0];
    if admin
        .send(&Message::new(correlation, &k::encode(reply)).unwrap())
        .is_err()
    {
        return true;
    }
    if valid {
        let deadline = runtime::clock().saturating_add(w[1]);
        // Real service stops reading all channels; the timer still preempts it.
        // Zero is deliberately indefinite, recoverable only by owner restart/exit.
        while w[1] == 0 || runtime::clock() < deadline {
            core::hint::spin_loop();
        }
    }
    true
}
