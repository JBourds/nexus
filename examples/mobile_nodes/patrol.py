import os
import time

radio = os.path.expanduser("~/nexus/radio")
motion = os.path.expanduser("~/nexus/ctl.pos.motion")

# Start circling: center=(0,0,0), radius=15, angular velocity in deg/µs
# 0.0000036 deg/µs ≈ 0.36 deg/100ms ≈ one full circle per 100s
with open(motion, "r+") as f:
    f.write("circle 0 0 0 15 0.0000036")

seq = 0
while True:
    with open(radio, "r+") as f:
        msg = f.read()
        if msg:
            print(f"Patrol rx: {msg}")
        f.write(f"PATROL:{seq}")
        seq += 1

    time.sleep(0.5)
