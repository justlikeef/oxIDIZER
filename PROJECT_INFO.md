# ox_c_c — Secure Remote Configuration Client

## Objective
Provide a general-purpose secure configuration delivery system for managed hosts.
A signing broker validates and signs configuration payloads; clients fetch, verify,
decrypt, and apply them. The system is designed to ensure that a compromise of the
delivery infrastructure (Manifest instance or network) cannot result in arbitrary
command execution on client hosts.

`arcnition` is the first consumer, but the client, broker, and manifest format are
not exclusive to arcnition and should remain consumer-agnostic at the system level.

## Current Research Status
- **Architecture**: Move from local `conf/` files to signed remote manifests fetched via HTTPS.
- **Security**: Implement a **Third-Party Attestation** model where a separate **Security Broker** signs commands after validating them against a strict policy.
- **Integration**:
    - `oxIDIZER` will host the Admin UI and Reporting Collector as plugins.
    - `arcnition` will be updated to verify signatures on YAML pipelines before execution.

## Key Repositories
- [arcnition](file:///var/repos/arcnition): Agent and Local Management.
- [oxIDIZER](file:///var/repos/oxIDIZER): Web Framework for Admin UI.

## Architecture Summary

Initially, the development will be done in this isolated repository in order to focus the development. Repository isolation should be honored and only changes in this repository should be made unless explicitly allowed.

- **Stack**: Written in Rust.
- **Cryptography**: Ed25519 for signing, X25519 for encryption.
- **Communication Flow**: The agent should call out to the server (pull model).
- **Scope**: This repository contains the client, the broker, and the security broker/signatory verification components.
- **Storage**: Manifests should be stored on oxIDIZER. The security broker should only respond to the minimum amount of queries required for verification.
- **Deployment Design**: There should be a dedicated instance of `ox_webservice` to host the broker, and a separate instance to host and serve the manifest files in addition to the client.