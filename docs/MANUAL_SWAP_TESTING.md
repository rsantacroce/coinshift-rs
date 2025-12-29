## Manual Swap Testing Guide

This file is a **copy/paste** guide for manually testing swap functionality:
- Setting up a **local custom signet** with **descriptor wallets**
- Starting the **bip300301 enforcer** (connected to the signet node via RPC/ZMQ)
- Setting up the **coinshift sidechain** (propose, activate, fund)
- Starting **coinshift_app** and testing swap operations

Notes:
- This guide uses **descriptor wallets only**.
- This guide follows the same setup pattern as the integration tests.

---

### 0) Set environment variables (paste once per terminal)

```bash
# Bitcoin binaries
export BITCOIN_DIR="/home/parallels/Projects/bitcoin-patched/build/bin"
export BITCOIND="$BITCOIN_DIR/bitcoind"
export BITCOIN_CLI="$BITCOIN_DIR/bitcoin-cli"
export BITCOIN_UTIL="${BITCOIN_UTIL:-$BITCOIN_DIR/bitcoin-util}"
export SIGNET_MINER="${SIGNET_MINER:-/home/parallels/Projects/bitcoin-patched/contrib/signet/miner}"

# Enforcer
export ENFORCER="/home/parallels/Projects/bip300301_enforcer/target/debug/bip300301_enforcer"

# Coinshift
export COINSHIFT_APP="/home/parallels/Projects/coinshift-rs/target/debug/coinshift_app"

# RPC credentials
export RPC_USER="user"
export RPC_PASSWORD="passwordDC"

# Signet configuration
export SIGNET_RPC_PORT="18443"
export SIGNET_P2P_PORT="38333"
export SIGNET_DATADIR="/home/parallels/Projects/coinshift-signet-data"
export SIGNET_WALLET="signetwallet"

# Enforcer gRPC
export ENFORCER_GRPC_PORT="50051"
export ENFORCER_GRPC_ADDR="127.0.0.1:$ENFORCER_GRPC_PORT"
export ENFORCER_GRPC_URL="http://$ENFORCER_GRPC_ADDR"

# ZMQ endpoints
export ZMQ_SEQUENCE="tcp://127.0.0.1:29000"
export ZMQ_HASHBLOCK="tcp://127.0.0.1:29001"
export ZMQ_HASHTX="tcp://127.0.0.1:29002"
export ZMQ_RAWBLOCK="tcp://127.0.0.1:29003"
export ZMQ_RAWTX="tcp://127.0.0.1:29004"

# Signet challenge files
export SIGNET_CHALLENGE_FILE="$SIGNET_DATADIR/.signet_challenge"
export SIGNET_PRIVKEY_FILE="$SIGNET_DATADIR/.signet_privkey"

# Coinshift data directory (create a unique one for manual testing)
export COINSHIFT_DATADIR="/home/parallels/Projects/coinshift-manual-test"
export COINSHIFT_RPC_PORT="8332"
export COINSHIFT_NET_PORT="8333"
```

Create the datadirs:

```bash
mkdir -p "$SIGNET_DATADIR" "$COINSHIFT_DATADIR"
```

---

### 1) Choose a signet challenge (local mining)

For local development where you want to mine blocks yourself, start with the **trivial** challenge (anyone-can-mine):

```bash
# OP_TRUE (anyone-can-mine). Good for local dev.
export SIGNET_CHALLENGE="51"
echo "$SIGNET_CHALLENGE" > "$SIGNET_CHALLENGE_FILE"
```

If you already have a challenge file, load it:

```bash
export SIGNET_CHALLENGE="$(cat "$SIGNET_CHALLENGE_FILE")"
echo "SIGNET_CHALLENGE=$SIGNET_CHALLENGE"
```

---

### 2) Start signet (noconnect + custom challenge)

