import os
import time

uplink = os.path.expanduser("~/nexus/uplink")
downlink = os.path.expanduser("~/nexus/downlink")

seq = 0
while True:
    with open(uplink, "r") as f:
        data = f.read()
        if data:
            print(f"GW rx: {data}")

    with open(downlink, "w") as f:
        f.write(f"ACK:{seq}")
        seq += 1

    time.sleep(0.5)
