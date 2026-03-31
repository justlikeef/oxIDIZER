"""
Verifies that the ox_webservice_ping module works correctly over WebSocket:
  - Connects to /ws/ping/
  - Sends a text message
  - Receives a JSON response with {"response": "pong"}
"""

import asyncio
import websockets
import sys
import json


async def test_ws_ping(port: int):
    uri = f"ws://127.0.0.1:{port}/ws/ping/"
    print(f"Connecting to {uri}")

    try:
        async with websockets.connect(uri, open_timeout=5) as ws:
            print("Connected")

            await ws.send("ping")
            print("Sent: ping")

            response = await asyncio.wait_for(ws.recv(), timeout=5.0)
            print(f"Received: {response}")

            try:
                data = json.loads(response)
            except json.JSONDecodeError:
                print(f"FAILURE: Response is not valid JSON: {response!r}")
                sys.exit(1)

            if data.get("response") != "pong":
                print(f"FAILURE: Expected {{\"response\": \"pong\"}}, got: {data}")
                sys.exit(1)

            print("SUCCESS: Received pong via WebSocket")
            sys.exit(0)

    except asyncio.TimeoutError:
        print("FAILURE: Timed out waiting for response")
        sys.exit(1)
    except Exception as e:
        print(f"FAILURE: {e}")
        sys.exit(1)


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python3 ws_ping_client.py <port>")
        sys.exit(1)
    asyncio.run(test_ws_ping(int(sys.argv[1])))
