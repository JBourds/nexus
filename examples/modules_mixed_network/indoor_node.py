"""
indoor_node.py — Indoor Wi-Fi sensor, sends readings to the server.
"""

import os
import time

wifi = os.path.expanduser("~/nexus/wifi/channel")

seq = 0
while True:
    with open(wifi, "w") as f:
        f.write(f"INDOOR:{seq}:temp=23.{seq % 10}")
        seq += 1
    time.sleep(0.5)
