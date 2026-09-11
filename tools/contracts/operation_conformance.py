# SPDX-License-Identifier: Apache-2.0
"""Completed replacement and lookup fixture profile; no adapter or general catalog claim."""
import base64
import copy
import hashlib
import json
import re
from pathlib import Path
from .validation import require, validate_exchange

VECTORS = json.loads((Path(__file__).resolve().parents[2] / "contracts/services/v1/fixtures/replace-cases.json").read_text())["cases"]

def request(method, params): return {"version":1,"method":method,"params":params}
def response(method, result): return {"version":1,"method":method,"result":result}

def check_entries(catalog, entries):
    require(isinstance(entries, list) and len(entries) == len(VECTORS), "incomplete operation inventory")
    cases = {v["id"]:v for v in VECTORS}
    seen, operations = set(), set()
    for entry in entries:
        require(isinstance(entry, dict) and set(entry) == {"case","workspace","resource","previous_version","epoch","key","result","by_retry","by_id"}, "invalid operation evidence fields")
        name = entry["case"]
        require(name in cases and name not in seen, "unknown or duplicate operation vector")
        seen.add(name)
        vector = cases[name]; data = bytes([vector["byte"]])*vector["length"]
        params = {"workspace":entry["workspace"],"resource":entry["resource"],"expected_version":entry["previous_version"],"retry":{"epoch":entry["epoch"],"key":entry["key"]},"data":base64.b64encode(data).decode()}
        result = entry["result"]
        validate_exchange(catalog, request("files.replace",params), response("files.replace",result))
        require(result["state"] == "succeeded" and result["effect"] == "committed" and result["cancel_requested"] is False, "not a completed-operation profile")
        require(result["operation_id"] not in operations, "new replacement reused a retained operation ID")
        operations.add(result["operation_id"])
        for lookup, field in (({"workspace":params["workspace"],"retry":params["retry"]}, "by_retry"), ({"operation_id":result["operation_id"]}, "by_id")):
            validate_exchange(catalog, request("operations.get",lookup), response("operations.get",entry[field]))
            require(entry[field] == result, "lookup changed the original operation or receipt")
    return len(seen)

def native_check(catalog, evidence):
    require(isinstance(evidence, dict) and evidence.get("verified") is True and evidence.get("boots") == 46, "incomplete native recovery mission")
    require(isinstance(evidence.get("kernel_sha256"), str) and re.fullmatch(r"[0-9a-f]{64}", evidence["kernel_sha256"]), "missing guest identity")
    cases = evidence.get("cases")
    require(isinstance(cases, list) and len(cases) == 23 and all(isinstance(c, dict) and c.get("verified") is True for c in cases), "missing recovery cases")
    main = [c for c in cases if c.get("case") == "scoped_operations_lost_reply_namespaces_restart_reboot"]
    require(len(main) == 1, "missing scoped operation mission")
    for name in ("data","receipt","metadata","header","final_flush"):
        found = [c for c in cases if c.get("case") == "scoped_operation_"+name]
        require(len(found) == 1 and found[0].get("committed") is (name == "final_flush"), "wrong effect/receipt cut outcome")
    for name, skip in (("data", 0), ("before_header", 14), ("header", 15), ("final_flush", 16)):
        found = [c for c in cases if c.get("case") == "controlled_operation_" + name]
        require(len(found) == 1 and found[0].get("skip") == skip
                and found[0].get("committed") is (skip >= 15)
                and all(found[0].get(field) is True for field in ("revoked_while_pending", "ack_after_settlement", "reboot_verified")),
                "missing or contradictory owner-control settlement evidence")
    count = check_entries(catalog, main[0].get("exchanges"))
    return {"status":"success","backend":"native_uart_operation_evidence","verified_operations":["files.replace","operations.get"],"catalog_operations":len(catalog.entries),"fixture_profile":"completed_operations","replacement_cases":count,"shared_exchanges":count*3,"guest_execution":True,"kernel_sha256":evidence["kernel_sha256"]}

def host_check(catalog, backend=None):
    from .operation_fixture import HostOperations
    backend = backend or HostOperations(catalog)
    entries = []
    for vector in VECTORS:
        backend.rotate()
        data = bytes([vector["byte"]])*vector["length"]
        params = {"workspace":"workspace_a","resource":"file_a","expected_version":backend.version("file_a"),"retry":{"epoch":backend.epoch,"key":"shared_key"},"data":base64.b64encode(data).decode()}
        r = backend.call(request("files.replace",params)); validate_exchange(catalog,request("files.replace",params),r)
        require("result" in r, "fixture unexpectedly rejected replacement")
        result = r["result"]
        retry_query = request("operations.get",{"workspace":"workspace_a","retry":params["retry"]})
        id_query = request("operations.get",{"operation_id":result["operation_id"]})
        entries.append({"case":vector["id"],"workspace":"workspace_a","resource":"file_a","previous_version":params["expected_version"],"epoch":backend.epoch,"key":"shared_key","result":result,"by_retry":backend.call(retry_query)["result"],"by_id":backend.call(id_query)["result"]})
        backend.edit("file_a", b"later edit")
        before = backend.version("file_a")
        require(backend.call(request("files.replace",params)) == r and backend.version("file_a") == before, "retry duplicated effect or recomputed its result")
        changed = copy.deepcopy(params); changed["data"] = base64.b64encode(data+b"x").decode() if len(data)<1024 else base64.b64encode(b"x"*1024).decode()
        require(backend.call(request("files.replace",changed)).get("error",{}).get("code") == "idempotency_conflict", "changed arguments reused a key")
        other = copy.deepcopy(params); other.update(workspace="workspace_b",resource="file_b",expected_version=backend.version("file_b"))
        second = backend.call(request("files.replace",other)); validate_exchange(catalog,request("files.replace",other),second)
        require("result" in second and second["result"]["operation_id"] != result["operation_id"], "workspace retry namespaces alias")
        backend.enabled = False
        require(backend.call(id_query).get("error",{}).get("code") == "access_denied", "revoked inspection still worked")
        backend.enabled = True; backend.restart()
        require(backend.call(id_query).get("result") == result, "restart changed historical service/receipt identity")
        backend.subject = 2
        require(backend.call(id_query).get("error",{}).get("code") == "outcome_unknown", "foreign subject saw another operation")
        backend.subject = 1
    count = check_entries(catalog, entries)
    return {"status":"success","backend":"bounded_host_operation_fixture","implemented_operations":["files.replace","operations.get"],"catalog_operations":len(catalog.entries),"fixture_profile":"completed_operations","replacement_cases":count,"shared_exchanges":count*3,"guest_execution":False}
