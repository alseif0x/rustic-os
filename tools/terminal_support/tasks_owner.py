# SPDX-License-Identifier: Apache-2.0
"""Second native semantic tasks client: apps/utility under supervisor role TASKS_OWNER.

Every statement here is made from ordinary guest commands plus an independent
host reader of the committed volume. Nothing in this file is evidence until the
suite actually runs inside QEMU.

The owner steps, the reply layouts and the phase values are the contract stated
in docs/TASKS.md ("Second native client (utility)") and implemented in
apps/utility/src/tasks/{owner,state,report}.rs; every numeric refusal code used
below is cited at the Rust line that defines it.
"""
import json
import re
import tempfile
import time
from pathlib import Path
from boot_support.image import package
from .authority_cases import actor_result, cleanup, fence
from .cases import counters, exited, pid
from .connection import Connection
from .failure import preserve_failure
from .machine import disk, machine
from .oracle import snapshot

DOC = 'tasks-owner-doc'
PEER = 'tasks-owner-peer'
JOURNAL_NAME = 'tasks-owner-journal'
JOURNAL = '/config/' + JOURNAL_NAME
# The shell client's own record; a second client may never be handed this object.
SHELL_RECORD = '/config/tasks-intent'
DOCUMENT = 'rustic-tasks-v1\n7\topen\tReview kernel\n42\tdone\tBoot the OS\n'
# One owner chunk carries at most 32 bytes: crates/tasks-contract/src/candidate.rs:19.
MAX_CHUNK = 32
# File refusals keep the file ABI's own numbering, crates/abi/src/files.rs:56-88:
# Uncertain = 3 (line 59), Full = 11 (line 67), Version = 13 (line 69),
# Revoked = 18 (line 74), OutcomeUnknown = 27 (line 83), Unavailable = 31 (line 87).
UNCERTAIN, FULL, VERSION, REVOKED, OUTCOME_UNKNOWN, UNAVAILABLE = 3, 11, 13, 18, 27, 31
# Failure cut selectors of word 1 of TASKS_APPLY, apps/utility/src/tasks.rs:26-38
# and apps/utility/src/tasks/owner.rs (`cut`). Only 0 exists outside an explicit
# acceptance build; the supervisor refuses anything above MAX_CUT = 4
# (apps/supervisor/src/tasks_owner.rs:14,77).
NONE, PREPARED, LOST_REPLY, LOST_JOURNAL, CONFLICT, UNIMPLEMENTED = 0, 1, 2, 3, 4, 5
# The owner-client refusals, apps/utility/src/tasks/report.rs:38-48: journal 103,
# an unresolved intent 104, and the client's own out-of-phase refusal 105.
JOURNAL_CODE, PENDING, SEQUENCE = 103, 104, 105
# Phase values, apps/utility/src/tasks/state.rs:36-45 and docs/TASKS.md:81.
IDLE, EDIT, COLLECTING, READY, FINISHED = 0, 1, 2, 3, 4
# The intent record's magic, as the shared client writes it (tasks_write.py:37).
MAGIC = b'RTSKI001'


# --- Reply decoding ----------------------------------------------------------
# `actor-status PID` prints the supervisor phase plus child reply words 0..4 as
# status,value,other,control_denied,version (apps/supervisor/src/sessions.rs:149
# and apps/shell/src/commands/takeover.rs:30). Each step names those five words
# differently, so the four layouts of docs/TASKS.md:74-79 are decoded once here.

def reply_step(v):
    return {'error': v['status'], 'cursor': v['value'], 'total': v['other'],
            'phase': v['control_denied']}


def reply_apply(v):
    return {'error': v['status'], 'task': v['value'], 'journal': v['other'],
            'applied': v['control_denied'], 'version': v['version']}


def reply_status(v):
    return {'error': v['status'], 'phase': v['value'], 'cursor': v['other'],
            'total': v['control_denied'], 'pending': v['version']}


def reply_recover(v):
    return {'error': v['status'], 'recovered': v['value'], 'journal': v['other'],
            'task': v['control_denied'], 'version': v['version']}


# --- Guest command helpers ---------------------------------------------------

def settled(uart, child):
    """Polls one owner-stepped action to completion without assuming its code."""
    deadline = time.monotonic() + 12
    while True:
        output = uart.command(f'actor-status {child}')
        if 'actor state=complete' in output:
            return {key: int(value) for key, value in re.findall(
                r'(status|value|other|control_denied|version)=(\d+)', output)}
        if time.monotonic() >= deadline:
            raise AssertionError(output)
        time.sleep(.02)


def act(uart, child, verb, error=0):
    """One ACT verb, polled until the child's answer is complete."""
    uart.command(f'act {child} {verb}', 'actor state=pending')
    return actor_result(uart, child, error)


