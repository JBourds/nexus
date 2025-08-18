"""
client.py

Initiate a multi-hope communication chain.
"""

import os
import sys
import time

proxy_path = os.path.expanduser("~/nexus/client_proxy")

counter = 0
try:
    while True:
        with open(proxy_path, "r+") as proxy:
            while msg := proxy.read():
                print(f"[Client RX]: {msg}")
                if "[Proxy 1][Server][Proxy 2]" not in msg:
                    sys.exit(1)
            msg = f"[{counter}]"
            proxy.write(msg)
            proxy.flush()
            counter += 1
        time.sleep(0.25)
except Exception as e:
    print(str(e), file=sys.stderr)
