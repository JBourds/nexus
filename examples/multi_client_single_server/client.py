"""
client.py

Each client instance receives a unique number through command line arguments
which allow it to uniquely identify itself. They all write to the same file,
and it is expected the file will preserve message boundaries.
"""

import os
import sys
import time

client_num = int(sys.argv[1])
nexus_sock = os.path.expanduser("~/nexus/direct")

counter = 0
while True:
    with open(nexus_sock, "r+") as infile:
        while infile.read():
            pass
        infile.write(f"[{counter}]: Hello from client {client_num}!")
        counter += 1
