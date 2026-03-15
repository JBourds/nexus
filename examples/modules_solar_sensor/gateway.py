"""
gateway.py — Collects sensor data and sends ACKs over LoRa.
"""

import os
import time

lora = os.path.expanduser("~/nexus/lora/channel")

seq = 0
while True:
    with open(lora, "r+") as f:
        msg = f.read(4096)
        if msg:
            print(f"GW rx: {msg}")
            f.write(f"ACK:{seq}")
            f.flush()
            seq += 1
    time.sleep(0.25)
