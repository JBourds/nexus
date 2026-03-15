"""
server.py

Relay multihop back from client to server and back.
"""

import os
import sys
import time

path = os.path.expanduser("~/nexus/main/channel")

try:
    while True:
        with open(path, "r+") as ch:
            while msg := ch.read():
                print(f"[Server RX]: {msg}")
                # make sure we only ever receive proxied messages
                if msg.endswith("[Proxy 1]"):
                    payload = msg + "[Server]"
                    ch.write(payload)
                    ch.flush()
                elif msg.endswith("[Proxy 2]"):
                    continue
                else:
                    print(f"Failure! {msg}", file=sys.stderr)
                    sys.exit(1)
        time.sleep(0.25)
except Exception as e:
    print(str(e), file=sys.stderr)
