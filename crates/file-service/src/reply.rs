// SPDX-License-Identifier: Apache-2.0
use rustic_abi::files::{Error, Packet};
use rustic_fs::Node;
pub(super) fn node(mut packet: Packet, node: Node, cursor: u8) -> Packet {
    packet.id = node.id;
    packet.arg = u32::from(node.length);
    packet.version = node.version;
    packet.count = 40;
    packet.data[0] = node.kind as u8;
    packet.data[1] = node.space;
    packet.data[2] = node.name().len() as u8;
    packet.data[3] = cursor;
    packet.data[4..8].copy_from_slice(&node.parent.to_le_bytes());
    packet.data[8..8 + node.name().len()].copy_from_slice(node.name());
    packet
}
pub(super) fn error(error: rustic_fs::Error) -> Error {
    use rustic_fs::Error as E;
    match error {
        E::Io => Error::Io,
        E::Uncertain => Error::Uncertain,
        E::Invalid => Error::Invalid,
        E::Corrupt => Error::Corrupt,
        E::NotFound => Error::NotFound,
        E::Exists => Error::Exists,
        E::NotDirectory => Error::NotDirectory,
        E::IsDirectory => Error::IsDirectory,
        E::NotEmpty => Error::NotEmpty,
        E::Full => Error::Full,
        E::Size => Error::Size,
        E::Version => Error::Version,
        E::ReadOnly => Error::ReadOnly,
        E::Empty => Error::Empty,
        E::Exhausted => Error::Exhausted,
    }
}
