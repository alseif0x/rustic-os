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
        value = {"verified":True,"boots":46,"kernel_sha256":"a"*64,"cases":[{"case":"old"+str(i),"verified":True} for i in range(13)]}
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

if __name__ == "__main__": unittest.main()
