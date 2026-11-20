// SPDX-License-Identifier: Apache-2.0
//! Tracked commit keeps owner control on both receipt profiles and flat recovery.
use super::*;
use core::task::Poll;
use rustic_fs::{PollDisk, PollDisk7, Publication7Phase};

struct Held<'a> {
    disk: &'a mut Sparse,
    pending: Option<(u8, u64)>,
    fail: bool,
}

impl Held<'_> {
    fn command(
        &mut self,
        kind: u8,
        sector: u64,
        run: impl FnOnce(&mut Sparse) -> Result<(), FsError>,
    ) -> Poll<Result<(), FsError>> {
        if let Some(pending) = self.pending.take() {
            assert_eq!(pending, (kind, sector), "pending command was replaced");
            Poll::Ready(if self.fail {
                Err(FsError::Io)
            } else {
                run(self.disk)
            })
        } else {
            self.pending = Some((kind, sector));
            Poll::Pending
        }
    }
}

impl Disk for Held<'_> {
    fn read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Result<(), FsError> {
        assert!(self.pending.is_none());
        self.disk.read(sector, bytes)
    }
    fn write(&mut self, sector: u64, bytes: &[u8; 512]) -> Result<(), FsError> {
        assert!(self.pending.is_none());
        self.disk.write(sector, bytes)
    }
    fn flush(&mut self) -> Result<(), FsError> {
        assert!(self.pending.is_none());
        self.disk.flush()
    }
}

impl PollDisk for Held<'_> {
    fn poll_write(&mut self, sector: u64, bytes: &[u8; 512]) -> Poll<Result<(), FsError>> {
        self.command(1, sector, |disk| disk.write(sector, bytes))
    }
    fn poll_flush(&mut self) -> Poll<Result<(), FsError>> {
        self.command(2, 0, Sparse::flush)
    }
}

impl PollDisk7 for Held<'_> {
    fn poll_read(&mut self, sector: u64, bytes: &mut [u8; 512]) -> Poll<Result<(), FsError>> {
        self.command(0, sector, |disk| disk.read(sector, bytes))
    }
}

#[test]
fn root_loss_during_helper_commit_stops_before_header_or_withholds_settled_receipt() {
    for framing in [0, 1, 2] {
        for (late, fail) in [(false, false), (true, false), (false, true)] {
            let mut f = fixture();
            let mut server = Server7::new(&mut f.volume);
            let root = server
                .grant(0, authority(f.workspace, TRACKED_WRITE7, SUBJECT))
                .unwrap();
            let binding = server
                .derive(
                    1,
                    root.context,
                    GrantRequest7 {
                        endpoint: 91,
                        ..authority(f.workspace, TRACKED_WRITE7, 0)
                    },
                    0,
                )
                .unwrap();
            let helper = Writer {
                slot: 1,
                context: binding.context,
            };
            let before = server.volume().stat(f.file).unwrap();
            let request = replacement(server.volume(), f.workspace, f.file, before.version, 0x901);
            let retry = rustic_abi::files::recovery::Retry {
                lineage: request.workspace.lineage(),
                epoch: request.retry.epoch.value(),
                key: request.retry.key.value(),
            };
            let open = if framing == 2 {
                let mut open = Packet::new(rustic_abi::files::TRACK_BEGIN);
                open.context = helper.context;
                open.id = f.file;
                open.version = before.version;
                open.arg = 900;
                open.count = 32;
                open.data[..32].copy_from_slice(&retry.encode());
                open
            } else if framing == 1 {
                request.packet(900, helper.context).unwrap()
            } else {
                Replacement { request }.packet(900, helper.context).unwrap()
            };
            status(helper.send(&mut server, &mut f.disk, open)).unwrap();
            for (index, bytes) in [77; 900].chunks(DATA).enumerate() {
                let mut chunk = Packet::new(if framing == 2 {
                    rustic_abi::files::CHUNK
                } else {
                    REPLACE_CHUNK
                });
                chunk.id = f.file;
                chunk.arg = (index * DATA) as u32;
                chunk.count = bytes.len() as u8;
                chunk.data[..bytes.len()].copy_from_slice(bytes);
                status(helper.send(&mut server, &mut f.disk, chunk)).unwrap();
            }
            let mut plain = Packet::new(rustic_abi::files::BEGIN);
            plain.context = root.context;
            plain.id = f.sibling;
            plain.version = server.volume().stat(f.sibling).unwrap().version;
            plain.arg = 1;
            status(server.handle(&mut f.disk, 0, PEER, plain, 0)).unwrap();
            assert_eq!(server.pending(), 2);
            let mut commit = Packet::new(if framing == 2 {
                rustic_abi::files::COMMIT
            } else {
                REPLACE_COMMIT
            });
            commit.context = helper.context;
            commit.id = f.file;
            let mut disk = Held {
                disk: &mut f.disk,
                pending: None,
                fail,
            };
            let mut revoked = false;
            let reply = server.handle_with(&mut disk, 1, PEER, commit, 0, |control| {
                if !revoked
                    && control.pending()
                    && (!late || control.phase() == Publication7Phase::Settling)
                {
                    assert_eq!(
                        control.transfer_count(),
                        1,
                        "publisher already consumed its candidate"
                    );
                    assert_eq!(control.revoke_root(root.context), 3);
                    assert_eq!(control.transfer_count(), 0);
                    revoked = true;
                }
                0
            });
            assert!(revoked);
            assert!(disk.pending.is_none(), "pending I/O was not drained");
            assert_eq!(
                reply.status,
                if late || fail {
                    Error::Uncertain as u8
                } else {
                    Error::Revoked as u8
                }
            );
            assert_eq!(reply.count, 0);
            assert_eq!(server.pending(), 0);
            assert_eq!(server.volume().open_stages(), 0);
            if fail {
                assert!(server.volume().header().is_err());
                continue;
            }
            let after = server.volume().stat(f.file).unwrap();
            assert_eq!(after.length, if late { 900 } else { 0 });
            assert_eq!(after.version > before.version, late);
            assert_eq!(
                server
                    .volume()
                    .retained_records()
                    .unwrap()
                    .iter()
                    .flatten()
                    .count(),
                usize::from(late)
            );
            let fresh = Writer::grant(&mut server, 1, f.workspace, TRACKED_WRITE7, SUBJECT);
            let query = if framing == 2 {
                let mut query = Packet::new(rustic_abi::files::RECEIPT);
                query.context = fresh.context;
                query.id = f.file;
                query.count = 32;
                query.data[..32].copy_from_slice(&retry.encode());
                query
            } else {
                Lookup {
                    query: operation::Lookup::Retry {
                        workspace: request.workspace,
                        retry: request.retry,
                    },
                }
                .packet(fresh.context)
            };
            assert_eq!(
                fresh.send(&mut server, &mut f.disk, query).status,
                if late { 0 } else { Error::OutcomeUnknown as u8 }
            );
        }
    }
}
