"""
relay.py — Forwards any received LoRa message with a relay tag.
"""

import os
import time

lora = os.path.expanduser("~/nexus/lora/channel")

while True:
    with open(lora, "r+") as f:
        msg = f.read(4096)
        if msg:
            print(f"Relay rx: {msg}")
            f.write(f"{msg}|RELAYED")
            f.flush()
    time.sleep(0.25)
