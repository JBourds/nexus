"""
server.py

The server will receive all messages at this given link and it is expected for
them to preserve message boundaries.
"""

import os

nexus_sock = os.path.expanduser("~/nexus/ideal")

counter = 0
while True:
    with open(nexus_sock, "r+") as infile:
        msg = infile.read()
        if msg:
            client = 1 if "client 1" in msg else 2
            infile.write(f"[{counter}]: Hello, client {client}! (Received {msg})")
        counter += 1
