"""
gateway.py — Receives relayed sensor data over LoRa.
"""

import os
import time

lora = os.path.expanduser("~/nexus/lora")

while True:
    with open(lora, "r") as f:
        msg = f.read()
        if msg:
            print(f"GW rx: {msg}")
    time.sleep(0.25)
