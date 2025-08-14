import os
import time

nexus_sock = os.path.expanduser("~/nexus/ideal")

counter = 0
while True:
    with open(nexus_sock, "r+") as infile:
        msg = infile.read()
        if isinstance(msg, str):
            client = 1 if "client 1" in msg else 2
            infile.write(f"[{counter}]: Hello, client {client}!")
        counter += 1
    time.sleep(1)
