// SPDX-License-Identifier: Apache-2.0
use super::output;
pub(super) fn display(r: [u64; 8]) {
    let access = match r[1] {
        1 => "requested",
        2 => "unconfirmed",
        3 => "fenced",
        _ => "invalid",
    };
    let effects = match r[4] {
        0 => "unknown",
        1 => "settled",
        2 => "recovery-required",
        _ => "invalid",
    };
    output::format(format_args!(
        "ok access={access} members={} discarded_staging=",
        r[2]
    ));
    if r[3] == u64::MAX {
        output::text("unknown");
    } else {
        output::format(format_args!("{}", r[3]));
    }
    output::format(format_args!(
        " effects={effects} sequence={} root={} service={}\r\n",
        r[5], r[6], r[7]
    ));
}
pub(super) fn actor(r: [u64; 8]) {
    output::format(format_args!(
        "actor state={} status={} value={} other={} control_denied={} version={}\r\n",
        match r[1] {
            0 => "idle",
            1 => "pending",
            2 => "complete",
            _ => "unconfirmed",
        },
        r[2],
        r[3],
        r[4],
        r[5],
        r[6]
    ));
}
