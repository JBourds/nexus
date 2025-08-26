"""
gateway.py

Script simulating the gateway node in a TDMA protocol.
"""

import os
import time

radio_path = os.path.expanduser("~/nexus/radio")

SLOT_LENGTH = 1
GUARD_LENGTH = 0.1

MS = 0.001
TTL_MS = 100


def sleep_until(deadline: float):
    time.sleep(max(0, deadline - time.time()))


if __name__ == "__main__":
    with open(radio_path, "r+", errors="replace") as conn:
        while True:
            # 1. Sync - Send the start of the window
            window_start = round(time.time()) + SLOT_LENGTH
            msg = f"Window: {window_start}"
            conn.write(msg)
            conn.flush()
            expiration = time.time() + TTL_MS * MS

            # Testing code to ensure correct operation of medium
            # Wait to read own write
            counter = 0
            read_own_write = False
            while time.time() < expiration:
                if conn.read() == msg:
                    read_own_write = True
                    break
                counter += 1
            assert read_own_write, "Gateway: Didn't read own write."
            # Wait until the message should be expired and try again
            # Assume this is an ideal link and transmission time is
            # neglible. Add slight guard period.
            sleep_until(expiration + GUARD_LENGTH)
            assert conn.read() == "", "Gateway: Reading write beyond TTL"

            # 2. Listen to as many nodes as possible that want to talk
            listen_for_next = True
            slot_start = window_start
            while listen_for_next:
                # Pessimistically assume we won't get anything
                listen_for_next = False
                sleep_until(slot_start)
                slot_start += SLOT_LENGTH
                while time.time() < slot_start:
                    msg = conn.read()
                    if msg:
                        print(f'Gateway: Received message: "{msg}"')
                        listen_for_next = True
                        break
