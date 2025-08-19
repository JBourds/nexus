"""
client.py

Initiate a multi-hope communication chain.
"""

import os
import sys
import time

proxy_path = os.path.expanduser("~/nexus/client_proxy")
display_path = os.path.expanduser("~/nexus/display")

counter = 0
try:
    while True:
        with open(proxy_path, "r+", errors="replace") as proxy:
            # `display` is just so the final reads from the client appear with
            # any bit errors which happened on the return trip from the proxy
            with open(display_path, "r+", errors="replace") as display:
                while msg := proxy.read():
                    display.write(msg)
                    display.flush()
                while msg := display.read():
                    pass
                msg = f"[{counter}]"
                proxy.write(msg)
                proxy.flush()
                counter += 1
        time.sleep(0.25)
except Exception as e:
    print(str(e), file=sys.stderr)
