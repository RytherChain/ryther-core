# Ryther Protocol Core

A high-performance asynchronous Layer 1 blockchain implementation in Rust.

## Features

- **Helix DAG Consensus** - Leaderless asynchronous BFT with deterministic post-facto leader election
- **Native Parallel EVM (RytherVM)** - Optimistic parallel execution with MVCC conflict detection
- **Async State Storage (RytherDB)** - Jellyfish Merkle Tree with LRU caching
- **Gossip-based P2P** - Efficient event propagation with reputation scoring
- **Ethereum-compatible RPC** - JSON-RPC API compatible with existing tooling
- **Threshold Encryption (MEV Protection)** - Commit-reveal scheme protecting against front-running

## Architecture

```
ryther-core/
├── types/      - Core data types (events, transactions, validators)
├── crypto/     - BLS12-381 signatures, Threshold Encryption, DKG
├── dag/        - DAG storage with concurrent access
├── consensus/  - Leader election and commit detection
├── execution/  - MVCC state and parallel executor
├── storage/    - Jellyfish Merkle Tree, page store, cache
├── evm/        - Transaction execution engine
├── network/    - P2P gossip and TCP transport
└── node/       - Full node assembly, mempool, RPC
```

## Quick Start

### Build

```bash
cargo build --release
```

### Run Node

```bash
cargo run --bin ryther
```

The node will:
- Listen for P2P connections on port `10000` (default)
- Expose JSON-RPC on port `8646`
- Create a default `config.json` on first run

### Configuration

Edit `config.json` to customize:
- Chain ID and network name
- P2P listen address and bootstrap peers
- RPC endpoints
- Execution parameters
- Validator keys (if running as validator)

### RPC Examples

```bash
# Get node info
curl -X POST http://127.0.0.1:8646 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"ryther_nodeInfo","params":[],"id":1}'

# Get latest block
curl -X POST http://127.0.0.1:8646 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_getBlockByTag","params":["latest", true],"id":1}'
```

## Testing

```bash
# Run all unit tests
cargo test

# Run integration tests
cargo test --test integration_test

# Run benchmarks
cargo bench
```

## RPC Methods

### Ethereum-compatible
- `eth_chainId`
- `eth_blockNumber`
- `eth_gasPrice`
- `eth_getBalance`
- `eth_getTransactionCount`
- `eth_call`
- `eth_estimateGas`
- `eth_getBlockByNumber`
- `eth_getBlockByHash`
- `eth_getTransactionByHash`
- `eth_getTransactionReceipt`
- `eth_getLogs`

### Ryther-specific
- `ryther_nodeInfo` - Node status and statistics
- `ryther_dagInfo` - DAG round information
- `ryther_getPeers` - Connected peers
- `ryther_poolStatus` - Transaction pool status

### Network
- `net_version`
- `net_peerCount`
- `net_listening`

### Web3
- `web3_clientVersion`
- `web3_sha3`

## License

MIT

## Links

- [Whitepaper](../docs/)
- [GitHub](https://github.com/RytherChain/ryther-core)
