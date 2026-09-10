# SPDX-License-Identifier: Apache-2.0
"""Actual submitted writes/flushes, abandoned owner reply, retained drain and remount."""
from .cases import pid, counters
from .authority_cases import actor
from .management_cases import start_restart, wait_job
from .oracle import snapshot

def exercise(session, mount, temporary, base, output, owned_disk):
    cases=[]
    for name,skip,committed in (("admitted_data",0,False),("admitted_final_flush",16,True)):
        with owned_disk(temporary/(name+".raw"),True,evidence_name=name+"-reboot") as data:
            with data.open("r+b") as f:f.write(base["bytes"])
            with session(mount,data,name) as uart:
                baseline=counters(uart)
                c=pid(uart,"session hello other")
                h=pid(uart,f"helper {c} hello other")
                actor(uart,c,"read");actor(uart,c,"stage")
                uart.command(f"hold-io {skip} 400","diagnostic armed")
                uart.send(f'replace hello {base["old"]["version"]} {base["retry"]} "after"\r'.encode())
                uart.until(b"RUSTIC IO_OBSERVATION held=1")
                uart.send(b"\x03")
                reply=uart.until()
                assert "error: Uncertain" in reply,reply
                uart.commands+=1
                uart.command("io-status","held=1")
                uart.command("mem","pending_io=1")
                uart.command(f"revoke {c}","access=requested")
                status=uart.command(f"revocation {h}","effects=unknown")
                assert "access=fenced" not in status,status
                job=start_restart(uart)
                pending=uart.command(f"job-status {job}","pending_io=1")
                assert "phase=2" in pending,pending
                uart.command("services","control-pending")
                uart.command("echo owner-during-admitted-write","owner-during-admitted-write")
                wait_job(uart,job)
                uart.command(f"revocation {c}","discarded_staging=unknown effects=recovery-required")
                uart.command(f"actor-status {h}","denied")
                uart.command("cat hello","after" if committed else "before")
                uart.command("cat other","untouched")
                uart.command(f'receipt {base["old"]["id"]} {base["retry"]}',"committed id=" if committed else "OutcomeUnknown")
                assert counters(uart)==baseline
            with session(mount,data,name+"-reboot") as uart:
                uart.command("cat hello","after" if committed else "before")
                uart.command(f'receipt {base["old"]["id"]} {base["retry"]}',"committed id=" if committed else "OutcomeUnknown")
            prefix,state=snapshot(data)
            assert state["files"][(4,"hello")]==(b"after" if committed else b"before")
            assert state["files"][(4,"other")]==b"untouched"
            assert bool(state["records"])==committed
            if committed:
                assert len(state["records"])==1 and state["records"][0]["content"]==b"after"
                record=state["records"][0];assert state["nodes"][record["id"]]["version"]==record["committed"]
            else:
                assert state["nodes"][base["old"]["id"]]["version"]==base["old"]["version"]
            (output/(name+".bin")).write_bytes(prefix)
            cases.append({"case":name,"verified":True,"skip":skip,"committed":committed,
                "real_submission":True,"owner_interrupted_wait":True,"restart_drained_io":True,"sha256":state["selected_sha256"]})
    return cases
