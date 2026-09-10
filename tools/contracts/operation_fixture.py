# SPDX-License-Identifier: Apache-2.0
"""Small in-memory contract fixture. It never claims disk, IPC or native authority execution."""
import copy
import hashlib
from .validation import ContractError, validate, decode_data
class HostOperations:
    def __init__(self, catalog):
        self.catalog = catalog
        self.files = {"file_a":("workspace_a", "v1", b""), "file_b":("workspace_b", "v1", b"")}
        self.sequence = 1
        self.records = {}
        self.epoch = "epoch1"
        self.enabled = True
        self.subject = 1
        self.incarnation = 1
    def version(self, file): return self.files[file][1]
    def rotate(self):
        self.sequence += 1; self.epoch = "epoch"+str(self.sequence); self.records.clear()
    def restart(self): self.incarnation += 1
    def edit(self, file, data):
        self.sequence += 1
        self.files[file] = (self.files[file][0], "v"+str(self.sequence), data)
    def failure(self, method, code):
        unknown = code in ("expired_epoch","outcome_unknown")
        action = "reconcile" if unknown else "refresh" if code == "version_conflict" else "fix_request" if code in ("invalid_request","idempotency_conflict") else "stop"
        return {"version":1,"method":method,"error":{"code":code,"effect":"unknown" if unknown else "none","next_action":action}}
    def call(self, message):
        method = message["method"]
        try: validate(self.catalog, message, "request")
        except ContractError: return self.failure(method,"invalid_request")
        if not self.enabled: return self.failure(method,"access_denied")
        p = message["params"]
        if method == "operations.get":
            for (subject, workspace, epoch, key), (_, result) in self.records.items():
                if subject == self.subject and (p.get("operation_id") == result["operation_id"] or p.get("workspace") == workspace and p.get("retry") == {"epoch":epoch,"key":key}):
                    return {"version":1,"method":method,"result":copy.deepcopy(result)}
            return self.failure(method,"expired_epoch" if "retry" in p and p["retry"]["epoch"] != self.epoch else "outcome_unknown")
        if method != "files.replace": return self.failure(method,"invalid_request")
        key = (self.subject,p["workspace"],p["retry"]["epoch"],p["retry"]["key"])
        old = self.records.get(key)
        if old:
            if old[0] != p: return self.failure(method,"idempotency_conflict")
            return {"version":1,"method":method,"result":copy.deepcopy(old[1])}
        file = self.files.get(p["resource"])
        if file is None or file[0] != p["workspace"]: return self.failure(method,"access_denied")
        if p["retry"]["epoch"] != self.epoch: return self.failure(method,"expired_epoch")
        if len(self.records) == 2: return self.failure(method,"quota_exceeded")
        if p["expected_version"] != file[1]: return self.failure(method,"version_conflict")
        data = decode_data(p["data"]); self.edit(p["resource"],data)
        result = {"operation_id":"op"+str(self.sequence),"service_instance":"instance"+str(self.incarnation),"state":"succeeded","effect":"committed","cancel_requested":False,
                  "receipt":{"workspace":p["workspace"],"resource":p["resource"],"previous_version":p["expected_version"],"version":self.version(p["resource"]),"size":len(data),"sha256":hashlib.sha256(data).hexdigest(),"retry":copy.deepcopy(p["retry"])}}
        self.records[key] = (copy.deepcopy(p), result)
        return {"version":1,"method":method,"result":copy.deepcopy(result)}