def apply(uart, child, error=0):
    return reply_apply(act(uart, child, 'tasks-apply', error))


def apply_cut(uart, child, cut, error=0):
    """`tasks-owner-apply-cut PID CUT`: an apply that takes an explicit cut.

    It is the same child action with the selector added, so it keeps the apply's
    own delivery window and is polled exactly like an ordinary apply
    (apps/supervisor/src/tasks_owner.rs:71-81).
    """
    uart.command(f'tasks-owner-apply-cut {child} {cut}', 'actor state=pending')
    return reply_apply(actor_result(uart, child, error))


def query(uart, child, error=0):
    return reply_status(act(uart, child, 'tasks-status', error))


def recover(uart, child, error=0):
    return reply_recover(act(uart, child, 'tasks-recover', error))


def step(uart, child, command, error=0):
    """One `tasks-owner-*` owner step, polled until the child answers it."""
    uart.command(command, 'actor state=pending')
    return reply_step(actor_result(uart, child, error))


def failure(uart, command, expected):
    output = uart.command(command, 'error: ' + expected)
    assert [line for line in output.splitlines() if line.startswith('error:')] == ['error: ' + expected]
    uart.command('status', '1')
    return output


def stat(uart, path):
    fields = re.findall(r'(id|parent|bytes|version)=(\d+)', uart.command(f'stat {path}'))
    return {key: int(value) for key, value in fields}


def write_document(uart, path, text):
    escaped = text.replace('\\', '\\\\').replace('"', '\\"').replace('\n', '\\n').replace('\t', '\\t')
    uart.command(f'write {path} "{escaped}"', f'written {len(text)} bytes')


def launch(uart, document=DOC, journal=JOURNAL):
    return pid(uart, f'tasks-owner {document} {journal}')