```bash
# Sanity defaults (helps on clean shells)
: "${SIGNET_DATADIR:?SIGNET_DATADIR is not set (run step 0)}"
: "${SIGNET_CHALLENGE_FILE:="$SIGNET_DATADIR/.signet_challenge"}"
mkdir -p "$SIGNET_DATADIR"

# Load persisted challenge into the environment.
# If this is a clean setup and the file doesn't exist yet, default to OP_TRUE.
if [ -f "$SIGNET_CHALLENGE_FILE" ]; then
  export SIGNET_CHALLENGE="$(cat "$SIGNET_CHALLENGE_FILE")"
else
  export SIGNET_CHALLENGE="51"
  echo "$SIGNET_CHALLENGE" > "$SIGNET_CHALLENGE_FILE"
fi
echo "SIGNET_CHALLENGE=$SIGNET_CHALLENGE"

# Stop an already-running signet node (ignore errors if it's not running)
"$BITCOIN_CLI" -signet -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  stop || true

"$BITCOIND" -signet -noconnect \
  -signetchallenge="$SIGNET_CHALLENGE" \
  -fallbackfee=0.0002 \
  -rpcuser="$RPC_USER" \
  -rpcpassword="$RPC_PASSWORD" \
  -rpcport="$SIGNET_RPC_PORT" \
  -server -txindex -rest \
  -zmqpubsequence="$ZMQ_SEQUENCE" \
  -zmqpubhashblock="$ZMQ_HASHBLOCK" \
  -zmqpubhashtx="$ZMQ_HASHTX" \
  -zmqpubrawblock="$ZMQ_RAWBLOCK" \
  -zmqpubrawtx="$ZMQ_RAWTX" \
  -listen -port="$SIGNET_P2P_PORT" \
  -datadir="$SIGNET_DATADIR" \
  -daemon
```

Wait until RPC is ready:

```bash
"$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  getblockchaininfo
```

---

### 3) Create a descriptor wallet (signet)

```bash
# Descriptor wallet (default in modern Core)
"$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  createwallet "$SIGNET_WALLET" || true
```

---

### 4) Mine initial blocks

Mine 101 blocks (matures coinbase).

If you are using the **trivial** challenge (OP_TRUE), `generatetoaddress` should work:

```bash
ADDR=$("$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  -rpcwallet="$SIGNET_WALLET" \
  getnewaddress)

"$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  generatetoaddress 101 "$ADDR"
```

If `generatetoaddress` doesn't work for your build, use the signet miner:

```bash
NBITS_DEFAULT="1e0377ae"
NBITS="${NBITS:-$NBITS_DEFAULT}"

ADDR=$("$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  -rpcwallet="$SIGNET_WALLET" \
  getnewaddress)

"$SIGNET_MINER" \
  --debug \
  --cli="$BITCOIN_CLI -signet -rpcwait -signetchallenge=$SIGNET_CHALLENGE -rpcuser=$RPC_USER -rpcpassword=$RPC_PASSWORD -rpcport=$SIGNET_RPC_PORT -datadir=$SIGNET_DATADIR -rpcwallet=$SIGNET_WALLET" \
  generate \
  --grind-cmd="$BITCOIN_UTIL grind" \
  --address="$ADDR" \
  --nbits="$NBITS" \
  --set-block-time="$(date +%s)" \
  101
```

---

### 5) Start the enforcer (for signet)

This assumes **signet is running** (step 2). The enforcer connects to the signet node via RPC and ZMQ.

```bash
"$ENFORCER" \
  --node-rpc-addr=127.0.0.1:"$SIGNET_RPC_PORT" \
  --node-rpc-user="$RPC_USER" \
  --node-rpc-pass="$RPC_PASSWORD" \
  --node-zmq-addr-sequence="$ZMQ_SEQUENCE" \
  --serve-grpc-addr "$ENFORCER_GRPC_ADDR" \
  --enable-wallet \
  --wallet-sync-source=disabled
```

**Note:** Keep this running in a separate terminal. The enforcer needs to stay running for the sidechain to work.

---

### 6) Create the sidechain proposal (gRPC)

Submit the proposal payload in `docs/create_sidechain_proposal.json` to the enforcer's gRPC server.

Install `grpcurl` (if not already installed):

```bash
# On Fedora/RHEL
sudo dnf install grpcurl

# On Ubuntu/Debian
sudo apt-get install grpcurl

# On macOS
brew install grpcurl
```

Optional: confirm the gRPC service/method exists (uses server reflection):

```bash
grpcurl -plaintext "$ENFORCER_GRPC_ADDR" list | grep -Ei 'cusf\.mainchain\.v1|WalletService|proposal'
grpcurl -plaintext "$ENFORCER_GRPC_ADDR" describe cusf.mainchain.v1.WalletService
```

Submit the proposal:

```bash
grpcurl -plaintext -d @ "$ENFORCER_GRPC_ADDR" \
  cusf.mainchain.v1.WalletService/CreateSidechainProposal \
  < ../docs/create_sidechain_proposal.json
```

---

### 7) Activate and fund the sidechain

After proposing, you need to activate the sidechain and fund the enforcer. This requires mining additional blocks.

First, mine a block to activate the sidechain:

```bash
ADDR=$("$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  -rpcwallet="$SIGNET_WALLET" \
  getnewaddress)

"$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  generatetoaddress 1 "$ADDR"
```

