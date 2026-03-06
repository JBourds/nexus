import os
import time

radio = os.path.expanduser("~/nexus/radio")
motion = os.path.expanduser("~/nexus/ctl.pos.motion")

# Move linearly to (20, 10, 0) over 50 seconds (50_000_000 µs)
with open(motion, "r+") as f:
    f.write("linear 20 10 0 50000000")

seq = 0
while True:
    with open(radio, "r+") as f:
        msg = f.read()
        if msg:
            print(f"Scout rx: {msg}")
        f.write(f"SCOUT:{seq}")
        seq += 1

    time.sleep(0.8)
