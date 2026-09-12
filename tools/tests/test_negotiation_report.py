# SPDX-License-Identifier: Apache-2.0
"""Synthetic checker tests; these fixtures do not demonstrate guest execution."""
import unittest
from copy import deepcopy
from test_lifecycle_report import projected, ack
from terminal_support.negotiation_report import verify


def fixture():
    def profiles(peer, available=True, client=False):
        values = []
        for method in ('operations.get', 'operations.cancel'):
            item = dict(method=method,version=2,profile=1,availability='available' if available else 'unavailable',
                        responder=peer,context=1,retained=2,tickets=2,active=1,sha256='a'*64)
            if client:
                item['client'] = dict(status=0,value=0x01020201,other=peer,control_denied=1,version=2)
            values.append(item)
        return values
    identity = dict(id='ad_'+'07'*16+'_0000000000000009', instance='si_'+'07'*16+'_0000000000000001')
    after = projected(identity, 'cancelled')
    denial = dict(status=17,value=0,other=0,control_denied=0,version=0)
    return dict(before_upgrade=dict(profiles=profiles(3,False),disk_sha256='a'*64),
                legacy=dict(format=4,profiles=profiles(3,True,True),prepared=projected(identity,'prepared'),terminal=projected(identity,'prevented'),
                            accepted=ack(identity,'requested',True),too_late=ack(identity,'too_late'),
                            inspect_denied=denial.copy(),cancel_denied=denial.copy(),
                            disk=dict(admission=9,state='cancelled',terminal=10,committed=0)),
                restart=dict(before=profiles(3,True,True),after=profiles(4),operation=after,
                             after_operation=dict(operation=after['operation'].copy()),retired_client=7,
                             revoked=dict(denial,status=18),disk_sha256='a'*64),
                reboot=dict(profiles=profiles(3),operation=dict(operation=after['operation'].copy()),disk_sha256='a'*64))


class NegotiationReport(unittest.TestCase):
    def test_uart_echo_is_not_a_second_descriptor_but_duplicate_replies_are_rejected(self):
        from terminal_support.negotiation_cases import profiles
        class Uart:
            duplicate = False
            def command(self, command):
                method = command.split()[1]
                line = f'lifecycle-profile method={method} version=2 profile=1 availability=unavailable responder=47 context=2 retained=2 tickets=2 active=1 sha256={"a"*64}\r\n'
                return command + '\r\n' + line * (2 if self.duplicate else 1) + 'rustic:/workspaces> '
        uart = Uart()
        self.assertEqual(len(profiles(uart, False)), 2)
        uart.duplicate = True
        with self.assertRaises(AssertionError):
            profiles(uart, False)

    def test_profile_transition_restart_and_old_operation_origin(self):
        self.assertTrue(verify(fixture()))

    def test_false_support_mixed_binding_or_missing_client_evidence_is_rejected(self):
        changes = [lambda r:r['before_upgrade']['profiles'][0].update(availability='available'),
                   lambda r:r['legacy']['profiles'][0].update(version=1),
                   lambda r:r['legacy']['profiles'][0].update(profile=2),
                   lambda r:r['legacy']['profiles'][1].update(responder=9),
                   lambda r:r['legacy']['profiles'][0].update(context=0),
                   lambda r:r['legacy']['profiles'][0].update(tickets=3),
                   lambda r:r['legacy']['profiles'][0].update(sha256='b'*64),
                   lambda r:r['legacy']['profiles'][0]['client'].update(control_denied=0),
                   lambda r:r['legacy']['profiles'][0]['client'].update(other=9),
                   lambda r:r['legacy']['profiles'][0].pop('client'),
                   lambda r:r['restart']['after'][0].update(responder=3),
                   lambda r:r['restart']['revoked'].update(other=1),
                   lambda r:r['reboot']['operation']['operation'].update(service_instance='si_'+'07'*16+'_0000000000000008'),
                   lambda r:r['legacy']['accepted'].update(operation={}),
                   lambda r:r['legacy']['disk'].update(prevention='requested'),
                   lambda r:r['legacy']['terminal']['operation'].update(state='cancelled')]
        for mutate in changes:
            report = deepcopy(fixture())
            mutate(report)
            with self.assertRaises((AssertionError,KeyError)):
                verify(report)