Then fund the enforcer by sending BTC to the sidechain deposit address. First, get the deposit address from coinshift (see step 8), then send BTC to it.

---

### 8) Start coinshift_app

Start coinshift in a new terminal:

```bash
mkdir -p "$COINSHIFT_DATADIR"

"$COINSHIFT_APP" \
  --datadir "$COINSHIFT_DATADIR" \
  --headless \
  --mainchain-grpc-url "$ENFORCER_GRPC_URL" \
  --net-addr "127.0.0.1:$COINSHIFT_NET_PORT" \
  --rpc-addr "127.0.0.1:$COINSHIFT_RPC_PORT" \
  --log-level info
```

**Note:** Keep this running in a separate terminal.

Wait a few seconds for coinshift to initialize, then set up the wallet:

```bash
# Generate mnemonic
MNEMONIC=$(curl -s -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "generate_mnemonic",
    "params": []
  }' | jq -r '.result')

echo "Generated mnemonic: $MNEMONIC"

# Set seed from mnemonic
curl -s -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 2,
    \"method\": \"set_seed_from_mnemonic\",
    \"params\": [\"$MNEMONIC\"]
  }" | jq '.'
```

---

### 9) Get deposit address and fund the sidechain

Get a deposit address:

```bash
DEPOSIT_ADDR=$(curl -s -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 3,
    "method": "get_new_address",
    "params": []
  }' | jq -r '.result')

echo "Deposit address: $DEPOSIT_ADDR"
```

Send BTC from signet to this address:

```bash
# Send 0.1 BTC (10,000,000 sats) to the deposit address
DEPOSIT_AMOUNT=0.1
TXID=$("$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  -rpcwallet="$SIGNET_WALLET" \
  sendtoaddress "$DEPOSIT_ADDR" "$DEPOSIT_AMOUNT")

echo "Deposit transaction ID: $TXID"
```

Mine a block to confirm the deposit:

```bash
ADDR=$("$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  -rpcwallet="$SIGNET_WALLET" \
  getnewaddress)

"$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  generatetoaddress 1 "$ADDR"
```

BMM (Build-Merge-Mine) a block on the sidechain to process the deposit:

```bash
# Mine a sidechain block (this will process the deposit)
curl -s -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 4,
    "method": "mine",
    "params": [null]
  }' | jq '.'
```

Then mine a mainchain block:

```bash
ADDR=$("$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  -rpcwallet="$SIGNET_WALLET" \
  getnewaddress)

"$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  generatetoaddress 1 "$ADDR"
```

Check your balance:

```bash
curl -s -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 5,
    "method": "balance",
    "params": []
  }' | jq '.'
```

---

### 10) Test swap creation

Create a swap (L2 → L1). First, get addresses:

```bash
# Get L2 recipient address (on sidechain)
L2_RECIPIENT=$(curl -s -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 6,
    "method": "get_new_address",
    "params": []
  }' | jq -r '.result')

echo "L2 recipient: $L2_RECIPIENT"

# Get L1 recipient address (on signet)
L1_RECIPIENT=$("$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  -rpcwallet="$SIGNET_WALLET" \
  getnewaddress)

echo "L1 recipient: $L1_RECIPIENT"
```

Create the swap:

```bash
# Create swap: 0.05 BTC L2 → 0.05 BTC L1 (Signet)
L1_AMOUNT_SATS=5000000  # 0.05 BTC
L2_AMOUNT_SATS=5000000  # 0.05 BTC
SWAP_FEE_SATS=1000

SWAP_RESULT=$(curl -s -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 7,
    \"method\": \"create_swap\",
    \"params\": {
      \"parent_chain\": \"BTC\",
      \"l1_recipient_address\": \"$L1_RECIPIENT\",
      \"l1_amount_sats\": $L1_AMOUNT_SATS,
      \"l2_recipient\": \"$L2_RECIPIENT\",
      \"l2_amount_sats\": $L2_AMOUNT_SATS,
      \"required_confirmations\": 1,
      \"fee_sats\": $SWAP_FEE_SATS
    }
  }")

SWAP_ID=$(echo "$SWAP_RESULT" | jq -r '.result[0]')
SWAP_TXID=$(echo "$SWAP_RESULT" | jq -r '.result[1]')

echo "Swap ID: $SWAP_ID"
echo "Swap TXID: $SWAP_TXID"
```

BMM a block to confirm the swap transaction:

```bash
# Mine sidechain block
curl -s -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 8,
    "method": "mine",
    "params": [null]
  }' | jq '.'

# Mine mainchain block
ADDR=$("$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  -rpcwallet="$SIGNET_WALLET" \
  getnewaddress)

"$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  generatetoaddress 1 "$ADDR"
```

