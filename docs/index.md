# Astram Documentation

This folder contains project documentation for Astram. All documents describe the current codebase behavior — not a separate specification.

## Contents

| File | Description |
|------|-------------|
| [whitepaper.md](whitepaper.md) | System overview, consensus rules, protocol specification, and parameter reference |
| [architecture.md](architecture.md) | Component responsibilities, data flow, storage, and port mapping |
| [security.md](security.md) | Threat model, DoS limits, validation constraints, and hardening checklist |
| [design.md](design.md) | Design principles, configuration UX, API design, and known gaps |
| [kawpow-blake3-pow.md](kawpow-blake3-pow.md) | KawPow-Blake3 PoW algorithm — DAG, memory-hard mixing, ASIC resistance |
| [mining-performance-guide.md](mining-performance-guide.md) | CUDA miner setup, performance tuning, troubleshooting |
| [dapp-rpc-reference.md](dapp-rpc-reference.md) | dApp RPC reference — Public RPC endpoints, wallet API, fee calculation |

## Notes

- Consensus sections reflect target-based PoW (`hash < target`) and damped rolling retargeting for ~120 s blocks using a **24-block** window.
- The miner is a **standalone binary** (`Astram-miner`), separate from the node process.
- Logs are written to rotating daily files; only the last 5 files per service are retained.
- If you need more detail (formal proofs, protocol schemas, or API extensions), open a request and we will extend the docs.
