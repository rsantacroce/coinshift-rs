# Scripts Overview

This document explains all the shell scripts in the `docs/` folder and when to use each one.

## Quick Reference

### For Integration Tests (Automated Testing)
- **Use:** `run_integration_tests.sh` - Runs all integration tests automatically
- **No other scripts needed** - Tests handle setup internally

### For Manual Testing/Development
- **Use:** Scripts in order: `1_start_mainchain.sh` → `3_start_enforcer.sh` → (wallet scripts) → `mine_with_enforcer.sh`
- **See:** `SETUP_ORDER.md` for detailed order

## Script Categories

### 1. Infrastructure Setup Scripts

#### `1_start_mainchain.sh`
**Purpose:** Start Bitcoin Core regtest node for mainchain  
**When to use:** First step in manual setup  
**Dependencies:** None  
**What it does:**
- Starts bitcoind on port 18443
- Creates mainchain wallet
- Mines 101 blocks to mature coinbase

#### `2_start_parentchain.sh`
**Purpose:** Start Bitcoin Core regtest node for parentchain (swap target)  
**When to use:** Only if testing swaps manually  
**Dependencies:** None (optional)  
**What it does:**
- Starts bitcoind on port 18444 (different from mainchain)
- Creates parentchain wallet
- Mines 101 blocks

#### `3_start_enforcer.sh`
**Purpose:** Start the bip300301_enforcer  
**When to use:** After mainchain is running  
**Dependencies:** `1_start_mainchain.sh`  
**What it does:**
- Starts enforcer connecting to mainchain
- Creates sidechain proposal (if not exists)
- Waits for proposal approval
- Enables wallet functionality

### 2. Wallet Management Scripts

#### `create_enforcer_wallet.sh`
**Purpose:** Create enforcer wallet with mnemonic  
**When to use:** After enforcer is started, if wallet doesn't exist  
**Dependencies:** `3_start_enforcer.sh`  
**Usage:**
```bash
./create_enforcer_wallet.sh ""           # No password
./create_enforcer_wallet.sh "mypassword" # With password
```

#### `unlock_enforcer_wallet.sh`
**Purpose:** Unlock enforcer wallet  
**When to use:** After creating wallet or restarting enforcer  
**Dependencies:** `3_start_enforcer.sh`, wallet must exist  
**Usage:**
```bash
./unlock_enforcer_wallet.sh ""           # No password
./unlock_enforcer_wallet.sh "mypassword" # With password
```

#### `fund_enforcer_wallet.sh`
**Purpose:** Send funds from mainchain wallet to enforcer wallet  
**When to use:** If you need to create deposit transactions  
**Dependencies:** `1_start_mainchain.sh`, `3_start_enforcer.sh`, wallet unlocked  
**Usage:**
```bash
./fund_enforcer_wallet.sh        # Send 1 BTC (default)
./fund_enforcer_wallet.sh 5.0   # Send 5 BTC
```

### 3. Mining Scripts

#### `mine_with_enforcer.sh`
**Purpose:** Mine blocks using the enforcer  
**When to use:** After wallet is created and unlocked  
**Dependencies:** `3_start_enforcer.sh`, wallet unlocked  
**Usage:**
```bash
./mine_with_enforcer.sh        # Mine 1 block
./mine_with_enforcer.sh 5     # Mine 5 blocks
./mine_with_enforcer.sh 5 true # Mine 5 blocks and ACK proposals
```

#### `4_mine_blocks.sh`
**Purpose:** Mine blocks directly on Bitcoin Core (bypass enforcer)  
**When to use:** For initial setup or when you don't need enforcer features  
**Dependencies:** `1_start_mainchain.sh` (and optionally `2_start_parentchain.sh`)  
**Usage:**
```bash
./4_mine_blocks.sh              # Mine 1 block on both chains
./4_mine_blocks.sh mainchain 5  # Mine 5 blocks on mainchain
./4_mine_blocks.sh parentchain 10 # Mine 10 blocks on parentchain
```

### 4. Testing Scripts

#### `run_integration_tests.sh` ⭐ **NEW**
**Purpose:** Run swap creation integration tests  
**When to use:** For automated testing of swap functionality  
**Dependencies:** Binaries must be built, `example.env` configured  
**Usage:**
```bash
./run_integration_tests.sh                           # Run all tests
./run_integration_tests.sh swap_creation_fixed        # Run specific test
./run_integration_tests.sh swap_creation_fixed swap_creation_open  # Run multiple
```

**Note:** This script does NOT require running other setup scripts. Tests handle setup internally.

#### `test_swap.sh` (in `scripts/` folder)
**Purpose:** Manual swap testing via RPC calls  
**When to use:** For manual testing when coinshift_app is already running  
**Dependencies:** coinshift_app running, Bitcoin regtest running  
**Note:** This is a simple RPC test script, not a full integration test

### 5. Utility Scripts