Check swap status:

```bash
curl -s -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 9,
    \"method\": \"get_swap_status\",
    \"params\": {
      \"swap_id\": \"$SWAP_ID\"
    }
  }" | jq '.'
```

---

### 11) Fill the swap (send L1 transaction)

Send BTC on signet to the L1 recipient address to fill the swap:

```bash
# Send L1 transaction to fill the swap
L1_TXID=$("$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  -rpcwallet="$SIGNET_WALLET" \
  sendtoaddress "$L1_RECIPIENT" 0.05)

echo "L1 transaction ID: $L1_TXID"
```

Mine signet blocks to confirm the transaction:

```bash
ADDR=$("$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  -rpcwallet="$SIGNET_WALLET" \
  getnewaddress)

# Mine 3 blocks for confirmations
for i in {1..3}; do
  "$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
    -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
    -datadir="$SIGNET_DATADIR" \
    generatetoaddress 1 "$ADDR"
  echo "Mined block $i"
done
```

Update swap with L1 transaction ID:

```bash
# Convert TXID to hex
L1_TXID_HEX=$(echo -n "$L1_TXID" | xxd -r -p | xxd -p -c 256)

# Or if L1_TXID is already a hex string, use it directly
# L1_TXID_HEX="$L1_TXID"

curl -s -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 10,
    \"method\": \"update_swap_l1_txid\",
    \"params\": {
      \"swap_id\": \"$SWAP_ID\",
      \"l1_txid_hex\": \"$L1_TXID\",
      \"confirmations\": 3
    }
  }" | jq '.'
```

Check swap status again:

```bash
curl -s -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 11,
    \"method\": \"get_swap_status\",
    \"params\": {
      \"swap_id\": \"$SWAP_ID\"
    }
  }" | jq '.'
```

---

### 12) List all swaps

```bash
curl -s -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 12,
    "method": "list_swaps",
    "params": []
  }' | jq '.'
```

---

### 13) Stop everything

```bash
# Stop coinshift (if running in foreground, use Ctrl+C)
# Or if you have a stop RPC method:
curl -s -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 13,
    "method": "stop",
    "params": []
  }' | jq '.'

# Stop enforcer (if running in foreground, use Ctrl+C)
pkill -f "bip300301_enforcer" || true

# Stop signet
"$BITCOIN_CLI" -signet -rpcwait -signetchallenge="$SIGNET_CHALLENGE" \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$SIGNET_RPC_PORT" \
  -datadir="$SIGNET_DATADIR" \
  stop || true
```

---

## Quick Reference: RPC Methods

### Get new address
```bash
curl -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"get_new_address","params":[]}'
```

### Get balance
```bash
curl -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"balance","params":[]}'
```

### List UTXOs
```bash
curl -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"list_utxos","params":[]}'
```

### Create swap
```bash
curl -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "id":1,
    "method":"create_swap",
    "params":{
      "parent_chain":"BTC",
      "l1_recipient_address":"bc1q...",
      "l1_amount_sats":5000000,
      "l2_recipient":"0x...",
      "l2_amount_sats":5000000,
      "required_confirmations":1,
      "fee_sats":1000
    }
  }'
```

### Get swap status
```bash
curl -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "id":1,
    "method":"get_swap_status",
    "params":{"swap_id":"..."}
  }'
```

### Update swap L1 TXID
```bash
curl -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "id":1,
    "method":"update_swap_l1_txid",
    "params":{
      "swap_id":"...",
      "l1_txid_hex":"...",
      "confirmations":3
    }
  }'
```

### List swaps
```bash
curl -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"list_swaps","params":[]}'
```

### Mine block
```bash
curl -X POST "http://127.0.0.1:$COINSHIFT_RPC_PORT" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"mine","params":[null]}'
```

---

## Troubleshooting

### Coinshift RPC not responding
- Check that coinshift_app is running
- Verify the RPC port is correct: `curl http://127.0.0.1:$COINSHIFT_RPC_PORT`
- Check logs for errors

### Enforcer not responding
- Check that the enforcer is running
- Verify gRPC port: `grpcurl -plaintext "$ENFORCER_GRPC_ADDR" list`
- Check that signet is running and accessible

### Swap not appearing
- Make sure you BMM'd a block after creating the swap
- Check swap status with `get_swap_status`
- Verify the swap transaction was included in a block

### Deposit not showing up
- Make sure you BMM'd a block after sending the deposit
- Check UTXOs with `list_utxos`
- Verify the deposit transaction was confirmed on signet
