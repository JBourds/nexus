import os

nexus_sock = os.path.expanduser("~/nexus/ideal")

while True:
    with open(nexus_sock, "r") as infile:
        print(infile.read())
