"""
sensor.py — Periodically sends temperature readings over LoRa.
"""

import os
import time

lora = os.path.expanduser("~/nexus/lora")

seq = 0
while True:
    with open(lora, "w") as f:
        f.write(f"SENSOR:{seq}:temp=21.{seq % 10}")
        seq += 1
    time.sleep(0.5)
