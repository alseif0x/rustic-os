# SPDX-License-Identifier: Apache-2.0
"""Validate exported cause-aware observations; this checker does not run a VM."""
import re

CAUSE = dict(none=0, unknown=1, requested=2, version_conflict=3, authority_lost=4)


def view(value, state, cause):
    assert type(value['profile']) is int and type(value['terminal']) is int, 'noninteger observation fields'
    assert value['profile'] == 2 and value['kind'] == 'retained', 'wrong observation profile/variant'
    identity = re.fullmatch(r'ad_([0-9a-f]{32})_([0-9a-f]{16})', value['id'])
    instance = re.fullmatch(r'si_([0-9a-f]{32})_([0-9a-f]{16})', value['instance'])
    assert identity and instance and int(identity[1], 16) and identity[1] == instance[1], 'invalid lineage'
    assert 0 < int(instance[2], 16) <= int(identity[2], 16), 'invalid originating instance'
    assert value['state'] == state and value['prevention'] == cause, 'wrong retained cause/state'
    assert (state == 'cancelled') == (cause != 'none'), 'cause contradicts state'
    assert value['terminal'] == 0 if state == 'admitted' else value['terminal'] > int(identity[2], 16), 'invalid terminal sequence'
    assert set(value) == {'profile','kind','id','instance','state','terminal','prevention'}, 'mixed live and retained fields'


def paired(group, states, causes):
    assert len(group['views']) == len(group['clients']) == len(states) == len(causes), 'missing native comparison'
    for v, client, state, cause in zip(group['views'], group['clients'], states, causes, strict=True):
        view(v, state, cause)
        assert all(type(v) is int for v in client.values()), 'noninteger client report'
        assert client == dict(status=0, value=dict(admitted=1,cancelled=2,committed=3)[state],
                              other=v['terminal'],control_denied=0,version=CAUSE[cause]), 'clients disagree'


def verify(report):
    assert type(report['format']) is int and type(report['replay_writes']) is int
    assert report['verified'] is True and report['format'] == 5 and report['replay_writes'] == 0
    assert report['legacy_cause'] == 'unknown' and report['reasons'] == ['requested','version_conflict']
    observations = report['observations']
    paired(observations['legacy'], ['cancelled'], ['unknown'])
    paired(observations['migrated'], ['cancelled'], ['unknown'])
    assert observations['legacy'] == observations['migrated'], 'migration relabeled a cause'
    paired(observations['prepared'], ['admitted','admitted'], ['none','none'])
    paired(observations['terminal'], ['cancelled','cancelled'], report['reasons'])
    paired(observations['reboot'], ['cancelled','cancelled'], report['reasons'])
    assert observations['terminal'] == observations['reboot'], 'reboot changed a cause'
    terminal = observations['terminal']['views']
    assert len(report['results']) == 2 and terminal[0]['id'] != terminal[1]['id'], 'aliased results'
    for before, after, record in zip(observations['prepared']['views'], terminal, report['results'], strict=True):
        assert before['id'] == after['id'] == record['id'] and before['instance'] == after['instance'] == record['instance']
        assert after['terminal'] == record['terminal'] and record['state'] == 'cancelled', 'retained fact mismatch'
        assert int(after['id'].rsplit('_',1)[1],16) == record['number'], 'wrong admission number'
    authority = observations['authority']
    view(authority['retained'], 'cancelled', 'authority_lost')
    paired(authority['paired'], ['cancelled'], ['authority_lost'])
    assert authority['paired']['views'] == [authority['retained']]
    live = authority['live']
    assert all(type(live[k]) is int for k in ('profile','pending','requested'))
    assert all(type(v) is int for v in authority['live_client'].values())
    assert live == dict(profile=2,kind='active',id=authority['retained']['id'],
                        instance=authority['retained']['instance'],phase='running',pending=1,requested=0), 'speculative live cause'
    assert authority['live_client'] == dict(status=0,value=17,other=0,control_denied=1,version=0)
    for name, status in [('cancel_only',17),('revoked',18)]:
        assert all(type(v) is int for v in authority[name].values())
        assert authority[name] == dict(status=status,value=0,other=0,control_denied=0,version=0), 'denial disclosed data'
    ids = [observations['legacy']['views'][0]['id'], live['id'], *(v['id'] for v in terminal)]
    assert len(set(ids)) == 4, 'unrelated lifecycle facts alias'
    return True
