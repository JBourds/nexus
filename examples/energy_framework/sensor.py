import os
import time

uplink = os.path.expanduser("~/nexus/uplink")
downlink = os.path.expanduser("~/nexus/downlink")
ctl_state = os.path.expanduser("~/nexus/ctl.energy_state")

seq = 0
while True:
    # Switch to active, send reading, then back to sleep
    with open(ctl_state, "r+") as f:
        f.write("active")

    with open(uplink, "w") as f:
        f.write(f"SENSOR:{seq}:temp=22.{seq % 10}")
        seq += 1

    # Read any downlink ack
    with open(downlink, "r") as f:
        ack = f.read()
        if ack:
            print(f"Sensor rx: {ack}")

    with open(ctl_state, "r+") as f:
        f.write("sleep")

    time.sleep(0.8)