def hand(uart, child, action, value, expected, path=DOC):
    """`tasks hand PID ACTION PATH VALUE`, checked against the plan it relayed."""
    output = uart.command(f'tasks hand {child} {action} {path} {value}', f'handed pid={child}')
    match = re.search(rf'handed pid={child} bytes=(\d+) chunks=(\d+) task=(\d+)', output)
    assert match, output
    size, chunks, task = (int(group) for group in match.groups())
    assert size == len(expected), ('handed length differs from the candidate', size, len(expected))
    assert chunks == -(-size // MAX_CHUNK), ('chunk count', chunks, size)
    return task


# --- Pure document expectations ----------------------------------------------
# The planner's documented rules (docs/TASKS.md:41), restated so the disk oracle
# compares against independently computed bytes rather than the guest's output.

def added(document, title):
    ids = [int(line.split('\t')[0]) for line in document.splitlines()[1:]]
    return document + f'{max(ids, default=0) + 1}\topen\t{title}\n'


def completed(document, task):
    return ''.join(line.replace('\topen\t', '\tdone\t', 1) if line.startswith(f'{task}\t') else line
                   for line in document.splitlines(keepends=True))


# --- Independent disk assertions ---------------------------------------------

def state(data, document, *, pending=False, records=None):
    _, value = snapshot(data)
    assert value['files'][(4, DOC)] == document.encode('ascii'), 'committed document differs'
    journal = value['files'][(3, JOURNAL_NAME)]
    assert bool(journal) == pending, ('retained intent presence', pending)
    if pending:
        assert journal.startswith(MAGIC) and len(journal) > 136, 'journal is not an owner intent record'
    if records is not None:
        assert len(value['records']) == records, ('retained outcomes', records, len(value['records']))
    return value


def retained(value, ids):
    """The journal version an unresolved record is keyed by."""
    return value['nodes'][ids['journal']]['version']


def receipt(value, key, expected, subject):
    records = [record for record in value['records'] if record['key'] == key]
    assert len(records) == 1, ('unique receipt for the intent key', key)
    record = records[0]
    # This client is identified by the journal object it was granted, not by the
    # shell's owner subject 1: two clients that retain their own intents may not
    # share one retry namespace (apps/supervisor/src/grant.rs, Request::subject).
    assert record['subject'] == subject and record['workspace'] == 4, record
    assert record['committed'] > key, 'receipt is older than the journal key it claims'
    assert record['content'] == expected, 'receipt bytes are not the applied candidate'
    return record['committed']


# --- Cases -------------------------------------------------------------------

def setup(uart, data):
    """Fixtures: one task document, one peer document, one empty journal."""
    baseline = counters(uart)
    uart.command('tasks enable', 'persistent storage format is at least v3')
    write_document(uart, DOC, DOCUMENT)
    write_document(uart, PEER, DOCUMENT)
    # The child never creates its record: the granted journal object must already
    # exist (crates/tasks-client/src/record.rs:61-67, journal.rs:20-26).
    uart.command(f'touch {JOURNAL}')
    ids = {'document': stat(uart, DOC)['id'], 'peer': stat(uart, PEER)['id'],
           'journal': stat(uart, JOURNAL)['id']}
    assert len(set(ids.values())) == 3, ('the two scopes and the peer are distinct objects', ids)
    _, value = snapshot(data)
    assert value['files'][(3, JOURNAL_NAME)] == b'', 'journal fixture is not an idle record'
    return baseline, ids


def applied_effect(uart, data, ids, child, document, action, value, affected, records):
    """One hand-off plus apply that must commit, proved against the volume."""
    candidate = added(document, affected) if action == 'add' else completed(document, int(affected))
    task = hand(uart, child, action, value, candidate)
    result = apply(uart, child)
    assert result['applied'] == 1, ('a committed and verified effect', result)
    assert result['task'] == task and result['journal'] != 0 and result['version'] != 0, result
    committed = state(data, candidate, records=records)
    assert receipt(committed, result['journal'], candidate.encode('ascii'),
                   ids['journal']) == result['version']
    assert committed['nodes'][ids['document']]['version'] == result['version'], \
        'reported commit version differs from the committed object version'
    # The concluded apply released its candidate; the stored edit survives.
    assert query(uart, child)['phase'] == FINISHED
    return candidate, result


def parity(uart, data, ids, child, results):
    """The same two commands through the shell client and the second client."""
    document = DOCUMENT
    # The shell client edits its own fixture first; its bytes are the reference.
    uart.command(f'tasks add {PEER} "Ship Rust"', 'task=43 applied')
    _, value = snapshot(data)
    reference = value['files'][(4, PEER)]
    assert reference == added(document, 'Ship Rust').encode('ascii'), 'shell add differs from the rule'
    # Free both retained slots; the shell effect is already verified and cleared.
    uart.command('rotate-receipts')
    document, result = applied_effect(uart, data, ids, child, document, 'add',
                                      '"Ship Rust"', 'Ship Rust', 1)
    _, value = snapshot(data)
    assert value['files'][(4, DOC)] == reference, 'second client produced other bytes than the shell'
    results['add'] = result

    uart.command(f'tasks done {PEER} 7', 'task=7 applied')
    _, value = snapshot(data)
    reference = value['files'][(4, PEER)]
    uart.command('rotate-receipts')
    document, result = applied_effect(uart, data, ids, child, document, 'done', '7', '7', 1)
    _, value = snapshot(data)
    assert value['files'][(4, DOC)] == reference, 'second client produced other bytes than the shell'
    results['done'] = result

    # An already completed task submits nothing: no receipt, no journal write.
    before, _ = snapshot(data)
    hand(uart, child, 'done', '7', document)
    noop = apply(uart, child)
    assert noop['applied'] == 0 and noop['task'] == 7 and noop['journal'] == 0, noop
    assert noop['version'] == stat(uart, DOC)['version'], noop
    after, _ = snapshot(data)
    assert before == after, 'an unchanged edit consumed storage or a receipt'
    state(data, document, records=1)
    results['unchanged'] = noop
    return document


def conflict(uart, data, child, document, results):
    """A human edit between the plan and the apply is a conclusive refusal."""
    hand(uart, child, 'add', '"Conflict"', added(document, 'Conflict'))
    # Same bytes, new version: the version pin, not the content, is the check.
    write_document(uart, DOC, document)
    before, _ = snapshot(data)
    result = apply(uart, child, VERSION)
    assert result == {'error': VERSION, 'task': 0, 'journal': 0, 'applied': 0, 'version': 0}, result
    after, _ = snapshot(data)
    assert before == after, 'a refused apply changed the volume'
    state(data, document, records=1)
    # Version is one of the six canonical refusals, so the plan was released.
    assert query(uart, child)['phase'] == FINISHED
    results['conflict'] = result


def sequence(uart, child, results):
    """Steps the current phase does not accept change nothing and say so."""
    # No candidate is held after the concluded apply above.
    empty = apply(uart, child, SEQUENCE)
    assert empty == {'error': SEQUENCE, 'task': 0, 'journal': 0, 'applied': 0, 'version': 0}, empty
    # 0x41 is one legal chunk payload; without a begin there is nothing to append.
    chunk = step(uart, child, f'tasks-owner-chunk {child} 41', SEQUENCE)
    assert chunk == {'error': SEQUENCE, 'cursor': 0, 'total': 0, 'phase': FINISHED}, chunk
    # A non-canonical summary (version 0) is refused by the supervisor word
    # translation, apps/supervisor/src/tasks_owner.rs:30-34; the child never sees it.
    before = query(uart, child)
    failure(uart, f'tasks-owner-begin {child} 40 2 0 42 1', 'service invalid request')
    assert query(uart, child) == before, 'a refused owner step changed the child'
    results['sequence'] = {'apply': empty, 'chunk': chunk, 'phase': before['phase']}


def denied(uart, data, child, document, results):
    """A revoked grant refuses both the apply and the status query."""
    hand(uart, child, 'add', '"Revoked"', added(document, 'Revoked'))
    assert query(uart, child)['phase'] == READY
    # The comparison starts after the revocation settles: only the refused apply
    # is being measured here, not the file service's own fencing work.
    fence(uart, child, 'access=fenced')
    before, _ = snapshot(data)
    # The refusal happens in the guard, before anything is retained, so the reply
    # carries the code alone (apps/utility/src/tasks/report.rs:79-82).
    result = apply(uart, child, REVOKED)
    assert result == {'error': REVOKED, 'task': 0, 'journal': 0, 'applied': 0, 'version': 0}, result
    revoked = query(uart, child, REVOKED)
    assert revoked['pending'] == 0, 'an unreadable record cannot report a pending version'
    after, _ = snapshot(data)
    assert before == after, 'a revoked client changed the volume'
    state(data, document, records=1)
    results['revoked'] = {'apply': result, 'status': revoked}


def launch_refusals(uart, data, baseline, results):
    """The two scopes must be two distinct, existing, unshared objects."""
    before, _ = snapshot(data)
    usage = 'invalid arguments; type help'
    # A journal aliasing the target would record its recovery evidence inside the
    # document that evidence protects, apps/shell/src/commands/authority.rs:24-32.
    failure(uart, f'tasks-owner {DOC} {DOC}', usage)
    # The shell keeps its own record; a second client may not be handed it.
    failure(uart, f'tasks-owner {DOC} {SHELL_RECORD}', usage)
    failure(uart, f'tasks-owner {DOC}', usage)
    failure(uart, f'tasks-owner {DOC} /config/tasks-owner-absent', 'NotFound')
    # A directory passes the supervisor's own check (a live object above the owner
    # subject and distinct from the scope) and is then refused by the file service,
    # which grants only one live file as second scope
    # (crates/file-service/src/grants.rs:78-86). The already installed root must be
    # withdrawn, so the job ends with the owner's service error and no child.
    withdrawn = failure(uart, f'tasks-owner {DOC} /config', 'service denied')
    assert 'started pid=' not in withdrawn, withdrawn
    after, _ = snapshot(data)
    assert before == after, 'a refused launch changed the volume'
    assert counters(uart) == baseline, 'a refused launch leaked a process or channel'
    # The withdrawn slot is immediately reusable by a valid launch.
    reused = launch(uart)
    assert query(uart, reused) == {'error': 0, 'phase': IDLE, 'cursor': 0, 'total': 0, 'pending': 0}
    cleanup(uart, reused)
    assert counters(uart) == baseline, (baseline, counters(uart))
    results['launch_refusals'] = {'aliased_journal': usage, 'shell_record': usage,
                                  'too_few_arguments': usage, 'missing_journal': 'NotFound',
                                  'directory_journal': 'service denied', 'slot_reused': True}


def cuts(uart, data, ids, document, results):
    """The deterministic failure cuts of the acceptance build.

    Each cut starts from a rotated inventory and a freshly written document, so
    the case is decided by the cut and not by what an earlier one left behind.
    """
    observed = {}
    length = None

    # Cut 1: the intent is retained and the target is never submitted.
    uart.command('rotate-receipts')
    write_document(uart, DOC, document)
    child = launch(uart)
    candidate = added(document, 'Prepared')
    hand(uart, child, 'add', '"Prepared"', candidate)
    prepared = apply_cut(uart, child, PREPARED, UNCERTAIN)
    value = state(data, document, pending=True, records=0)
    key = retained(value, ids)
    assert prepared == {'error': UNCERTAIN, 'task': 0, 'journal': key,
                        'applied': 0, 'version': 0}, prepared
    length = len(candidate)
    # Uncertain proves nothing, so the plan stays available to the recovery that
    # must resolve it: the phase is still ready and the bytes are still held.
    assert query(uart, child) == {'error': 0, 'phase': READY, 'cursor': length,
                                  'total': length, 'pending': key}
    # The unresolved intent blocks the next mutation, even a freshly planned one.
    hand(uart, child, 'add', '"Blocked"', added(document, 'Blocked'))
    blocked = apply(uart, child, PENDING)
    assert blocked == {'error': PENDING, 'task': 0, 'journal': 0, 'applied': 0, 'version': 0}, blocked
    before, _ = snapshot(data)
    unresolved = recover(uart, child, OUTCOME_UNKNOWN)
    assert unresolved == {'error': OUTCOME_UNKNOWN, 'recovered': 0, 'journal': key,
                          'task': 0, 'version': 0}, unresolved
    assert snapshot(data)[0] == before, 'recovery replayed or rebased an absent outcome'
    assert step(uart, child, f'tasks-owner-forget {child} {key}')['error'] == 0
    state(data, document, records=0)
    cleanup(uart, child)
    observed['prepared'] = {'apply': prepared, 'blocked': blocked, 'recover': unresolved, 'key': key}

    # Cut 2: the replacement is submitted and its reply is discarded.
    uart.command('rotate-receipts')
    write_document(uart, DOC, document)
    child = launch(uart)
    candidate = added(document, 'LostReply')
    task = hand(uart, child, 'add', '"LostReply"', candidate)
    lost = apply_cut(uart, child, LOST_REPLY, UNCERTAIN)
    value = state(data, candidate, pending=True, records=1)
    key = retained(value, ids)
    assert lost == {'error': UNCERTAIN, 'task': 0, 'journal': key, 'applied': 0, 'version': 0}, lost
    committed = receipt(value, key, candidate.encode('ascii'), ids['journal'])
    # Discarding the reply unbinds this child's file client, and the child holds
    # no supervisor authority, so it can never rebind itself: it cannot even read
    # its own record any more (crates/tasks-client/src/acceptance.rs, `discard`).
    assert query(uart, child, UNAVAILABLE)['pending'] == 0
    cleanup(uart, child)
    successor = launch(uart)
    assert query(uart, successor)['pending'] == key
    resolved = recover(uart, successor)
    assert resolved == {'error': 0, 'recovered': 1, 'journal': key, 'task': task,
                        'version': committed}, resolved
    after = state(data, candidate, records=1)
    # Only the intent cleanup commits: recovery never repeats the target effect.
    assert after['nodes'][ids['document']]['version'] == committed, 'recovery resubmitted the effect'
    absent = recover(uart, successor)
    assert absent == {'error': 0, 'recovered': 0, 'journal': 0, 'task': 0, 'version': 0}, absent
    gone = step(uart, successor, f'tasks-owner-forget {successor} {key}', JOURNAL_CODE)
    assert gone['error'] == JOURNAL_CODE, gone
    cleanup(uart, successor)
    observed['lost_reply'] = {'apply': lost, 'recover': resolved, 'absent': absent,
                              'key': key, 'committed': committed}

    # Cut 3: the retention is written but its acknowledgement is lost.
    uart.command('rotate-receipts')
    write_document(uart, DOC, document)
    child = launch(uart)
    candidate = added(document, 'LostJournal')
    hand(uart, child, 'add', '"LostJournal"', candidate)
    lost_journal = apply_cut(uart, child, LOST_JOURNAL, UNCERTAIN)
    # A lost retention acknowledgement stops before target submission and leaves
    # the client unable to name the key it may have written.
    assert lost_journal == {'error': UNCERTAIN, 'task': 0, 'journal': 0,
                            'applied': 0, 'version': 0}, lost_journal
    _, value = snapshot(data)
    assert value['files'][(4, DOC)] == document.encode('ascii'), 'a lost journal reply reached the target'
    assert not value['records'], 'a lost journal reply consumed a receipt'
    journal_bytes = value['files'][(3, JOURNAL_NAME)]
    assert query(uart, child, UNAVAILABLE)['pending'] == 0
    cleanup(uart, child)
    successor = launch(uart)
    pending = query(uart, successor)['pending']
    if journal_bytes:
        # The write was submitted, so the record exists even though its author
        # never learned that it does.
        assert journal_bytes.startswith(MAGIC), 'the record is not an owner intent'
        key = retained(value, ids)
        assert pending == key != 0, (pending, key)
        assert step(uart, successor, f'tasks-owner-forget {successor} {key}')['error'] == 0
    else:
        key = 0
        assert pending == 0, pending
    state(data, document, records=0)
    cleanup(uart, successor)
    observed['lost_journal'] = {'apply': lost_journal, 'retained': bool(journal_bytes), 'key': key}

    # Cut 4: foreign bytes reach the target first, so the submission conflicts.
    uart.command('rotate-receipts')
    write_document(uart, DOC, document)
    child = launch(uart)
    hand(uart, child, 'add', '"Conflicted"', added(document, 'Conflicted'))
    conflicted = apply_cut(uart, child, CONFLICT, VERSION)
    assert conflicted['applied'] == 0 and conflicted['version'] == 0, conflicted
    # The intent was retained before the conflicting write, so the reply names it
    # even though Version is conclusive and the record was cleared again.
    assert conflicted['journal'] != 0, conflicted
    state(data, 'human edit survives', records=0)
    assert query(uart, child)['phase'] == FINISHED

    # No build implements a fifth cut; the supervisor refuses it before delivery.
    before = query(uart, child)
    failure(uart, f'tasks-owner-apply-cut {child} {UNIMPLEMENTED}', 'service invalid request')
    assert query(uart, child) == before, 'a refused selector changed the child'
    cleanup(uart, child)
    observed['conflict'] = conflicted
    observed['unimplemented_selector'] = 'service invalid request'
    write_document(uart, DOC, document)
    results['cuts'] = observed
    return document


def death(uart, data, ids, document, results):
    """Process death during an apply, then resolution through a fresh child.

    Where the kill lands is not controlled, so nothing is assumed: the volume is
    read first and every branch is required to be self-consistent with it.
    """
    attempts = []
    for delay in (0, 0.002, 0.01, 0.04):
        uart.command('rotate-receipts')
        write_document(uart, DOC, document)
        child = launch(uart)
        candidate = added(document, 'Race')
        task = hand(uart, child, 'add', '"Race"', candidate)
        uart.command(f'act {child} tasks-apply', 'actor state=pending')
        if delay:
            time.sleep(delay)
        uart.command(f'kill {child}', 'ok')
        exited(uart, child, 3, 0)
        uart.command(f'reap {child}', 'exit_kind=3 code=0')
        # Killing the client does not cancel the exchange it already submitted:
        # the service still completes it. The successor's own query is ordered
        # after that work, so the volume is only read once it has settled.
        successor = launch(uart)
        observed = query(uart, successor)
        assert observed['phase'] == IDLE, 'a fresh child inherited more than its two objects'
        _, value = snapshot(data)
        journal = value['files'][(3, JOURNAL_NAME)]
        target = value['files'][(4, DOC)].decode('ascii')
        assert target in (document, candidate), \
            'the killed child left bytes that are neither the original nor its plan'
        record = {'delay': delay, 'retained': bool(journal), 'committed': target == candidate,
                  'receipts': len(value['records'])}
        attempts.append(record)
        assert bool(journal) == (observed['pending'] != 0), (observed, record)
        if not journal:
            cleanup(uart, successor)
            continue
        assert journal.startswith(MAGIC), 'the killed child left a record it did not write'
        key = retained(value, ids)
        assert observed['pending'] == key != 0, (observed, key)
        record['key'] = key
        # One unresolved intent blocks the next mutation of this same client.
        hand(uart, successor, 'add', '"Blocked"', added(target, 'Blocked'))
        # The guard refuses before anything is retained, so the reply carries the
        # code alone; the blocking key is read with TASKS_STATUS above.
        blocked = apply(uart, successor, PENDING)
        assert blocked == {'error': PENDING, 'task': 0, 'journal': 0, 'applied': 0, 'version': 0}, blocked
        state(data, target, pending=True, records=record['receipts'])
        uart.command(f'act {successor} tasks-recover', 'actor state=pending')
        resolution = reply_recover(settled(uart, successor))
        record['recovery'] = resolution
        if record['committed']:
            # The effect is on disk, so recovery must match it and clear the record.
            assert resolution == {'error': 0, 'recovered': 1, 'journal': key, 'task': task,
                                  'version': value['nodes'][ids['document']]['version']}, resolution
            state(data, candidate, records=record['receipts'])
        else:
            # Recovery is query-only: an unresolved record stays retained.
            assert resolution['recovered'] == 0 and resolution['error'] != 0, resolution
            state(data, document, pending=True, records=record['receipts'])
            # Forgetting is deliberate and exact: the key must be the retained one
            # (crates/tasks-client/src/journal.rs:148-153).
            # An unresolved intent is not conclusive, so the blocked plan is still
            # held (apps/utility/src/tasks/owner.rs:117-124).
            wrong = step(uart, successor, f'tasks-owner-forget {successor} {key + 1}', PENDING)
            assert wrong['phase'] == READY, wrong
            state(data, document, pending=True, records=record['receipts'])
            forget = step(uart, successor, f'tasks-owner-forget {successor} {key}')
            assert forget['error'] == 0, forget
            state(data, document, records=record['receipts'])
        # With the record resolved there is nothing left to forget.
        absent = step(uart, successor, f'tasks-owner-forget {successor} {key}', JOURNAL_CODE)
        assert absent['error'] == JOURNAL_CODE, absent
        cleanup(uart, successor)
        break
    results['death'] = attempts
    return attempts[-1]


def capacity(uart, data, ids, results):
    """The third tracked write reports the file ABI's own Full, and clears."""
    uart.command('rotate-receipts')
    write_document(uart, DOC, DOCUMENT)
    child = launch(uart)
    document = DOCUMENT
    for index, title in enumerate(('Alpha', 'Beta'), 1):
        document, _ = applied_effect(uart, data, ids, child, document, 'add',
                                     f'"{title}"', title, index)
    overflow = added(document, 'Gamma')
    hand(uart, child, 'add', '"Gamma"', overflow)
    result = apply(uart, child, FULL)
    # The refusal came from the submission, after the intent was retained, so the
    # reply still names the key it retained (apps/utility/src/tasks/report.rs:79-82).
    assert result['applied'] == 0 and result['journal'] != 0 and result['version'] == 0, result
    # Full is conclusive, so the intent this attempt retained was cleared again.
    full = state(data, document, records=2)
    assert all(record['content'] != overflow.encode('ascii') for record in full['records']), \
        'a refused capacity write left a receipt'
    assert query(uart, child)['phase'] == FINISHED
    cleanup(uart, child)
    results['capacity'] = result
    return document


def exercise(uart, data):
    results = {}
    baseline, ids = setup(uart, data)
    child = launch(uart)
    peak = counters(uart)
    assert peak['processes'] > baseline['processes'] and peak['channels'] > baseline['channels']
    idle = uart.command(f'actor-status {child}')
    assert 'actor state=idle status=0 value=0 other=0 control_denied=0 version=0' in idle, idle
    assert query(uart, child) == {'error': 0, 'phase': IDLE, 'cursor': 0, 'total': 0, 'pending': 0}
    document = parity(uart, data, ids, child, results)
    conflict(uart, data, child, document, results)
    sequence(uart, child, results)
    denied(uart, data, child, document, results)
    cleanup(uart, child)
    launch_refusals(uart, data, baseline, results)
    document = cuts(uart, data, ids, document, results)
    # The deterministic cuts carry the coverage; this keeps one realistic case in
    # which nothing chose where the client stopped.
    death(uart, data, ids, document, results)
    document = capacity(uart, data, ids, results)
    # Leave the volume as this suite found it: no retained outcome, no fixture.
    uart.command('rotate-receipts')
    for path in (DOC, PEER, JOURNAL, SHELL_RECORD):
        uart.command(f'rm {path}')
    _, final = snapshot(data)
    assert not final['records'], 'retained outcomes survived the suite'
    assert all(name not in (DOC, PEER, JOURNAL_NAME, 'tasks-intent') for _, name in final['files'])
    assert counters(uart) == baseline, (baseline, counters(uart))
    results.update(verified=True, role='TASKS_OWNER', baseline=baseline,
                   peak_processes=peak['processes'], peak_channels=peak['channels'],
                   final_sequence=final['sequence'], final_document=document)
    return results


def after_reboot(uart, data):
    """The grant, the record location and the protocol survive an actual reboot."""
    baseline = counters(uart)
    write_document(uart, DOC, DOCUMENT)
    uart.command(f'touch {JOURNAL}')
    ids = {'document': stat(uart, DOC)['id'], 'journal': stat(uart, JOURNAL)['id']}
    uart.command('rotate-receipts')
    child = launch(uart)
    assert query(uart, child) == {'error': 0, 'phase': IDLE, 'cursor': 0, 'total': 0, 'pending': 0}
    document, result = applied_effect(uart, data, ids, child, DOCUMENT, 'add',
                                      '"After reboot"', 'After reboot', 1)
    cleanup(uart, child)
    uart.command('rotate-receipts')
    for path in (DOC, JOURNAL):
        uart.command(f'rm {path}')
    assert counters(uart) == baseline, (baseline, counters(uart))
    return {'verified': True, 'applied': result, 'document': document}


def ready(uart):
    """Waits for the file service the shell starts asynchronously.

    The shell prints `Starting files; Ctrl-C keeps owner control available.` and
    offers its prompt without waiting, so a harness must confirm the service is
    mounted instead of assuming its first command has one to talk to. The wait is
    bounded and never restarts anything: a service that does not mount is a
    failure to report, not a condition to work around.
    """
    started = time.monotonic()
    while True:
        uart.send(b'services\r')
        output = uart.until()
        if 'mounted' in output:
            return {'mounted': True, 'seconds': round(time.monotonic() - started, 3)}
        if time.monotonic() - started > 15:
            raise AssertionError(output)
        time.sleep(.1)


def normal(uart, data):
    """Ordinary image: no cut exists at all, and the hand-off still works.

    This is the counterpart of the acceptance run: it proves the failure cuts are
    absent from an ordinary build rather than merely unused by it.
    """
    startup = ready(uart)
    baseline, ids = setup(uart, data)
    unknown = 'unknown command; type help'
    # Both acceptance commands are compiled in by the `tasks-acceptance` feature
    # alone (apps/shell/src/commands/mod.rs:101,127), so an ordinary shell does
    # not have them: the refusal is "unknown command", not an invalid argument.
    failure(uart, 'tasks-owner-apply-cut 4 1', unknown)
    failure(uart, f'tasks-write-acceptance prepared add {DOC} "Never submitted"', unknown)
    child = launch(uart)
    assert query(uart, child) == {'error': 0, 'phase': IDLE, 'cursor': 0, 'total': 0, 'pending': 0}
    document, add = applied_effect(uart, data, ids, child, DOCUMENT, 'add',
                                   '"Ship Rust"', 'Ship Rust', 1)
    document, done = applied_effect(uart, data, ids, child, document, 'done', '7', '7', 2)
    cleanup(uart, child)
    uart.command('rotate-receipts')
    for path in (DOC, PEER, JOURNAL):
        uart.command(f'rm {path}')
    assert counters(uart) == baseline, (baseline, counters(uart))
    return {'verified': True, 'rejected': {'tasks-owner-apply-cut': unknown,
                                           'tasks-write-acceptance': unknown},
            'startup': startup, 'add': add, 'done': done, 'document': document,
            'baseline': baseline}


def verify_normal(image, output):
    """One ordinary-image boot on its own disposable volume.

    The image must not carry the acceptance profile: the point of this run is
    that the cuts do not exist in it. It must also be an initializing mode, since
    the volume is fresh: `mode=terminal` mounts an existing filesystem and its
    file service exits with `Error::Empty` (code 15) on a blank one, while
    `mode=terminal-init` formats it (kernel/src/boot/mode.rs:32-36).
    """
    image, output = Path(image).resolve(), Path(output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    metadata = json.loads((image.parent / 'image.json').read_text())
    if metadata.get('tasks_acceptance'):
        raise ValueError('the normal-image run requires an image without the acceptance profile')
    with tempfile.TemporaryDirectory(prefix='rustic-tasks-owner-normal-') as directory:
        directory = Path(directory)
        with disk(directory / 'data.raw', True) as data, \
                preserve_failure(data, output, 'tasks-owner-normal', metadata):
            sock = directory / 'uart.sock'
            with machine(image, data, f'unix:{sock},server=on,wait=off', output / 'qemu.log') as vm:
                uart = Connection(sock, vm, output / 'serial.log', 30)
                try:
                    uart.until()
                    result = normal(uart, data)
                    uart.send(b'exit\r')
                    uart.until(b'RUSTIC TERMINAL stopped=1 reclaimed=1')
                    assert vm.wait(timeout=10) == 33
                finally:
                    uart.close()
            _, final = snapshot(data)
    evidence = {'verified': True, 'boots': 1, 'build_id': metadata['build_id'],
                'kernel_sha256': metadata['kernel_sha256'],
                'tasks_acceptance': bool(metadata.get('tasks_acceptance')),
                'result': result, 'final_sequence': final['sequence']}
    (output / 'evidence.json').write_text(json.dumps(evidence, separators=(',', ':')) + '\n')
    print(json.dumps(evidence))
    return evidence


def verify(image, output):
    """Runs the suite on its own disposable two-boot volume.

    The mutations here consume retained outcome slots, so they use a separate
    volume instead of the terminal's existing recovery evidence. The deterministic
    cuts are compiled into the utility by the `tasks-acceptance` feature alone
    (tools/application.py builds `native,tasks-acceptance` for this image), so the
    suite requires that explicit acceptance profile.
    """
    image, output = Path(image).resolve(), Path(output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    metadata = json.loads((image.parent / 'image.json').read_text())
    if metadata.get('tasks_acceptance') is not True:
        raise ValueError('the tasks-owner failure cuts require the explicit acceptance build')
    # The second boot reuses the exact ELF; it never invokes candidate host code.
    mount = package(image.parent / 'kernel.elf', 'terminal', metadata['build_id'], {})
    with tempfile.TemporaryDirectory(prefix='rustic-tasks-owner-') as directory:
        directory = Path(directory)
        with disk(directory / 'data.raw', True) as data, \
                preserve_failure(data, output, 'tasks-owner', metadata):
            for phase, boot in enumerate((image, mount), 1):
                sock = directory / f'uart-{phase}.sock'
                with machine(boot, data, f'unix:{sock},server=on,wait=off',
                             output / f'qemu-{phase}.log') as vm:
                    uart = Connection(sock, vm, output / f'serial-{phase}.log', 30)
                    try:
                        uart.until()
                        if phase == 1:
                            result = exercise(uart, data)
                        else:
                            result['after_reboot'] = after_reboot(uart, data)
                        uart.send(b'exit\r')
                        uart.until(b'RUSTIC TERMINAL stopped=1 reclaimed=1')
                        assert vm.wait(timeout=10) == 33
                    finally:
                        uart.close()
            selected, final = snapshot(data)
            (output / 'files.bin').write_bytes(selected)
    evidence = {'verified': True, 'boots': 2, 'build_id': metadata['build_id'],
                'kernel_sha256': metadata['kernel_sha256'],
                'tasks_acceptance': metadata.get('tasks_acceptance'),
                'result': result, 'final_sequence': final['sequence']}
    (output / 'tasks-owner.json').write_text(json.dumps(evidence, separators=(',', ':')) + '\n')
    print(json.dumps(evidence))
    return evidence
