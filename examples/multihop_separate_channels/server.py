"""
server.py

Response to proxyRelay multihop back from client to server and back.
"""

import os
import sys
import time

proxy_path = os.path.expanduser("~/nexus/proxy_server")

try:
    while True:
        with open(proxy_path, "r+") as proxy:
            while msg := proxy.read():
                # Make sure this message was stamped by the proxy
                if "[Proxy 1]" not in msg:
                    print(f"Failure! {msg}", file=sys.stderr)
                    sys.exit(1)
                payload = msg + "[Server]"
                print(f"msg: {msg}", file=sys.stderr)
                print(f"Wrote: {payload}", file=sys.stderr)
                proxy.write(payload)
                proxy.flush()
        time.sleep(0.25)
except Exception as e:
    print(str(e), file=sys.stderr)
