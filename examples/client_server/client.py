import os
import time

nexus_sock = os.path.expanduser("~/nexus/ideal")

for _ in range(5):
    with open(nexus_sock, "r") as infile:
        print(infile.read())
    time.sleep(1)
