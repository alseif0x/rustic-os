// SPDX-License-Identifier: Apache-2.0
//! Two utility slots, explicit scopes and fresh generations, no inherited shell authority.
use super::services::*;
use rustic_sdk::{abi::supervisor as s, runtime::abi as k};
/// Roles whose control endpoint carries owner-stepped actor replies. Those
/// replies belong to `actor_state`; the idle-endpoint branch must not absorb
/// them. Any other role has no owner-stepped action at all.
pub fn actor_role(role: u64) -> bool {
    matches!(
        role,
        s::SESSION
            | s::HELPER
            | s::ADMISSION_SESSION
            | s::PRIVATE_ADMISSION_SESSION
            | s::TASKS_OWNER
    )
}
pub struct Child {
    pub pid: u64,
    pub control: rustic_sdk::rpc::Rpc,
    pub actor_state: u64,
    pub actor_deadline: u64,
    pub scope: u32,
    pub rights: u8,
    pub expires: u64,
    pub generation: u32,
    pub root: u32,
    pub role: u64,
    pub file_token: u64,
    pub report: [u64; 7],
    pub closed: bool,
}
impl State {
    pub fn collect(&mut self) {
        let mut ended_roots = [0; 2];
        let mut ended = 0;
        for child in self.children.iter_mut().flatten() {
            if child.closed {
                continue;
            }
            if actor_role(child.role) && child.control.pending() {
                match child.control.poll() {
                    Ok(Some(message)) => {
                        if let Ok(w) = k::decode(message.payload()) {
                            child.report.copy_from_slice(&w[..7]);
                            child.actor_state = 2;
                        } else {
                            child.actor_state = 3;
                        }
                    }
                    Ok(None) => {}
                    Err(_) => {
                        child.actor_state = 3;
                    }
                }
                continue;
            }
            // Task rows are correlated with the finite relay state machine;
            // generic child collection must never consume its responses.
            if child.role == s::TASKS {
                continue;
            }
            // Idle native actors must not send unsolicited control messages. Closed
            // endpoints also reveal root death after a file endpoint has moved.
            match child.control.endpoint.receive() {
                Ok(message) => {
                    if message.sender() == child.pid
                        && let Ok(w) = k::decode(message.payload())
                    {
                        child.report.copy_from_slice(&w[..7]);
                    }
                }
                Err(rustic_sdk::Error::Ipc(rustic_sdk::abi::ipc::Error::WouldBlock)) => {}
                Err(_) => {
                    child.closed = true;
                    if child.role == s::SESSION && child.rights != 0 {
                        ended_roots[ended] = child.pid;
                        ended += 1;
                    }
                }
            }
        }
        // The independent control endpoint detects root death even after a file handle moves.
        for pid in &ended_roots[..ended] {
            let _ = self.revoke_session(*pid);
        }
    }
    pub fn reap(&mut self, pid: u64) -> Result<[u64; 8], u64> {
        let slot = self
            .children
            .iter()
            .position(|c| c.as_ref().is_some_and(|c| c.pid == pid))
            .ok_or(2u64)?;
        let r = call([k::REAP, pid, 0, 0, 0, 0, 0, 0]).map_err(|_| 3u64)?;
        if self.children[slot]
            .as_ref()
            .is_some_and(|c| c.role == s::SESSION && c.rights != 0)
        {
            // A pending actor reply can delay closed-endpoint observation by one pass.
            // Keep root identity until its fence has been scheduled, even after a move.
            self.revoke_session(pid)?;
        }
        let child = self.children[slot].take().unwrap();
        let _ = child.control.endpoint.close();
        // Dead client endpoints are also collected by the file server. While admin
        // is unavailable, retain that cleanup there without blocking owner control.

        Ok([0, r[0], r[1], 0, 0, 0, 0, 0])
    }
}
