import os
import time

nexus_sock = os.path.expanduser("~/nexus/ideal")

counter = 0
while True:
    with open(nexus_sock, "r+") as infile:
        print(infile.read())
        infile.write(f"[{counter}]: Hello from the client!")
        counter += 1
    time.sleep(1)
