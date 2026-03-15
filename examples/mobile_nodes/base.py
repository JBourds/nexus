import os
import time

radio = os.path.expanduser("~/nexus/radio/channel")

seq = 0
while True:
    with open(radio, "r+") as f:
        msg = f.read()
        if msg:
            print(f"Base rx: {msg}")
        f.write(f"BEACON:{seq}")
        seq += 1

    time.sleep(0.5)
