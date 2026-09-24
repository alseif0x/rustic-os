// SPDX-License-Identifier: Apache-2.0
//! Scope checks over the mounted generation's verified parent links only.
//! Caller-claimed workspaces are never trusted without walking real ancestry.
use crate::reply;
use rustic_abi::files::Error;
use rustic_fs::{
    Error as FsError, Kind, Volume7,
    format7::{NODES, Node7},
};

/// Root of every grantable V7 workspace.
const WORKSPACES_ROOT: u32 = 4;

/// A grant scope must exist and lie within the workspaces tree.
pub(super) fn grantable(volume: &Volume7, scope: u32) -> Result<(), Error> {
    let scope_node = match volume.stat(scope) {
        Ok(node) => node,
        Err(FsError::NotFound) => return Err(Error::Invalid),
        Err(error) => return Err(reply::error(error)),
    };
    if within(volume, scope_node, node(volume, WORKSPACES_ROOT)?)? {
        Ok(())
    } else {
        Err(Error::Denied)
    }
}

/// The resource node when `workspace` is a directory containing `resource`
/// and both lie within the grant `scope`; otherwise `Denied`.
pub(super) fn authorized_resource(
    volume: &Volume7,
    scope: u32,
    workspace: u32,
    resource: u32,
) -> Result<Node7, Error> {
    let workspace_node = node(volume, workspace)?;
    if workspace_node.kind != Kind::Directory {
        return Err(Error::Denied);
    }
    let resource_node = node(volume, resource)?;
    if !within(volume, resource_node, workspace_node)?
        || !within(volume, resource_node, node(volume, scope)?)?
    {
        return Err(Error::Denied);
    }
    Ok(resource_node)
}

fn node(volume: &Volume7, id: u32) -> Result<Node7, Error> {
    volume.stat(id).map_err(|error| match error {
        FsError::NotFound => Error::Denied,
        other => reply::error(other),
    })
}

/// A file scope reaches itself; only directories can authorize descendants.
fn within(volume: &Volume7, resource: Node7, ancestor: Node7) -> Result<bool, Error> {
    if resource.id == ancestor.id {
        return Ok(true);
    }
    if ancestor.kind != Kind::Directory || resource.space != ancestor.space {
        return Ok(false);
    }
    let mut current = resource;
    for _ in 0..NODES {
        if current.parent == 0 {
            return Ok(false);
        }
        let parent = node(volume, current.parent)?;
        if parent.kind != Kind::Directory || parent.space != current.space {
            return Ok(false);
        }
        if parent.id == ancestor.id {
            return Ok(true);
        }
        current = parent;
    }
    Ok(false)
}
