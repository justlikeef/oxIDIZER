# ox_cc Administration Guide

This guide covers the operational aspects of managing an ox_cc deployment.

## Key Management

The system relies on Ed25519 for signing and X25519 for encryption.

### Generating Keys

Use the `ox_cc_keygen` tool to create keys:

```bash
# Generate a broker signing key (keep the .key file secure!)
ox_cc_keygen broker --name production_broker

# Generate a client encryption key for a new node
ox_cc_keygen client --name node_01
```

- **Broker Key**: The `.key` file stays on the management workstation or secure signing server. The `.pub` file must be distributed to all clients.
- **Client Key**: The private key (`.key`) goes to the client node. The public key (`.pub`) must be registered with the Manifest Plugin.

## Client Enrollment

To enroll a new client:
1. Generate the client keypair.
2. Update the client's `client.yaml` with its `client_id` and private key.
3. Place the broker's public signing key in the client's `broker_signing_pubkeys_dir`.
4. Register the client's public encryption key with the server (typically via an out-of-band management process or a dedicated enrollment endpoint if available).

## Manifest Management

A manifest is a JSON object. A typical manifest might look like:

```json
{
  "manifest_id": "v1.2.3",
  "consumer": "network_config",
  "payload": {
    "interface": "eth0",
    "address": "192.168.1.10/24",
    "commandset": [
      {
        "command": "log_info",
        "params": { "msg": "Applying network config" }
      },
      {
        "command": "download",
        "params": { "url": "http://pkg.local/config.sh", "dest": "/tmp/config.sh" }
      },
      {
        "command": "install",
        "params": { "script": "/tmp/config.sh" }
      }
    ]
  }
}
```

### Deploying a Manifest

1. **Sign and Encrypt**: Use the internal signing tools (or a wrapper script) to wrap the manifest in a `WireEnvelope`.
2. **Post to Server**:
   ```bash
   curl -X POST https://cc.example.com/cc/manifest/node_01 \
        -H "Content-Type: application/json" \
        --data-binary @signed_envelope.json
   ```

## Monitoring and Troubleshooting

### Checking Client Status

The Manifest Plugin tracks the last time a client polled:
```bash
curl https://cc.example.com/cc/clients/node_01/status
```

### Reviewing Reports

The Report Plugin stores the results of manifest applications:
```bash
# Get all reports for a client
curl https://cc.example.com/cc/report/node_01

# Get reports for a specific manifest version
curl https://cc.example.com/cc/report/node_01/v1.2.3
```

### Client Logs

On the managed node, the `ox_cc_client` logs to the system journal (if running as a systemd service) or to the configured log output. Look for "manifest applied" or "commandset complete" messages.

## Configuration Reference

### Client (client.yaml)
- `client_id`: Unique identifier for the node.
- `manifest_url`: Base URL of the ox_cc server.
- `poll_interval_secs`: How often to check for updates.
- `db_path`: Local database for state tracking.
- `broker_signing_pubkeys_dir`: Directory containing `.pub` keys of trusted brokers.
- `client_enc_privkey_b64`: The node's private X25519 key.
