"""
proxy.py

Relay multihop back from client to server and back.
"""

import os
import sys
import time

client_path = os.path.expanduser("~/nexus/client_proxy")
server_path = os.path.expanduser("~/nexus/proxy_server")

try:
    while True:
        with open(client_path, "r+") as client:
            with open(server_path, "r+") as server:
                while msg := client.read():
                    print(f"Received: {msg}")
                    payload = msg + "[Proxy 1]"
                    server.write(payload)
                    server.flush()
                while msg := server.read():
                    # Make sure this message was stamped by us before
                    if "[Proxy 1][Server]" not in msg:
                        print(f"Failure! {msg}", file=sys.stderr)
                        sys.exit(1)
                    payload = msg + "[Proxy 2]"
                    client.write(payload)
                    client.flush()
        time.sleep(0.25)
except Exception as e:
    print(str(e), file=sys.stderr)
