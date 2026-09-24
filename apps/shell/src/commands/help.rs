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
        output::text("tasks add PATH TITLE | tasks done PATH ID | tasks recover\r\n");
        output::text(
            "tasks hand PID add PATH TITLE | tasks hand PID done PATH ID (plan here, apply in a tasks-owner child)\r\n",
        );
        output::text("tasks enable (one-time persistent storage upgrade for task writes)\r\n");
        output::text("tasks preview add PATH TITLE | tasks preview done PATH ID (no writes)\r\n");
    }
    Ok(())
}

fn advanced_help() {
    output::text(
        "tasks-owner FILE JOURNAL [TICKS] (persistent owner-stepped tasks child; the journal must be a different object)\r\n",
    );
    output::text(
        "tasks-owner-begin PID TOTAL COUNT VERSION TASK_ID CHANGED (announce the planned candidate and its preview summary)\r\n",
    );
    output::text("tasks-owner-edit PID E0 E1 E2 E3 E4 E5 (six preview edit words)\r\n");
    output::text(
        "tasks-owner-chunk PID HEX (up to 64 hex digits = 32 candidate bytes, in order)\r\n",
    );
    output::text("tasks-owner-forget PID KEY (discard recovery evidence; undoes no effect)\r\n");
    #[cfg(feature = "tasks-acceptance")]
    output::text(
        "tasks-owner-apply-cut PID CUT (acceptance build only; 0 none, 1 prepared, 2 lost-reply, 3 lost-journal, 4 conflict)\r\n",
    );
    output::text(
        "act PID tasks-apply|tasks-status|tasks-recover (owner-stepped tasks child; poll with actor-status PID)\r\n",
    );
    output::text(
        "act PID tasks-heap-stress (grow a tasks-owner child's heap to its budget and release it)\r\n",
    );
    output::text(
        "tasks forget INTENT_KEY (discard recovery evidence; does not cancel or undo an effect)\r\n",
    );
    output::text("tasks add PATH TITLE | tasks done PATH ID | tasks recover | tasks enable\r\n");
    output::text(
        "tasks hand PID add PATH TITLE | tasks hand PID done PATH ID (plan here, apply in a tasks-owner child)\r\n",
    );
    output::text("tasks list PATH (read-only native task document)\r\n");
    output::text(
        "tasks preview add PATH TITLE | tasks preview done PATH ID (read-only candidate)\r\n",
    );
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
        "help | pwd | cd PATH | ls [PATH] | mkdir PATH | touch PATH\r\nwrite PATH TEXT | cat PATH | stat PATH | rm PATH | echo TEXT | status\r\nrun spin|fault|exit | run read FILE | run probe FILE OTHER | run watch FILE [TICKS]\r\nps | kill PID | reap PID | permissions [PID] | revoke PID\r\nservices | mem | restart files | exit\r\nretry-key PATH KEY | replace PATH VERSION TOKEN TEXT | receipt ID TOKEN | rotate-receipts\r\nsession FILE OTHER [TICKS] | helper PID FILE OTHER | act PID read|stage|commit|flood|drain|stale\r\nmove-check CLIENT HELPER (moves its file endpoint)\r\nactor-status PID | revocation PID | stall files TICKS (0 = indefinite diagnostic)\r\njob-status [ID] | restart files [async] | hold-io SKIP TICKS | io-status\r\ncapabilities (implemented methods; availability is not permission)\r\nref WORKSPACE PATH | read-ref WORKSPACE RESOURCE VERSION|- OFFSET LENGTH\r\nstage-ref WORKSPACE ELF MANIFEST ELF_VERSION MANIFEST_VERSION (V7 only; dormant child)\r\nstart-staged PID exit|fault|spin (staged rustic.utility only; control channel, no file authority)\r\nenable-operations | replace-ref WORKSPACE RESOURCE VERSION EPOCH KEY TEXT\r\nreplace-fill-ref WORKSPACE RESOURCE VERSION EPOCH KEY BYTE COUNT\r\nreplace-pattern-v7 WORKSPACE RESOURCE VERSION EPOCH KEY SEED SIZE [cut|hold CHUNKS] (V7 only; streamed tracked write; cut revokes this shell's binding mid-write; hold asks for maintenance mid-write, then aborts)\r\noperation-v7 OPERATION_ID | operation-v7 WORKSPACE EPOCH KEY (V7 only; retained receipt)\r\nmaintain-v7 (V7 only; owner job: reclaim retained records, advance the retry epoch)\r\noperation OPERATION_ID | operation WORKSPACE EPOCH KEY\r\nenable-admissions | admit-ref WORKSPACE RESOURCE VERSION EPOCH KEY TEXT\r\nadmission ADMISSION_ID | admission WORKSPACE EPOCH KEY\r\nexecute-admission ADMISSION_ID | cancel-admission ADMISSION_ID\r\nact PID api-read|read-open|read-next|fill|capabilities (deterministic client)\r\nCtrl-C interrupts a wait, not an already submitted effect.\r\nPaths: /system (read-only), /data, /config, /workspaces.\r\nLimits: 32 objects, 1024 bytes/file, 2 utility slots. No AI/network required.\r\n",
    );
}
