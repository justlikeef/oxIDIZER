# ox_cc: Command and Control System

ox_cc is a secure, configuration-driven Command and Control (C2) system designed for managing remote nodes in the oxIDIZER ecosystem. It provides a robust mechanism for distributing signed and encrypted configuration manifests and executing remote commands with strong integrity and confidentiality guarantees.

## Components

The system consists of several specialized crates:

- **[ox_cc_client](../ox_cc_client)**: The daemon that runs on managed nodes. It polls for manifests, verifies them, and applies configurations or executes commands.
- **[ox_cc_manifest_plugin](../ox_cc_manifest_plugin)**: A server-side plugin for `ox_webservice` that stores and serves manifests to clients.
- **[ox_cc_report_plugin](../ox_cc_report_plugin)**: A server-side plugin for `ox_webservice` that collects and stores status reports from clients.
- **[ox_cc_executor](../ox_cc_executor)**: The engine used by the client to execute sets of commands (e.g., package installation, file downloads).
- **[ox_cc_common](../ox_cc_common)**: Shared data structures, serialization logic, and cryptographic primitives used across the system.
- **[ox_cc_keygen](../ox_cc_keygen)**: A utility for generating Ed25519 signing keys and X25519 encryption keys.

## Documentation

- **[Architecture](ARCHITECTURE.md)**: Deep dive into the system design, security model, and communication protocols.
- **[Administration](ADMINISTRATION.md)**: Guide for setting up, enrolling clients, and managing manifests.

## Key Features

- **End-to-End Security**: Manifests are signed by a broker and encrypted for a specific client.
- **Multi-Key Support**: Allows for multiple signing keys to authorize manifests.
- **Atomic Application**: Manifests are written to disk atomically to prevent partial configurations.
- **Extensible Commands**: Supports built-in commands (install, download, log) and external plugins.
- **Status Reporting**: Clients report back the outcome of manifest applications and command executions.
