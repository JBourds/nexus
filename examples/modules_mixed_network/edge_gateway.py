"""
edge_gateway.py — Bridges LoRa to UART.

Receives field sensor data on the LoRa channel and forwards it
over the wired UART link to the server.
"""

import os
import time

lora = os.path.expanduser("~/nexus/lora")
uart = os.path.expanduser("~/nexus/uart")

while True:
    with open(lora, "r") as f:
        msg = f.read()
    if msg:
        print(f"Edge GW rx (LoRa): {msg}")
        with open(uart, "w") as f:
            f.write(f"FWD:{msg}")
    time.sleep(0.25)
