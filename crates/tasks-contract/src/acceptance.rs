// SPDX-License-Identifier: Apache-2.0
//! Test-only owner controls for the native tasks lifecycle acceptance.
//!
//! These words are deliberately outside the production supervisor protocol and
//! are compiled only for the explicit guest acceptance build.  The supervisor
//! remains the authority for the controls; this crate only owns their bounded
//! wire shape so the shell fixture and supervisor cannot drift apart.

/// Arm the one-shot hold before the next native tasks grant is delivered.
pub const ARM: u64 = 240;
/// Return the current hold, admin-drain and expiry witnesses.
pub const STATUS: u64 = 241;
/// Release the currently held grant and disarm the fixture.
pub const RELEASE: u64 = 242;
/// Clear all fixture state without touching production task state.
pub const RESET: u64 = 243;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Arm,
    Status,
    Release,
    Reset,
}

/// Decode one owner control request, requiring canonical zero padding.
pub fn decode_request(words: [u64; 8]) -> Option<Request> {
    if words[1..].iter().any(|word| *word != 0) {
        return None;
    }
    match words[0] {
        ARM => Some(Request::Arm),
        STATUS => Some(Request::Status),
        RELEASE => Some(Request::Release),
        RESET => Some(Request::Reset),
        _ => None,
    }
}

/// Encode one owner control request with canonical zero padding.
pub const fn request(request: Request) -> [u64; 8] {
    [
        match request {
            Request::Arm => ARM,
            Request::Status => STATUS,
            Request::Release => RELEASE,
            Request::Reset => RESET,
        },
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ]
}

/// Encode the bounded fixture status.
///
/// The words are `status, held, armed, slot, pid, admin_pending, drained,
/// expiry_tick`.  Slot zero means no held draft; otherwise it is the actual
/// supervisor child slot plus one.  `drained` increments only when the real
/// supervisor admin RPC consumes its reply, and `expiry_tick` is written only
/// when the real cached task result is expired and disposed.
pub const fn status(
    held: u64,
    armed: u64,
    slot: u64,
    pid: u64,
    admin_pending: u64,
    drained: u64,
    expiry_tick: u64,
) -> [u64; 8] {
    [
        0,
        held,
        armed,
        slot,
        pid,
        admin_pending,
        drained,
        expiry_tick,
    ]
}

/// Decode a canonical status response from the owner supervisor.
pub fn decode_status(words: [u64; 8]) -> Option<[u64; 7]> {
    if words[0] != 0
        || words[1] > 1
        || words[2] > 1
        || words[3] > 2
        || words[5] > 1
        || (words[1] == 0 && (words[3] != 0 || words[4] != 0))
        || (words[1] == 1 && (words[3] == 0 || words[4] == 0))
    {
        return None;
    }
    Some([
        words[1], words[2], words[3], words[4], words[5], words[6], words[7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_require_exact_zero_padding() {
        for (words, expected) in [
            (request(Request::Arm), Request::Arm),
            (request(Request::Status), Request::Status),
            (request(Request::Release), Request::Release),
            (request(Request::Reset), Request::Reset),
        ] {
            assert_eq!(decode_request(words), Some(expected));
            let mut bad = words;
            bad[7] = 1;
            assert_eq!(decode_request(bad), None);
        }
        assert_eq!(decode_request([239, 0, 0, 0, 0, 0, 0, 0]), None);
    }

    #[test]
    fn status_codec_preserves_witnesses() {
        let words = status(1, 1, 1, 33, 1, 2, 77);
        assert_eq!(decode_status(words), Some([1, 1, 1, 33, 1, 2, 77]));
        let mut bad = words;
        bad[0] = 1;
        assert_eq!(decode_status(bad), None);
    }
}
