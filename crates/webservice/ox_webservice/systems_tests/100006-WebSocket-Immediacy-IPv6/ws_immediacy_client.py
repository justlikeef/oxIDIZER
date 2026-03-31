"""
Verifies that a WebSocket connection to /ws/ping/ is established immediately
(within MAX_CONNECT_SECONDS) over IPv6, and that the server responds to a ping.

Uses [::1] explicitly to test IPv6 connectivity.
"""

import asyncio
import websockets
import time
import sys
import json

MAX_CONNECT_SECONDS = 2.0


async def test_ws_immediacy_ipv6(port: int):
    uri = f"ws://[::1]:{port}/ws/ping/"
    print(f"Connecting to {uri}")

    start = time.monotonic()
    try:
        async with websockets.connect(uri, open_timeout=MAX_CONNECT_SECONDS + 1) as ws:
            elapsed = time.monotonic() - start
            print(f"Connected in {elapsed:.3f}s")

            if elapsed > MAX_CONNECT_SECONDS:
                print(f"FAILURE: Connection took {elapsed:.3f}s (limit: {MAX_CONNECT_SECONDS}s)")
                sys.exit(1)

            await ws.send("ping")
            response = await asyncio.wait_for(ws.recv(), timeout=5.0)
            data = json.loads(response)
            if data.get("response") != "pong":
                print(f"FAILURE: Unexpected response: {data}")
                sys.exit(1)

            print(f"SUCCESS: IPv6 connected in {elapsed:.3f}s and received pong")
            sys.exit(0)

    except asyncio.TimeoutError:
        elapsed = time.monotonic() - start
        print(f"FAILURE: Connection timed out after {elapsed:.3f}s")
        sys.exit(1)
    except Exception as e:
        elapsed = time.monotonic() - start
        print(f"FAILURE: {e} (after {elapsed:.3f}s)")
        sys.exit(1)


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python3 ws_immediacy_client.py <port>")
        sys.exit(1)
    asyncio.run(test_ws_immediacy_ipv6(int(sys.argv[1])))
