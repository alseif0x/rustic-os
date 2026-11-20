// SPDX-License-Identifier: Apache-2.0
//! Authority checks over verified live V7 ancestry.
//!
//! A zero primary scope reaches every live node. A nonzero primary scope
//! reaches itself and descendants; the optional second scope is checked as an
//! independent, live file scope and never extends the primary subtree.
use super::Grant7;
use crate::reply;
use rustic_abi::files::Error;
use rustic_fs::{
    Error as FsError, Kind, Volume7,
    format7::{NODES, Node7},
};

/// Canonical V7 workspaces root, retained for the terminal's virtual-root view.
pub(super) const WORKSPACES_ROOT: u32 = 4;

/// A grant scope is zero (the volume) or any live node.
pub(super) fn grantable(volume: &Volume7, scope: u32) -> Result<(), Error> {
    if scope == 0 {
        return Ok(());
    }
    volume.stat(scope).map(|_| ()).map_err(|error| match error {
        FsError::NotFound => Error::Invalid,
        other => reply::error(other),
    })
}

/// Whether a prospective child scope fits entirely within its primary parent.
/// A global parent may attenuate to any requested scope; installation performs
/// the live-node check after this relation is established.
pub(super) fn scope_within(volume: &Volume7, child: u32, parent: u32) -> bool {
    if parent == 0 {
        return true;
    }
    if child == 0 {
        return false;
    }
    let (Ok(child_node), Ok(parent_node)) = (volume.stat(child), volume.stat(parent)) else {
        return false;
    };
    within(volume, child_node, parent_node).unwrap_or(false)
}

/// A second scope is either absent or one live file disjoint from the primary.
pub(super) fn validate_second(volume: &Volume7, primary: u32, second: u32) -> Result<(), Error> {
    if second == 0 {
        return Ok(());
    }
    if primary == 0 || second == primary {
        return Err(Error::Invalid);
    }
    let second_node = volume.stat(second).map_err(|_| Error::Invalid)?;
    if second_node.kind != Kind::File {
        return Err(Error::Invalid);
    }
    let primary_node = volume.stat(primary).map_err(|_| Error::Invalid)?;
    if within(volume, second_node, primary_node)? || within(volume, primary_node, second_node)? {
        return Err(Error::Invalid);
    }
    Ok(())
}

/// A node identity when it is within the grant's independent primary or second
/// scope. Missing and out-of-scope identities are indistinguishable.
pub(super) fn authorized_node(volume: &Volume7, grant: Grant7, id: u32) -> Result<Node7, Error> {
    let resource = node(volume, id)?;
    if allows_node(volume, grant, resource)? {
        Ok(resource)
    } else {
        Err(Error::Denied)
    }
}

/// Verify the caller's claimed workspace ancestry and then require the
/// resource itself to be reachable through one of the grant's scopes.
pub(super) fn authorized_resource(
    volume: &Volume7,
    grant: Grant7,
    workspace: u32,
    resource: u32,
) -> Result<Node7, Error> {
    let workspace_node = node(volume, workspace)?;
    if workspace_node.kind != Kind::Directory {
        return Err(Error::Denied);
    }
    let resource_node = node(volume, resource)?;
    if !within(volume, resource_node, workspace_node)?
        || !allows_node(volume, grant, resource_node)?
    {
        return Err(Error::Denied);
    }
    Ok(resource_node)
}

/// Whether one of the primary scope's or the live second file's identities
/// can place a retained record inside this grant. A primary scope exactly
/// naming the removed workspace or object preserves legacy owner inspection;
/// a removed second-scope file authorizes nothing.
pub(super) fn retained_visible(
    volume: &Volume7,
    grant: Grant7,
    workspace: u32,
    object: u32,
) -> bool {
    if grant.scope == 0 || grant.scope == workspace || grant.scope == object {
        return true;
    }
    if let Ok(primary) = volume.stat(grant.scope) {
        for id in [workspace, object] {
            if let Ok(live) = volume.stat(id)
                && within(volume, live, primary).unwrap_or(false)
            {
                return true;
            }
        }
    }
    if grant.second == 0 {
        return false;
    }
    let Ok(second) = volume.stat(grant.second) else {
        return false;
    };
    second.kind == Kind::File
        && [workspace, object].into_iter().any(|id| {
            volume
                .stat(id)
                .is_ok_and(|live| within(volume, live, second).unwrap_or(false))
        })
}

/// Whether a root node may appear through the terminal's virtual parent zero.
pub(super) fn root_visible(grant: Grant7, node: Node7) -> bool {
    node.parent == 0
        && (1..=4).contains(&node.id)
        && (grant.scope == 0 || grant.scope == WORKSPACES_ROOT && node.id == WORKSPACES_ROOT)
}

fn allows_node(volume: &Volume7, grant: Grant7, resource: Node7) -> Result<bool, Error> {
    if grant.scope == 0 {
        return Ok(true);
    }
    if let Ok(primary) = volume.stat(grant.scope)
        && within(volume, resource, primary)?
    {
        return Ok(true);
    }
    if grant.second == 0 {
        return Ok(false);
    }
    let Ok(second) = volume.stat(grant.second) else {
        return Ok(false);
    };
    if second.kind != Kind::File {
        return Ok(false);
    }
    within(volume, resource, second)
}

fn node(volume: &Volume7, id: u32) -> Result<Node7, Error> {
    volume.stat(id).map_err(|error| match error {
        FsError::NotFound => Error::Denied,
        other => reply::error(other),
    })
}

/// Follow live parent links, rejecting cross-space edges and cycles. A file is
/// reachable only as the exact scope or as a disjoint second file identity.
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
    Err(Error::Corrupt)
}
