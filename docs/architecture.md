# Architecture

## Overview

Astram is composed of four main runtime components:

- Astram-node: core node, P2P, mining, and HTTP/JSON-RPC services.
- Astram-dns: public node registry for discovery.
- Astram-explorer: web UI indexing data from the node.
- wallet-cli: local wallet and transaction tool.

## Component Responsibilities

### Astram-node

- Maintains the blockchain database and UTXO state.
- Validates PoW using compact target bits with numeric comparison (`hash_u256 < target_u256`).
- Retargets mining difficulty every block from a rolling 30-block timing window toward a 120s target.
- Runs the P2P stack for block and transaction propagation.
- Exposes an HTTP API for basic chain queries and a dashboard.

### Astram-dns

- Accepts node registrations.
- Validates basic reachability constraints.
- Provides a list of best nodes for bootstrapping.

### Astram-explorer

- Reads chain data from the node API.
- Serves a web UI for browsing blocks and transactions.

### wallet-cli

- Generates wallets and manages keys.
- Queries balances and submits transactions to the node.
- Stores wallet config in a user-local JSON file.

## Data Flow

1. wallet-cli submits transactions to the node HTTP API.
2. The node validates and gossips transactions across P2P.
3. Miners produce blocks and broadcast them.
4. The explorer queries the node API to display chain state.
5. The DNS server offers bootstrap and reachability info for peers.

## Configuration Model

- Wallet config: config.json in the user home directory. Used only by wallet-cli.
- Node settings: config/nodeSettings.conf in the release package or working directory.

Key node settings include:

- P2P bind address and port
- HTTP API bind address and port
- DNS server URL
- Data directory

## Storage

- Core chain data is stored in a database under the configured data directory.
- Wallet keys are stored in a JSON file under the wallet path.

## Operational Notes

- The node should run with a reachable P2P port for public DNS registration.
- Default HTTP endpoint binds to localhost.
