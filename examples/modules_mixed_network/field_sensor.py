"""
field_sensor.py — Remote field sensor, sends readings over LoRa.
"""

import os
import time

lora = os.path.expanduser("~/nexus/lora")

seq = 0
while True:
    with open(lora, "w") as f:
        f.write(f"FIELD:{seq}:soil_moisture={30 + seq % 40}")
        seq += 1
    time.sleep(0.8)
