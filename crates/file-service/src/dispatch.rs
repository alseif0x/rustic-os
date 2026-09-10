// SPDX-License-Identifier: Apache-2.0
use crate::{CLIENTS, Grant, reply, transfer::Transfers};
use rustic_abi::files::*;
use rustic_fs::{Disk, Kind, Volume};
pub struct Server {
    pub volume: Volume,
    pub(super) grants: [Option<Grant>; CLIENTS],
    pub(super) roots: [u32; CLIENTS],
    pub(super) next: u32,
    pub(super) transfers: Transfers,
}
impl Server {
    pub fn new(volume: Volume) -> Self {
        Self {
            volume,
            grants: [None; CLIENTS],
            roots: [0; CLIENTS],
            next: 1,
            transfers: Transfers::new(),
        }
    }
    pub fn handle(
        &mut self,
        disk: &mut impl Disk,
        slot: usize,
        peer: u64,
        request: Packet,
        now: u64,
    ) -> Packet {
        let mut response = Packet::new(request.op);
        response.context = request.context;
        let result = (|| {
            crate::validation::request(&request)?;
            let grant = self.grant_at(slot).ok_or(Error::Denied)?;
            grant.check(peer, request.context, now)?;
            if matches!(request.op, REFERENCES | READ_OPEN | READ_CHUNK) {
                response = self.read_request(disk, grant, request)?;
                return Ok(());
            }
            if matches!(request.op, RECOVERY | TRACK_BEGIN | RECEIPT)
                || request.op == COMMIT && self.transfers.tracked(slot)
            {
                response = self.recovery_request(disk, slot, grant, request)?;
                return Ok(());
            }
            let write = matches!(
                request.op,
                CREATE | MKDIR | REMOVE | BEGIN | CHUNK | COMMIT | ABORT
            );
            if matches!(request.op, CHUNK | ABORT) && self.transfers.tracked(slot) {
                grant.inspect(&self.volume, request.id)?;
            } else {
                grant.access(&self.volume, request.id, write)?;
            }
            match request.op {
                LOOKUP => {
                    let n = self
                        .volume
                        .lookup(request.id, request.payload())
                        .map_err(reply::error)?;
                    grant.access(&self.volume, n.id, false)?;
                    response = reply::node(response, n, 0);
                }
                STAT => {
                    response = reply::node(
                        response,
                        self.volume.stat(request.id).map_err(reply::error)?,
                        0,
                    );
                }
                LIST => {
                    let mut cursor = request.arg as usize;
                    loop {
                        match self.volume.list(request.id, cursor).map_err(reply::error)? {
                            Some((next, node)) => {
                                cursor = next;
                                if grant.access(&self.volume, node.id, false).is_ok() {
                                    response = reply::node(response, node, next as u8);
                                    break;
                                }
                            }
                            None => {
                                response.id = 0;
                                break;
                            }
                        }
                    }
                }
                CREATE | MKDIR => {
                    let kind = if request.op == CREATE {
                        Kind::File
                    } else {
                        Kind::Directory
                    };
                    let n = self
                        .volume
                        .create(disk, request.id, request.payload(), kind)
                        .map_err(reply::error)?;
                    response = reply::node(response, n, 0);
                }
                REMOVE => {
                    self.volume.remove(disk, request.id).map_err(reply::error)?;
                }
                READ => {
                    let node = self.volume.stat(request.id).map_err(reply::error)?;
                    if request.version != 0 && request.version != node.version {
                        return Err(Error::Version);
                    }
                    let length = self
                        .volume
                        .read(disk, request.id, request.arg as usize, &mut response.data)
                        .map_err(reply::error)?;
                    response.count = length as u8;
                    response.version = node.version;
                    response.id = node.id;
                    response.arg = u32::from(node.length);
                }
                BEGIN => {
                    let node = self.volume.stat(request.id).map_err(reply::error)?;
                    if node.kind != Kind::File {
                        return Err(Error::IsDirectory);
                    }
                    if node.version != request.version {
                        return Err(Error::Version);
                    }
                    self.transfers.begin(slot, &request)?;
                }
                CHUNK => self.transfers.chunk(slot, &request)?,
                ABORT => self.transfers.clear(slot),
                COMMIT => {
                    let transfer = self.transfers.take(slot, &request)?;
                    grant.check(peer, transfer.context, now)?;
                    grant.access(&self.volume, transfer.id, true)?;
                    let n = self
                        .volume
                        .replace(
                            disk,
                            transfer.id,
                            transfer.version,
                            &transfer.data[..transfer.total],
                        )
                        .map_err(reply::error)?;
                    response = reply::node(response, n, 0);
                }
                _ => return Err(Error::Protocol),
            }
            Ok(())
        })();
        if let Err(error) = result {
            response = Packet::new(request.op);
            response.context = request.context;
            response.status = error as u8;
        }
        response
    }
}
