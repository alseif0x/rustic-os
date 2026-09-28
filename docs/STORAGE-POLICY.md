<!-- SPDX-License-Identifier: Apache-2.0 -->

# Storage format policy

RusticOS has no user volumes. Every volume that exists today is a disposable
development or test image created by the project's own tools, and no one's data
depends on reading it again.

## Supported on-disk formats

- **Workspace format 7** (magic `RUSTFS3`) is the one supported workspace format.
  Its layout, validation and host tools are described in
  [WORKSPACE-FORMAT7.md](WORKSPACE-FORMAT7.md).
- **The legacy `RUSTFS1` volume** is still used by the native terminal: the
  default `mode=terminal` file service and the `block-user` publication and
  admission fixtures run on it ([file service](FILES.md)). It keeps the in-layout
  format-version upgrades it already has (versions 2 to 5 of that same layout);
  those are existing behavior of the legacy volume, not a precedent for new ones.

No other on-disk format is supported. An intermediate v6 extent layout and the
host-only v5 to v7 converter were removed; no data was carried from either.

## No migration, upgrade, rollback or compatibility path

No migration, upgrade, rollback or cross-format compatibility path is implemented,
and none is accepted as a change, until a real user's volume exists. A format
change is therefore made by re-provisioning: create a fresh image with the current
tools (for V7, `rustic-volume seed7`, then `add7` or `maintain7` when a fixture
needs more content) and discard the old one. A reader refuses an image it does not
recognize instead of converting it.

When a real user's volume first exists, this policy must be revisited in its own
decision before any path that preserves that volume's data is designed.
