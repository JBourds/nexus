"""
client.py

Initiate a multi-hop communication chain.
"""

import os
import sys
import time

path = os.path.expanduser("~/nexus/main")

counter = 0
try:
    while True:
        with open(path, "r+") as ch:
            while msg := ch.read():
                print(f"[Client RX]: {msg}")
                # make sure we only ever receive proxied messages
                if msg.endswith("[Proxy 1]") or msg.endswith("[Proxy 2]"):
                    continue
                else:
                    print(f"Failure! {msg}", file=sys.stderr)
                    sys.exit(1)
            msg = f"[Client RX ({counter})]"
            ch.write(msg)
            ch.flush()
            counter += 1
        time.sleep(0.25)
except Exception as e:
    print(str(e), file=sys.stderr)
