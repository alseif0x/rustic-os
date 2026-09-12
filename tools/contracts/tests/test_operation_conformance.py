# SPDX-License-Identifier: Apache-2.0
"""Challenge the host contract validator; synthetic evidence never demonstrates a guest."""
import base64
import copy
import unittest
from tools.contracts.catalog import Catalog
from tools.contracts.operation_fixture import HostOperations
from tools.contracts.operation_conformance import VECTORS, check_entries, host_check, native_check, request
from tools.contracts.validation import ContractError

def entries(catalog):
    backend, result = HostOperations(catalog), []
    for case in VECTORS:
        backend.rotate()
        p = {"workspace":"workspace_a","resource":"file_a","expected_version":backend.version("file_a"),"retry":{"epoch":backend.epoch,"key":"key"},"data":base64.b64encode(bytes([case["byte"]])*case["length"]).decode()}
        operation = backend.call(request("files.replace",p))["result"]
        result.append({"case":case["id"],"workspace":p["workspace"],"resource":p["resource"],"previous_version":p["expected_version"],"epoch":backend.epoch,"key":"key","result":operation,"by_retry":copy.deepcopy(operation),"by_id":copy.deepcopy(operation)})
    return result

class OperationConformance(unittest.TestCase):
    def setUp(self): self.catalog = Catalog()
    def test_good_host_fixture_exercises_the_completed_profile(self):
        result = host_check(self.catalog)
        self.assertEqual(result["replacement_cases"],10)
        self.assertEqual(result["shared_exchanges"],30)
        self.assertFalse(result["guest_execution"])
    def test_bad_backend_digest_or_historical_identity_is_detected(self):
        for corrupt in ("digest", "instance", "workspace"):
            class Broken(HostOperations):
                def call(self, message):
                    value = super().call(message)
                    if "result" in value:
                        if corrupt == "digest": value["result"]["receipt"]["sha256"] = "0"*64
                        elif corrupt == "instance" and message["method"] == "operations.get": value["result"]["service_instance"] = "new_instance"
                        elif corrupt == "workspace": value["result"]["receipt"]["workspace"] = "wrong_workspace"
                    return value
            with self.subTest(corrupt=corrupt), self.assertRaises(ContractError): host_check(self.catalog, Broken(self.catalog))
    def test_missing_duplicate_and_changed_receipt_evidence_is_rejected(self):
        good = entries(self.catalog)
        self.assertEqual(check_entries(self.catalog,good),10)
        mutations = [lambda v:v.pop(), lambda v:v.__setitem__(1,copy.deepcopy(v[0])),
                     lambda v:v[0]["by_id"].__setitem__("service_instance","changed"),
                     lambda v:v[0]["by_retry"]["receipt"]["retry"].__setitem__("key","wrong"),
                     lambda v:v[0]["result"]["receipt"].__setitem__("sha256","0"*64)]
        for mutate in mutations:
            value = copy.deepcopy(good); mutate(value)
            with self.assertRaises(ContractError): check_entries(self.catalog,value)
    def native_evidence(self):
        value = {"verified":True,"boots":62,"kernel_sha256":"a"*64,"cases":[{"case":"old"+str(i),"verified":True} for i in range(13)]}
        value["cases"] += [{"case":"public_activity_"+name,"verified":True,"skip":skip,"rights":rights,
                            "denied":denied,"committed":committed,"status_during_io":True,
                            "reboot_verified":True,"sha256":"b"*64}
                           for name,skip,rights,denied,committed in (("early",0,8,0,False),
                           ("header",15,8,0,True),("flush",16,8,0,True),("inspect_only",0,4,17,True),
                           ("foreign_scope",0,8,27,True))]
        value["cases"].append({"case":"public_activity_failed_drain","verified":True,
                               "uncertain":True,"reboot_verified":True,"sha256":"b"*64})
        value["cases"].append({"case":"public_activity_saturated","verified":True,"staging_full":True,
                               "undrained_client":True,"owner_progress":True,"stopped":True,
                               "committed":False,"reboot_verified":True,"sha256":"b"*64,
                               "discovery":{"files.read":"available","files.replace":"available",
                                            "operations.get":"available"}})
        value["cases"].append({"case":"public_activity_lost_stop","verified":True,"discarded_reply":True,
                               "stopped":True,"stale_reply_rejected":True,"committed":False,
                               "reboot_verified":True,"sha256":"b"*64})
        value["cases"].append({"case":"scoped_operations_lost_reply_namespaces_restart_reboot","verified":True,"exchanges":entries(self.catalog)})
        value["cases"] += [{"case":"scoped_operation_"+name,"verified":True,"committed":name=="final_flush"} for name in ("data","receipt","metadata","header","final_flush")]
        value["cases"] += [{"case":"controlled_operation_"+name,"verified":True,"skip":skip,"committed":skip>=15,
                            "revoked_while_pending":True,"ack_after_settlement":True,"reboot_verified":True}
                           for name,skip in (("data",0),("before_header",14),("header",15),("final_flush",16))]
        return value
    def test_native_handoff_requires_complete_cuts_and_image_identity(self):
        value = self.native_evidence()
        self.assertEqual(native_check(self.catalog,value)["shared_exchanges"],30)
        for field, invalid in (("verified",False),("boots",18),("kernel_sha256","unknown")):
            altered = copy.deepcopy(value); altered[field] = invalid
            with self.assertRaises(ContractError): native_check(self.catalog,altered)
        value["cases"][-1]["committed"] = False
        with self.assertRaises(ContractError): native_check(self.catalog,value)
    def test_native_control_requires_each_boundary_and_settlement_before_acknowledgment(self):
        good = self.native_evidence()
        for field in ("revoked_while_pending", "ack_after_settlement", "reboot_verified"):
            altered = copy.deepcopy(good); altered["cases"][-1][field] = False
            with self.assertRaises(ContractError): native_check(self.catalog, altered)
        for invalid in (None, good["cases"][-2], {**good["cases"][-1], "skip": 15}):
            altered = copy.deepcopy(good); altered["cases"][-1] = invalid
            with self.assertRaises(ContractError): native_check(self.catalog, altered)

    def test_native_activity_rejects_missing_cases_and_contradictory_outcomes(self):
        good = self.native_evidence()
        live = [i for i,c in enumerate(good["cases"]) if c["case"].startswith("public_activity_")]
        for index in live:
            mutations = [("case", "unrelated"), ("reboot_verified", False), ("sha256", "unknown")]
            if good["cases"][index]["case"].endswith("failed_drain"):
                mutations += [("uncertain", False), ("committed", False)]
            elif good["cases"][index]["case"].endswith("lost_stop"):
                mutations += [("discarded_reply", False), ("stale_reply_rejected", False),
                              ("stopped", False), ("committed", True)]
            elif good["cases"][index]["case"].endswith("saturated"):
                mutations += [("staging_full", False), ("undrained_client", False),
                              ("owner_progress", False), ("stopped", False), ("committed", True),
                              ("discovery", {"files.read": "available"})]
            else:
                case = good["cases"][index]
                mutations += [("status_during_io", False), ("skip", 99), ("rights", 15),
                              ("denied", 99), ("committed", not case["committed"])]
            for field, invalid in mutations:
                altered = copy.deepcopy(good); altered["cases"][index][field] = invalid
                with self.subTest(index=index, field=field), self.assertRaises(ContractError):
                    native_check(self.catalog, altered)
        altered = copy.deepcopy(good)
        altered["cases"][live[1]] = copy.deepcopy(altered["cases"][live[0]])
        with self.assertRaises(ContractError): native_check(self.catalog, altered)
        altered = copy.deepcopy(good); altered["boots"] = 60
        with self.assertRaises(ContractError): native_check(self.catalog, altered)

if __name__ == "__main__": unittest.main()
