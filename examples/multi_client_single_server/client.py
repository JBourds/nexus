import os
import sys
import time

client_num = int(sys.argv[1])
nexus_sock = os.path.expanduser("~/nexus/ideal")

counter = 0
while True:
    with open(nexus_sock, "r+") as infile:
        print(infile.read())
        infile.write(f"[{counter}]: Hello from client {client_num}!")
        counter += 1
    time.sleep(1)
