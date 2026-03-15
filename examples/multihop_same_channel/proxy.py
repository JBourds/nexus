"""
proxy.py

Relay multihop back from client to server and back.
"""

import os
import re
import sys
import time

path = os.path.expanduser("~/nexus/main/channel")

try:
    while True:
        with open(path, "r+") as ch:
            while msg := ch.read().strip():
                print(f"Received: {msg}")
                # coming back from server
                if msg.endswith("[Server]"):
                    payload = msg + "[Proxy 2]"
                    print(f"Writing {payload}")
                    ch.write(payload)
                    ch.flush()
                # just received by client
                elif re.match(r"\[Client RX \(\d+\)\]$", msg):
                    payload = msg + "[Proxy 1]"
                    print(f"Writing {payload}")
                    ch.write(payload)
                    ch.flush()
                else:
                    print(f"Unknown message: {msg}", file=sys.stderr)
                    sys.exit(1)
        time.sleep(0.25)
except Exception as e:
    print(str(e), file=sys.stderr)
