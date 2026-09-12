// SPDX-License-Identifier: Apache-2.0
// Compile the pure transport guard on the host; native IPC remains separate evidence.
mod transport {
    pub mod control {
        pub mod replies {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/serving/control/replies.rs"
            ));
        }
    }

    use control::replies;
    use rustic_file_service::Grant;
    use rustic_sdk::{
        abi::files::{Error, Packet, READ, READ_RIGHT},
        ipc::Message,
    };

    fn binding() -> Grant {
        Grant {
            peer: 9,
            endpoint: 2,
            scope: 4,
            rights: READ_RIGHT,
            generation: 3,
            expires: 20,
            subject: 1,
        }
    }

    #[test]
    fn expiry_or_revocation_scrubs_a_queued_success_without_losing_correlation() {
        let mut original = Packet::new(READ);
        original.context = 3;
        original.id = 44;
        original.version = 99;
        original.arg = 77;
        original.count = 6;
        original.data[..6].copy_from_slice(b"secret");
        for (mut grant, now, expected) in [
            (binding(), 20, Error::Expired),
            (binding(), 21, Error::Revoked),
        ] {
            if expected == Error::Revoked {
                grant.rights = 0;
            }
            let mut reply = Some(Message::new(17, &original.encode()).unwrap());
            for _ in 0..3 {
                replies::restrict(&mut reply, replies::denial(&grant, now));
                let message = reply.as_ref().unwrap();
                assert_eq!(message.correlation(), 17);
                let mut want = Packet::new(READ);
                want.context = 3;
                want.status = expected as u8;
                assert_eq!(message.payload(), want.encode());
            }
        }
    }

    #[test]
    fn retained_cause_is_scrubbed_when_authority_expires_in_the_reply_queue() {
        use rustic_sdk::abi::files::{admission as a, operation::Instance};
        let original = a::ObservationV2::Retained {
            status: a::Status {
                id: a::AdmissionId::new([7; 16], 9).unwrap(),
                service_instance: Instance::new([7; 16], 8).unwrap(),
                state: a::State::Cancelled,
                terminal: 12,
            },
            prevention: Some(a::PreventionReason::AuthorityLost),
        }
        .packet(3)
        .unwrap();
        let mut reply = Some(Message::new(17, &original.encode()).unwrap());
        replies::restrict(&mut reply, replies::denial(&binding(), 20));
        let message = reply.unwrap();
        let p = Packet::decode(message.payload()).unwrap();
        assert_eq!(message.correlation(), 17);
        assert_eq!(
            (p.op, p.context, p.status),
            (a::OBSERVE, 3, Error::Expired as u8)
        );
        assert_eq!(
            (p.id, p.version, p.arg, p.count, p.data),
            (0, 0, 0, 0, [0; 40])
        );
    }

    #[test]
    fn live_binding_keeps_its_reply_and_expired_binding_drops_malformed_bytes() {
        let original = Message::new(12, &Packet::new(READ).encode()).unwrap();
        let bytes = original.wire().to_vec();
        let mut reply = Some(original);
        replies::restrict(&mut reply, replies::denial(&binding(), 19));
        assert_eq!(reply.as_ref().unwrap().wire(), bytes);
        reply = Some(Message::new(12, b"malformed secret").unwrap());
        replies::restrict(&mut reply, replies::denial(&binding(), 20));
        assert!(reply.is_none());
    }
}
