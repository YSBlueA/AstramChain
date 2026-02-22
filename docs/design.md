# Design

## Design Principles

- Simplicity over cleverness.
- Clear separation of wallet and node settings.
- Minimal runtime dependencies and easy local testing.
- Reasonable defaults with explicit override paths.

## Configuration UX

- wallet-cli manages a user-local config.json for wallet paths and node RPC URL.
- The node reads config/nodeSettings.conf at startup for runtime parameters.
- Settings are text-based and easy to edit without specialized tools.

## API Design

- HTTP API is optimized for local dashboards and basic chain queries.

## Explorer UX

- Explorer is separate from the node process.
- The node remains the single source of truth for chain data.

## Extensibility

- Feature flags enable optional CUDA mining.
- Modular crates allow future refactors without breaking end-user tooling.
- Consensus tuning favors smoothness: target-based PoW plus damped per-block retargeting for stable ~2 minute blocks.

## Known Gaps

- No formal specification document yet.
- Automated configuration validation is minimal.
- Wallet encryption and hardware wallet support are not implemented.
