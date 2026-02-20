import socket
import sys
import time

host = sys.argv[1]
port = int(sys.argv[2])
client_id = sys.argv[3]
wait_time = int(sys.argv[4])

print(f"{client_id}: Attempting to connect to {host}:{port}...")
try:
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(5.0) # 5s install timeout
    s.connect((host, port))
    print(f"{client_id}: Connected successfully.")
    
    # Send MQTT Connect Packet (Minimal)
    # len=16, type=1 (Connect)
    # 10 0E 00 04 4D 51 54 54 04 02 00 3C 00 02 69 64
    packet = bytes.fromhex("10 0E 00 04 4D 51 54 54 04 02 00 3C") + len(client_id).to_bytes(2, 'big') + client_id.encode()
    s.sendall(packet)
    print(f"{client_id}: Sent MQTT Connect.")
    
    # Wait for disconnection or timeout
    s.settimeout(float(wait_time))
    try:
        data = s.recv(1024)
        if not data:
            print(f"{client_id}: Connection closed by peer.")
            sys.exit(1)
        else:
            print(f"{client_id}: Received data: {len(data)} bytes.")
            # If we receive data, we loop or just exit? 
            # Ideally we want to see if connection STAYS open. 
            # If server sends ConnAck, it stays open.
            # We should continue waiting.
            # But sleep is bad.
            # We can select? Or just recv again?
            # Simple approach: If packet is ConnAck, good. Then wait more.
            # But simpler: Recv should block until close or timeout.
            # If ConnAck comes, recv returns.
            # We want to wait UNTIL wait_time expires.
            # So simplistic recv(1) might return early.
            # We should loop until time expires?
            # Or just set timeout and ignore data?
            # If ConnAck comes, we print and wait again?
            # Let's keep it simple: Read until timeout or close.
            
            start_time = time.time()
            while (time.time() - start_time) < wait_time:
                 remaining = wait_time - (time.time() - start_time)
                 if remaining <= 0: break
                 s.settimeout(remaining)
                 data = s.recv(1024)
                 if not data:
                     print(f"{client_id}: Connection closed by peer.")
                     sys.exit(1)
            print(f"{client_id}: Finished waiting (Connection sustained).")
            
    except socket.timeout:
        print(f"{client_id}: Finished waiting (Connection sustained).")
    
    s.close()
    
except socket.timeout:
    # If connect timed out
    print(f"{client_id}: Connection timed out (during connect or final check).")
    # If final check, we printed "Finished waiting" in try block? NO.
    # We should distinguish.
    # The inner try-except catches recv timeout.
    # But s.settimeout affects connect too.
    # Wait, I set s.settimeout(5.0) earlier. 
    # I should reset it.
    pass
except ConnectionRefusedError:
    print(f"{client_id}: Connection refused.")
    sys.exit(1) # Treat refused as exit 1 (disconnected)
except OSError as e:
    print(f"{client_id}: Connection failed/closed: {e}")
    sys.exit(1)
except Exception as e:
    print(f"{client_id}: Unexpected error: {e}")
    sys.exit(1)
