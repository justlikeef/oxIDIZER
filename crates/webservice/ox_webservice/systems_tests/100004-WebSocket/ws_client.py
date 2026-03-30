import asyncio
import websockets
import sys
import json

async def test_websocket():
    uri = f"ws://localhost:{sys.argv[1]}/ws/ping/"
    print(f"Connecting to {uri}")
    try:
        async with websockets.connect(uri) as websocket:
            print("Connected")
            # Send a message (pipeline ignores generic messages for ping but requires connection)
            await websocket.send("ping")
            print("Sent ping")
            
            response = await websocket.recv()
            print(f"Received: {response}")
            
            try:
                data = json.loads(response)
                if data.get("response") == "pong":
                    print("SUCCESS: Received pong")
                    sys.exit(0)
                else:
                    print(f"FAILURE: Unexpected response content: {data}")
                    sys.exit(1)
            except json.JSONDecodeError:
                print(f"FAILURE: Response was not JSON: {response}")
                sys.exit(1)
                
    except Exception as e:
        print(f"FAILURE: Connection error: {e}")
        sys.exit(1)

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python3 ws_client.py <port>")
        sys.exit(1)
    asyncio.run(test_websocket())
