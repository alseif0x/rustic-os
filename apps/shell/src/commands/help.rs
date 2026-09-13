// SPDX-License-Identifier: Apache-2.0
use super::{Error, output};
use rustic_shell::parser::Args;

const DEFAULT: &str = "help | pwd | cd PATH | ls [PATH] | mkdir PATH | touch PATH\r\nwrite PATH TEXT | cat PATH | stat PATH | rm PATH | echo TEXT | status\r\nrun read FILE | run watch FILE [TICKS]\r\nps | kill PID | reap PID | permissions [PID] | revoke PID\r\ncapabilities | services | mem | restart files | exit\r\nCtrl-C interrupts a wait, not an already submitted effect.\r\nType help advanced for the complete command reference and diagnostics.\r\n";

pub(super) fn execute(args: &Args<'_>) -> Result<(), Error> {
    let advanced = match args.len() {
        1 => false,
        2 if args.get(1) == Some("advanced") => true,
        _ => return Err(Error::Usage),
    };

    if advanced {
        advanced_help();
    } else {
        output::text(DEFAULT);
        output::text("tasks list PATH\r\n");
    }
    Ok(())
}

fn advanced_help() {
    output::text("tasks list PATH (read-only native task document)\r\n");
    output::text(
        "select-lifecycle operations.get|operations.cancel\r\ninspect-selected ADMISSION_ID | cancel-selected ADMISSION_ID\r\nact PID select-get|select-cancel|mission-prepare|mission-verify\r\nact-admission PID inspect-selected|cancel-selected ADMISSION_ID\r\n",
    );
    output::text(
        "lifecycle-profile operations.get|operations.cancel\r\ninspect-negotiated ADMISSION_ID | cancel-negotiated ADMISSION_ID\r\nact PID profile-get|profile-cancel | act-admission PID inspect-negotiated|cancel-negotiated ADMISSION_ID\r\n",
    );
    output::text(
        "inspect-operation ADMISSION_ID | request-operation-cancel ADMISSION_ID\r\nact-admission PID inspect|cancel|lost-cancel ADMISSION_ID (service v2)\r\n",
    );
    output::text("enable-prevention-reasons (explicit persistent format v5 upgrade)\r\n");
    output::text(
        "observe-admission ADMISSION_ID | observe-admission-v2 ADMISSION_ID | schedule-admission ADMISSION_ID | admission-activity ADMISSION_ID | request-cancel ADMISSION_ID\r\nadmission-session FILE OTHER RIGHTS [private] | act-admission PID execute|schedule|get|observe|observe-v2|activity|request-cancel|lost-stop|lost-schedule|lost-result ADMISSION_ID\r\nScheduling returns before settlement; cancellation replies acknowledge a request, not durable prevention. Private sessions use a supervisor-assigned subject.\r\n",
    );
    output::text(
        "help | pwd | cd PATH | ls [PATH] | mkdir PATH | touch PATH\r\nwrite PATH TEXT | cat PATH | stat PATH | rm PATH | echo TEXT | status\r\nrun spin|fault|exit | run read FILE | run probe FILE OTHER | run watch FILE [TICKS]\r\nps | kill PID | reap PID | permissions [PID] | revoke PID\r\nservices | mem | restart files | exit\r\nretry-key PATH KEY | replace PATH VERSION TOKEN TEXT | receipt ID TOKEN | rotate-receipts\r\nsession FILE OTHER [TICKS] | helper PID FILE OTHER | act PID read|stage|commit|flood|drain|stale\r\nmove-check CLIENT HELPER (moves its file endpoint)\r\nactor-status PID | revocation PID | stall files TICKS (0 = indefinite diagnostic)\r\njob-status [ID] | restart files [async] | hold-io SKIP TICKS | io-status\r\ncapabilities (implemented methods; availability is not permission)\r\nref WORKSPACE PATH | read-ref WORKSPACE RESOURCE VERSION|- OFFSET LENGTH\r\nenable-operations | replace-ref WORKSPACE RESOURCE VERSION EPOCH KEY TEXT\r\nreplace-fill-ref WORKSPACE RESOURCE VERSION EPOCH KEY BYTE COUNT\r\noperation OPERATION_ID | operation WORKSPACE EPOCH KEY\r\nenable-admissions | admit-ref WORKSPACE RESOURCE VERSION EPOCH KEY TEXT\r\nadmission ADMISSION_ID | admission WORKSPACE EPOCH KEY\r\nexecute-admission ADMISSION_ID | cancel-admission ADMISSION_ID\r\nact PID api-read|read-open|read-next|fill|capabilities (deterministic client)\r\nCtrl-C interrupts a wait, not an already submitted effect.\r\nPaths: /system (read-only), /data, /config, /workspaces.\r\nLimits: 32 objects, 1024 bytes/file, 2 utility slots. No AI/network required.\r\n",
    );
}
