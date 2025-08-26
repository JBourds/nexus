"""
peripheral.py

Script simulating a peripheral node in a TDMA protocol.
"""

import os
import sys
import time

radio_path = os.path.expanduser("~/nexus/radio")
client_id = int(sys.argv[1])

SLOT_LENGTH = 1
GUARD_LENGTH = 0.1

MS = 0.001


def sleep_until(deadline: float):
    time.sleep(max(0, deadline - time.time()))


if __name__ == "__main__":
    counter = 0
    with open(radio_path, "r+", errors="replace") as conn:
        while True:
            # 1. Wait for a sync message by gateway
            while "Window" not in (msg := conn.read()):
                time.sleep(MS)

            # 2. Wait for turn and check that every other slot gets sent
            start = int(msg.split()[1])
            my_slot = start + (client_id - 1) * SLOT_LENGTH
            sleep_until(my_slot)

            # 3. My turn! Send a message.
            time.sleep(GUARD_LENGTH)
            msg = f"[Client {client_id}][{counter}]"
            conn.write(msg)
            conn.flush()
            counter += 1
