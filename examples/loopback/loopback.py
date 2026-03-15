"""
loopback.py

Simulate writing to one's self on the same protocol.
"""

import os
import time

path = os.path.expanduser("~/nexus/ideal/channel")
counter = 0

if __name__ == "__main__":
    with open(path, "r+") as loopfile:
        while True:
            assert (
                loopfile.read() == ""
            ), "Expected there to be no message but found one."
            msg = f"[{counter}]"
            counter += 1
            loopfile.write(msg)
            loopfile.flush()
            time.sleep(1)
            assert (
                found := loopfile.read()
            ) == msg, f"Expected to read {msg} but found {found}"
