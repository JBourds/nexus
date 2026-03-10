"""
server.py — Central server receiving data from UART and Wi-Fi.
"""

import os
import time

uart = os.path.expanduser("~/nexus/uart")
wifi = os.path.expanduser("~/nexus/wifi")

while True:
    with open(uart, "r") as f:
        msg = f.read()
        if msg:
            print(f"Server rx (UART): {msg}")

    with open(wifi, "r") as f:
        msg = f.read()
        if msg:
            print(f"Server rx (Wi-Fi): {msg}")

    time.sleep(0.25)
