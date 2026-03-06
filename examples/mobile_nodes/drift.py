import os
import time

radio = os.path.expanduser("~/nexus/radio")
motion = os.path.expanduser("~/nexus/ctl.pos.motion")

# Drift at constant velocity: 0.3 km/s in x, -0.2 km/s in y
# velocity units are distance_unit per microsecond, so:
# 0.3 km/s = 0.0000003 km/µs
with open(motion, "r+") as f:
    f.write("velocity 0.0000003 -0.0000002 0")

seq = 0
while True:
    with open(radio, "r+") as f:
        msg = f.read()
        if msg:
            print(f"Drift rx: {msg}")
        f.write(f"DRIFT:{seq}")
        seq += 1

    time.sleep(0.6)
