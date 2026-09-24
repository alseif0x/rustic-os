// SPDX-License-Identifier: Apache-2.0
//! What the V7 service answers while one admission publication is in flight.
//!
//! The publication borrows the volume, so the owner can only revoke or detach
//! client slots meanwhile (the v5 owner-control rule). A revocation takes
//! effect at once, but its acknowledgement waits until the publication has
//! settled, so the owner never re-grants a slot under an unsettled effect.
//! Grants, retention maintenance and anything else that needs the whole
//! service are refused with `Busy`, and so is every client request other than
//! the one being published: nothing is consumed or replaced.
use rustic_sdk::abi::files::{Error, Packet, REVOKE};

/// Administrative detach request of the V7 admin channel.
const DETACH: u64 = 35;
/// Administrative readiness probe of the V7 admin channel.
const PROBE: u64 = 34;

/// The owner's request during a publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admin7 {
    /// Revoke the slot and close its endpoint now; acknowledge with all-zero
    /// words once the publication has settled.
    Revoke(usize),
    /// Close the slot's endpoint and forget it now, and acknowledge.
    Detach(usize),
    /// Readiness probe: acknowledge.
    Probe,
    /// Refuse now; nothing changed.
    Refuse(Error),
}

/// Classify administrative `words` for a service with `clients` slots. The
/// shapes are those of the idle service: a malformed revocation is `Invalid`
/// and a malformed probe or detach `Protocol`.
pub fn admin(words: [u64; 8], clients: usize) -> Admin7 {
    let slot = usize::try_from(words[1])
        .ok()
        .filter(|slot| *slot < clients);
    let rest_zero = words[2..].iter().all(|word| *word == 0);
    match words[0] {
        command if command == u64::from(REVOKE) => match slot {
            Some(slot) if rest_zero => Admin7::Revoke(slot),
            _ => Admin7::Refuse(Error::Invalid),
        },
        DETACH => match slot {
            Some(slot) if rest_zero => Admin7::Detach(slot),
            _ => Admin7::Refuse(Error::Protocol),
        },
        PROBE if words[1] == 0 && rest_zero => Admin7::Probe,
        PROBE => Admin7::Refuse(Error::Protocol),
        _ => Admin7::Refuse(Error::Busy),
    }
}

/// The reply to another client's request: `Busy` for a well-formed packet,
/// `Protocol` for one that does not decode (as the idle service answers it).
pub fn busy(request: Option<&Packet>) -> Packet {
    match request {
        Some(request) => {
            let mut reply = Packet::new(request.op);
            reply.context = request.context;
            reply.status = Error::Busy as u8;
            reply
        }
        None => {
            let mut reply = Packet::new(1);
            reply.status = Error::Protocol as u8;
            reply
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustic_sdk::abi::files::{GRANT, MAINTAIN_RETENTION, REPLACE_OPEN, admission};

    #[test]
    fn only_revocation_detach_and_the_probe_are_served() {
        assert_eq!(admin([33, 0, 0, 0, 0, 0, 0, 0], 4), Admin7::Revoke(0));
        assert_eq!(admin([33, 3, 0, 0, 0, 0, 0, 0], 4), Admin7::Revoke(3));
        assert_eq!(admin([35, 2, 0, 0, 0, 0, 0, 0], 4), Admin7::Detach(2));
        assert_eq!(admin([34, 0, 0, 0, 0, 0, 0, 0], 4), Admin7::Probe);
    }

    #[test]
    fn grants_and_maintenance_wait_for_settlement_with_busy() {
        let grant = [u64::from(GRANT), 0, 3, 17, 0, 15, 0, 2];
        let maintain = [u64::from(MAINTAIN_RETENTION), 0, 0, 0, 0, 0, 0, 0];
        for words in [grant, maintain, [39, 10, 0, 0, 0, 0, 0, 0], [0; 8]] {
            assert_eq!(admin(words, 4), Admin7::Refuse(Error::Busy), "{words:?}");
        }
    }

    #[test]
    fn malformed_owner_requests_keep_their_idle_refusals() {
        for words in [
            [33, 4, 0, 0, 0, 0, 0, 0],
            [33, 0, 1, 0, 0, 0, 0, 0],
            [33, u64::MAX, 0, 0, 0, 0, 0, 0],
        ] {
            assert_eq!(admin(words, 4), Admin7::Refuse(Error::Invalid), "{words:?}");
        }
        for words in [
            [35, 4, 0, 0, 0, 0, 0, 0],
            [35, 0, 0, 0, 0, 0, 0, 9],
            [34, 1, 0, 0, 0, 0, 0, 0],
        ] {
            assert_eq!(
                admin(words, 4),
                Admin7::Refuse(Error::Protocol),
                "{words:?}"
            );
        }
    }

    #[test]
    fn other_clients_are_busy_without_consuming_their_request() {
        for op in [REPLACE_OPEN, admission::EXECUTE, admission::GET] {
            let mut request = Packet::new(op);
            request.context = 9;
            request.id = 5;
            request.arg = 7;
            let reply = busy(Some(&request));
            assert_eq!((reply.op, reply.context), (op, 9));
            assert_eq!(reply.status, Error::Busy as u8);
            assert_eq!((reply.id, reply.arg, reply.count), (0, 0, 0));
        }
        assert_eq!(busy(None).status, Error::Protocol as u8);
    }
}
