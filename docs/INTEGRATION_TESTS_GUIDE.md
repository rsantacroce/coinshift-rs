# Integration Tests Guide

This guide explains how to run the swap creation integration tests and the order of operations.

## Overview

The integration tests verify swap creation functionality end-to-end, including:
- Creating fixed swaps (with l2_recipient)
- Creating open swaps (without l2_recipient)
- Filling and claiming open swaps
- Validation tests (duplicate swaps, insufficient funds)

## Prerequisites

### 1. Build Required Binaries

Before running tests, ensure all binaries are built:

```bash
# Build coinshift_app
cd /home/parallels/Projects/coinshift-rs
cargo build --bin coinshift_app

# Build bip300301_enforcer (if not already built)
cd /home/parallels/Projects/bip300301_enforcer
cargo build --bin bip300301_enforcer

# Build Bitcoin Core (if not already built)
cd /home/parallels/Projects/bitcoin-patched
./autogen.sh
./configure
make
```

### 2. Set Up Environment Variables

Create or update `integration_tests/example.env`:

```bash
BIP300301_ENFORCER='/home/parallels/Projects/bip300301_enforcer/target/debug/bip300301_enforcer'
BITCOIND='/home/parallels/Projects/bitcoin-patched/build/bin/bitcoind'
BITCOIN_CLI='/home/parallels/Projects/bitcoin-patched/build/bin/bitcoin-cli'
BITCOIN_UTIL='/home/parallels/Projects/bitcoin-patched/build/bin/bitcoin-util'
ELECTRS='/home/parallels/Projects/electrs/target/debug/electrs'
SIGNET_MINER='/home/parallels/Projects/bitcoin-patched/contrib/signet/miner'
COINSHIFT_APP='/home/parallels/Projects/coinshift-rs/target/debug/coinshift_app'
```

Or set the environment variable:
```bash
export COINSHIFT_INTEGRATION_TEST_ENV='integration_tests/example.env'
```

## Running Tests

### Option 1: Using the Script (Recommended)

```bash
cd /home/parallels/Projects/coinshift-rs/docs

# Run all tests
./run_integration_tests.sh

# Run specific test(s)
./run_integration_tests.sh swap_creation_fixed
./run_integration_tests.sh swap_creation_fixed swap_creation_open
```

### Option 2: Manual Execution

```bash
cd /home/parallels/Projects/coinshift-rs

# Build the test binary
cargo build --example integration_tests --manifest-path integration_tests/Cargo.toml

# Run all tests
COINSHIFT_INTEGRATION_TEST_ENV=integration_tests/example.env \
    cargo run --example integration_tests --manifest-path integration_tests/Cargo.toml

# Run specific test
COINSHIFT_INTEGRATION_TEST_ENV=integration_tests/example.env \
    cargo run --example integration_tests --manifest-path integration_tests/Cargo.toml -- \
    --tests swap_creation_fixed
```

## Test Order

The integration tests are **self-contained** - each test:
1. Sets up its own mainchain node
2. Sets up its own enforcer
3. Proposes and activates a sidechain
4. Creates a coinshift_app instance
5. Runs the test scenario
6. Cleans up

**You do NOT need to run the setup scripts** (`1_start_mainchain.sh`, `3_start_enforcer.sh`, etc.) before running integration tests. The tests handle all setup automatically.

However, if you want to run tests **manually** (not using integration tests), you would use the setup scripts in this order:

### Manual Testing Order (Not Required for Integration Tests)

If you want to test manually (outside of integration tests), use these scripts in order:

1. **Start Mainchain**
   ```bash
   ./docs/1_start_mainchain.sh
   ```

2. **(Optional) Start Parentchain** (for swap testing)
   ```bash
   ./docs/2_start_parentchain.sh
   ```

3. **Start Enforcer**
   ```bash
   ./docs/3_start_enforcer.sh
   ```

4. **Create Enforcer Wallet** (if needed)
   ```bash
   ./docs/create_enforcer_wallet.sh ""
   ```

5. **Unlock Enforcer Wallet** (if needed)
   ```bash
   ./docs/unlock_enforcer_wallet.sh ""
   ```

6. **Fund Enforcer Wallet** (if needed for deposits)
   ```bash
   ./docs/fund_enforcer_wallet.sh 1.0
   ```

7. **Mine Blocks** (as needed)
   ```bash
   ./docs/mine_with_enforcer.sh 10
   ```

## Available Tests

### Happy Path Tests

1. **`swap_creation_fixed`**
   - Creates a pre-specified swap (with l2_recipient)
   - Verifies swap details, state, and UTXO locking
   - Verifies swap appears in list_swaps

2. **`swap_creation_open`**
   - Creates an open swap (without l2_recipient)
   - Verifies swap creation and UTXO locking
   - Verifies no l2_recipient is set

3. **`swap_creation_open_fill`**
   - Creates an open swap
   - Simulates L1 transaction detection
   - Claims the swap
   - Verifies state transitions and lock release

### Validation Tests

4. **`swap_creation_duplicate`**
   - Creates a swap
   - Attempts to create the same swap again
   - Verifies duplicate is rejected

5. **`swap_creation_insufficient_funds`**
   - Attempts to create swap with more than available balance
   - Verifies error handling

## Test Output

Tests output detailed logging. You can control log levels by setting:
```bash
export RUST_LOG=debug  # or info, warn, error
```

## Troubleshooting

### "Binary not found"
- Ensure all binaries are built
- Check paths in `example.env`
- Verify environment variables are set

### "Port already in use"
- The integration tests use temporary ports, so this shouldn't happen
- If it does, kill existing processes:
  ```bash
  pkill -f bitcoind
  pkill -f bip300301_enforcer
  pkill -f coinshift_app
  ```

### "Test timeout"
- Some tests may take time to set up infrastructure
- Increase timeout if needed (tests handle this automatically)

### "Swap validation failed"
- Check test logs for specific error messages
- Verify swap validation logic in `lib/state/swap.rs`

## Differences: Integration Tests vs Manual Scripts

| Aspect | Integration Tests | Manual Scripts |
|--------|------------------|----------------|
| Setup | Automatic, per test | Manual, persistent |
| Cleanup | Automatic | Manual |
| Isolation | Each test isolated | Shared state |
| Use Case | CI/CD, regression testing | Manual exploration |
| Speed | Slower (full setup each time) | Faster (reuse setup) |

## Next Steps

After running tests:
- Review test output for any failures
- Check logs for detailed error messages
- Update tests if validation logic changes
- Add new tests for additional scenarios

## Related Documentation

- `SETUP_ORDER.md` - Manual setup guide
- `MANUAL_SETUP_SWAP_REGTEST.md` - Manual swap testing
- `swap_security_implementation_status.md` - Security features