#### `get_raw_transaction.sh`
**Purpose:** Get raw transaction from Bitcoin node  
**When to use:** For debugging or manual transaction inspection  
**Dependencies:** Bitcoin node running

## Workflow Comparison

### Automated Testing Workflow
```bash
# 1. Build binaries
cargo build --bin coinshift_app
# ... build other binaries

# 2. Configure environment
# Edit integration_tests/example.env

# 3. Run tests (everything is automatic)
cd docs
./run_integration_tests.sh
```

### Manual Testing Workflow
```bash
# 1. Start infrastructure
cd docs
./1_start_mainchain.sh
./2_start_parentchain.sh  # Optional, for swaps
./3_start_enforcer.sh

# 2. Set up wallet
./create_enforcer_wallet.sh ""
./unlock_enforcer_wallet.sh ""

# 3. Fund wallet (if needed)
./fund_enforcer_wallet.sh 1.0

# 4. Mine blocks
./mine_with_enforcer.sh 10

# 5. Start coinshift_app manually
cd /home/parallels/Projects/coinshift-rs
cargo run --bin coinshift_app -- --headless

# 6. Test via RPC or GUI
# Use test_swap.sh or GUI
```

## Script Dependencies Graph

```
Integration Tests (run_integration_tests.sh)
└─> No dependencies (self-contained)

Manual Setup:
1_start_mainchain.sh
└─> (no dependencies)

2_start_parentchain.sh
└─> (no dependencies, optional)

3_start_enforcer.sh
└─> Requires: 1_start_mainchain.sh

create_enforcer_wallet.sh
└─> Requires: 3_start_enforcer.sh

unlock_enforcer_wallet.sh
└─> Requires: 3_start_enforcer.sh, wallet exists

fund_enforcer_wallet.sh
└─> Requires: 1_start_mainchain.sh, 3_start_enforcer.sh, wallet unlocked

mine_with_enforcer.sh
└─> Requires: 3_start_enforcer.sh, wallet unlocked

4_mine_blocks.sh
└─> Requires: 1_start_mainchain.sh (and optionally 2_start_parentchain.sh)
```

## When to Use Which Script

| Scenario | Scripts to Use |
|----------|----------------|
| **Run automated tests** | `run_integration_tests.sh` |
| **Manual swap testing** | `1_start_mainchain.sh` → `3_start_enforcer.sh` → wallet scripts → start coinshift_app |
| **Quick mining** | `4_mine_blocks.sh` |
| **Enforcer mining** | `mine_with_enforcer.sh` |
| **Fresh environment** | All setup scripts in order (see `SETUP_ORDER.md`) |
| **Debugging** | Individual scripts as needed |

## Missing Scripts

The following scripts would be useful but don't exist yet:

1. **`stop_all.sh`** - Stop all running services (bitcoind, enforcer, coinshift_app)
2. **`cleanup_all.sh`** - Clean up all data directories
3. **`check_status.sh`** - Check status of all services
4. **`run_single_test.sh`** - Wrapper for running a single integration test with better output

## Environment Variables

Most scripts use these environment variables (set in scripts or `example.env`):

- `BITCOIN_DIR` - Path to Bitcoin binaries
- `BITCOIND` - Path to bitcoind binary
- `BITCOIN_CLI` - Path to bitcoin-cli binary
- `ENFORCER` - Path to bip300301_enforcer binary
- `COINSHIFT_APP` - Path to coinshift_app binary
- `RPC_USER` / `RPC_PASSWORD` - Bitcoin RPC credentials
- Port numbers (MAINCHAIN_RPC_PORT, etc.)

## Port Usage

| Service | Port | Script |
|---------|------|--------|
| Mainchain RPC | 18443 | `1_start_mainchain.sh` |
| Mainchain P2P | 38333 | `1_start_mainchain.sh` |
| Parentchain RPC | 18444 | `2_start_parentchain.sh` |
| Parentchain P2P | 38334 | `2_start_parentchain.sh` |
| Enforcer gRPC | 50051 | `3_start_enforcer.sh` |
| ZMQ (various) | 29000-29004 | `1_start_mainchain.sh` |

Integration tests use **temporary ports** and don't conflict with these.

## Troubleshooting

### "Script not found"
- Ensure you're in the `docs/` directory
- Check script permissions: `chmod +x docs/*.sh`

### "Port already in use"
- Kill existing processes: `pkill -f bitcoind; pkill -f bip300301_enforcer`
- Or use different ports (modify scripts)

### "Wallet already exists"
- Skip `create_enforcer_wallet.sh`
- Just unlock with `unlock_enforcer_wallet.sh`

### "Enforcer not running"
- Run `3_start_enforcer.sh` first
- Check enforcer logs

## Related Documentation

- `SETUP_ORDER.md` - Detailed setup order for manual testing
- `INTEGRATION_TESTS_GUIDE.md` - Guide for running integration tests
- `MANUAL_SETUP_SWAP_REGTEST.md` - Manual swap testing guide

