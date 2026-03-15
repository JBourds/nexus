"""
sensor.py — Solar-powered ESP32 sensor.

Cycles between active and sleep power states.
Sends a reading over LoRa while active, then sleeps to conserve energy.
Reads the energy control file to log remaining charge.
"""

import os
import time

lora = os.path.expanduser("~/nexus/lora/channel")
ctl_state = os.path.expanduser("~/nexus/ctl.energy_state")
ctl_energy = os.path.expanduser("~/nexus/ctl.energy_left")

seq = 0
while True:
    # Wake up
    with open(ctl_state, "r+") as f:
        f.write("active")

    # Send a sensor reading
    with open(lora, "w") as f:
        f.write(f"DATA:{seq}:humidity={40 + seq % 20}")
        seq += 1

    # Check remaining energy (bounded read -- control files don't send EOF)
    with open(ctl_energy, "r") as f:
        energy = f.read(64).strip()
        if energy:
            print(f"Sensor energy: {energy}")

    # Read any response from gateway (bounded read for same reason)
    with open(lora, "r") as f:
        msg = f.read(4096)
        if msg:
            print(f"Sensor rx: {msg}")

    # Go back to sleep
    with open(ctl_state, "r+") as f:
        f.write("deep_sleep")

    time.sleep(1.0)
