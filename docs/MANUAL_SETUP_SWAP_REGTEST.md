## Manual setup (regtest mainchain + regtest parentchain for swaps)

This file is a **copy/paste** guide for bringing up:
- a **regtest mainchain** (where the sidechain is activated via BIP300)
- a **regtest parentchain** (for swap transactions - Bob sends coins here)
- the **bip300301 enforcer** (connected to the mainchain regtest node via RPC/ZMQ)
- a complete **Alice & Bob swap flow** demonstrating L2 → L1 swaps

**End Goal**: After completing this guide:
- **Alice** will have parentchain coins (received from Bob's swap payment)
- **Bob** will have sidechain coins (claimed from Alice's swap offer)

Notes:
- This guide uses **descriptor wallets only**.
- Both chains use regtest for easy local testing.
- The mainchain is used for deposits/withdrawals (BIP300 operations).
- The parentchain is used for swap transactions (coinshift operations).

---

### 0) Set environment variables (paste once per terminal)

```bash
export BITCOIN_DIR="/Users/rob/projects/layertwolabs/bitcoin-patched/build/bin"
export BITCOIND="$BITCOIN_DIR/bitcoind"
export BITCOIN_CLI="$BITCOIN_DIR/bitcoin-cli"
export ENFORCER="/Users/rob/projects/layertwolabs/bip300301_enforcer/target/debug/bip300301_enforcer"
export BITCOIN_UTIL="${BITCOIN_UTIL:-$BITCOIN_DIR/bitcoin-util}"
export RPC_USER="user"
export RPC_PASSWORD="passwordDC"

# Mainchain regtest (for sidechain activation)
export MAINCHAIN_RPC_PORT="18443"
export MAINCHAIN_P2P_PORT="38333"
export MAINCHAIN_DATADIR="/Users/rob/projects/layertwolabs/coinshift-mainchain-data"
export MAINCHAIN_WALLET="mainchainwallet"

# Parentchain regtest (for swap transactions)
export PARENTCHAIN_RPC_PORT="18444"
export PARENTCHAIN_P2P_PORT="38334"
export PARENTCHAIN_DATADIR="/Users/rob/projects/layertwolabs/coinshift-parentchain-data"
export PARENTCHAIN_WALLET="parentchainwallet"

# Enforcer
export ENFORCER_GRPC_PORT="50051"
export ENFORCER_GRPC_ADDR="127.0.0.1:$ENFORCER_GRPC_PORT"
export ENFORCER_GRPC_URL="http://$ENFORCER_GRPC_ADDR"

# ZMQ (for mainchain)
export ZMQ_SEQUENCE="tcp://127.0.0.1:29000"
export ZMQ_HASHBLOCK="tcp://127.0.0.1:29001"
export ZMQ_HASHTX="tcp://127.0.0.1:29002"
export ZMQ_RAWBLOCK="tcp://127.0.0.1:29003"
export ZMQ_RAWTX="tcp://127.0.0.1:29004"
```

Create the datadirs:

```bash
mkdir -p "$MAINCHAIN_DATADIR" "$PARENTCHAIN_DATADIR"
```

---

### 1) Start mainchain regtest (for sidechain activation)

This is the regtest chain where the sidechain will be activated via BIP300. It handles deposits and withdrawals.

```bash
# Stop an already-running mainchain node (ignore errors if it's not running)
"$BITCOIN_CLI" -regtest \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$MAINCHAIN_RPC_PORT" \
  -datadir="$MAINCHAIN_DATADIR" \
  stop || true

"$BITCOIND" -regtest \
  -fallbackfee=0.0002 \
  -rpcuser="$RPC_USER" \
  -rpcpassword="$RPC_PASSWORD" \
  -rpcport="$MAINCHAIN_RPC_PORT" \
  -server -txindex -rest \
  -zmqpubsequence="$ZMQ_SEQUENCE" \
  -zmqpubhashblock="$ZMQ_HASHBLOCK" \
  -zmqpubhashtx="$ZMQ_HASHTX" \
  -zmqpubrawblock="$ZMQ_RAWBLOCK" \
  -zmqpubrawtx="$ZMQ_RAWTX" \
  -listen -port="$MAINCHAIN_P2P_PORT" \
  -datadir="$MAINCHAIN_DATADIR" \
  -daemon
```

Wait until RPC is ready:

```bash
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$MAINCHAIN_RPC_PORT" \
  -datadir="$MAINCHAIN_DATADIR" \
  getblockchaininfo
```

---

### 2) Create descriptor wallet (mainchain)

```bash
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$MAINCHAIN_RPC_PORT" \
  -datadir="$MAINCHAIN_DATADIR" \
  createwallet "$MAINCHAIN_WALLET" || true
```

---

### 3) Mine initial blocks on mainchain

Mine 101 blocks to mature coinbase:

```bash
ADDR=$("$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$MAINCHAIN_RPC_PORT" \
  -datadir="$MAINCHAIN_DATADIR" \
  -rpcwallet="$MAINCHAIN_WALLET" \
  getnewaddress)

"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$MAINCHAIN_RPC_PORT" \
  -datadir="$MAINCHAIN_DATADIR" \
  generatetoaddress 101 "$ADDR"
```

---

### 4) Start parentchain regtest (for swap transactions)

This is a separate regtest chain used for swap transactions. Bob will send coins here, and Alice will receive them.

```bash
# Stop an already-running parentchain node (ignore errors if it's not running)
"$BITCOIN_CLI" -regtest \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$PARENTCHAIN_RPC_PORT" \
  -datadir="$PARENTCHAIN_DATADIR" \
  stop || true

"$BITCOIND" -regtest \
  -fallbackfee=0.0002 \
  -rpcuser="$RPC_USER" \
  -rpcpassword="$RPC_PASSWORD" \
  -rpcport="$PARENTCHAIN_RPC_PORT" \
  -server -txindex -rest \
  -listen -port="$PARENTCHAIN_P2P_PORT" \
  -datadir="$PARENTCHAIN_DATADIR" \
  -daemon
```

Wait until RPC is ready:

```bash
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$PARENTCHAIN_RPC_PORT" \
  -datadir="$PARENTCHAIN_DATADIR" \
  getblockchaininfo
```

---

### 5) Create descriptor wallet (parentchain)

```bash
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$PARENTCHAIN_RPC_PORT" \
  -datadir="$PARENTCHAIN_DATADIR" \
  createwallet "$PARENTCHAIN_WALLET" || true
```

---

### 6) Mine initial blocks on parentchain

Mine 101 blocks to mature coinbase:

```bash
ADDR=$("$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$PARENTCHAIN_RPC_PORT" \
  -datadir="$PARENTCHAIN_DATADIR" \
  -rpcwallet="$PARENTCHAIN_WALLET" \
  getnewaddress)

"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$PARENTCHAIN_RPC_PORT" \
  -datadir="$PARENTCHAIN_DATADIR" \
  generatetoaddress 101 "$ADDR"
```

---

### 7) Start the enforcer (for mainchain)

This assumes **mainchain is running** (step 1). The enforcer connects to the mainchain node via RPC and ZMQ.

```bash
"$ENFORCER" \
  --node-rpc-addr=127.0.0.1:"$MAINCHAIN_RPC_PORT" \
  --node-rpc-user="$RPC_USER" \
  --node-rpc-pass="$RPC_PASSWORD" \
  --node-zmq-addr-sequence="$ZMQ_SEQUENCE" \
  --serve-grpc-addr "$ENFORCER_GRPC_ADDR" \
  --enable-wallet \
  --wallet-sync-source=disabled
```

---

### 8) Create the sidechain proposal (gRPC)

Submit the proposal payload in `docs/create_sidechain_proposal.json` to the enforcer's gRPC server.

Install `grpcurl` (macOS):

```bash
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
  < docs/create_sidechain_proposal.json
```

---

### 9) Activate the sidechain

After the proposal is submitted, you need to activate the sidechain. This typically involves:
1. Waiting for the proposal to be included in a block
2. Mining additional blocks to activate it

Check the enforcer logs or use the gRPC API to check proposal status.

---

### 9b) Configure parentchain RPC in coinshift app

**Important**: The coinshift app needs to know how to connect to the parentchain regtest node to monitor swap transactions. You need to configure the RPC settings for `ParentChainType::Regtest` in the coinshift app.

This is typically done via the app's GUI (L1 Config section) or via configuration file. The RPC URL should point to the parentchain regtest node:

- **RPC URL**: `http://127.0.0.1:$PARENTCHAIN_RPC_PORT`
- **RPC User**: `$RPC_USER`
- **RPC Password**: `$RPC_PASSWORD`
- **Parent Chain**: `Regtest`

**Note**: The system uses this RPC configuration to query the parentchain for transactions matching pending swaps when processing 2WPD.

#### 9c) Start the Coinshift Sidechain Node

After configuring the parentchain RPC, start the coinshift node (sidechain app). If you've built the Rust binary, run:

```bash
./target/release/coinshift --network regtest
```

Or, if using a different build path/location, substitute accordingly. The node will listen for swaps and process 2WPD data for the parentchain you configured in the previous step.

You should see logs indicating connection to both mainchain and parentchain regtest nodes, and readiness for swaps.



---

### 10) Alice & Bob Swap Flow

This section demonstrates the complete swap flow where:
- **Alice** has L2 coins and wants parentchain coins
- **Bob** has parentchain coins and wants L2 coins
- After the swap: Alice has parentchain coins, Bob has sidechain coins

#### 10a) Alice deposits to mainchain (gets L2 coins)

Alice needs to deposit mainchain coins to get L2 coins. First, get Alice's deposit address from the sidechain node via RPC, then create a deposit transaction.

**Get Alice's deposit address**:
```bash
# Get the formatted deposit address from the coinshift app
DEPOSIT_ADDRESS=$(curl -X POST "$COINSHIFT_RPC_URL" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 10,
    \"method\": \"format_deposit_address\",
    \"params\": []
  }" | jq -r '.result')

echo "Deposit address: $DEPOSIT_ADDRESS"
```

**Fund the deposit address on mainchain**:
```bash
# Send coins to the deposit address
DEPOSIT_AMOUNT=1.0  # 1 BTC
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$MAINCHAIN_RPC_PORT" \
  -datadir="$MAINCHAIN_DATADIR" \
  -rpcwallet="$MAINCHAIN_WALLET" \
  sendtoaddress "$DEPOSIT_ADDRESS" "$DEPOSIT_AMOUNT"

# Mine a block to confirm the deposit
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$MAINCHAIN_RPC_PORT" \
  -datadir="$MAINCHAIN_DATADIR" \
  generatetoaddress 1 "$DEPOSIT_ADDRESS"
```

**Create the deposit transaction** (this tells the sidechain about the deposit):
```bash
DEPOSIT_AMOUNT_SATS=100000000  # 1 BTC in sats
DEPOSIT_FEE_SATS=1000          # Deposit fee

curl -X POST "$COINSHIFT_RPC_URL" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 11,
    \"method\": \"create_deposit\",
    \"params\": {
      \"amount_sats\": $DEPOSIT_AMOUNT_SATS,
      \"fee_sats\": $DEPOSIT_FEE_SATS
    }
  }"
```

**Mine blocks on mainchain to process the deposit** (BIP300 requires blocks to be mined):
```bash
# Mine additional blocks to process the deposit through BIP300
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$MAINCHAIN_RPC_PORT" \
  -datadir="$MAINCHAIN_DATADIR" \
  generatetoaddress 6 "$DEPOSIT_ADDRESS"
```

**After this step**: Alice should have L2 coins (visible in the sidechain wallet). Check balance:
```bash
curl -X POST "$COINSHIFT_RPC_URL" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 12,
    \"method\": \"get_balance\",
    \"params\": []
  }"
```

#### 10b) Alice creates a swap offer

Alice creates a swap offering L2 coins in exchange for parentchain coins. She generates a parentchain address where she wants to receive the coins.

```bash
# Get Alice's parentchain address (where she wants to receive swap payment)
ALICE_PARENTCHAIN_ADDR=$("$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$PARENTCHAIN_RPC_PORT" \
  -datadir="$PARENTCHAIN_DATADIR" \
  -rpcwallet="$PARENTCHAIN_WALLET" \
  getnewaddress)

echo "Alice's parentchain address (for receiving swap payment): $ALICE_PARENTCHAIN_ADDR"
```

**Create the swap via coinshift app RPC**:

Set the swap parameters:
```bash
# Swap parameters
SWAP_L1_AMOUNT_SATS=5000000   # 0.05 BTC
SWAP_L2_AMOUNT_SATS=10000000  # 0.1 BTC
SWAP_FEE_SATS=1000            # Transaction fee
REQUIRED_CONFIRMATIONS=1       # For regtest, 1 confirmation is sufficient

# Coinshift app RPC endpoint
# Default port is 6000 + sidechain_id (typically 6255 for sidechain_id 255)
# Adjust if you started the app with a different --rpc-addr
COINSHIFT_RPC_URL="http://127.0.0.1:6255"
```

Create an open swap (anyone can fill it):
```bash
curl -X POST "$COINSHIFT_RPC_URL" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 1,
    \"method\": \"create_swap\",
    \"params\": {
      \"parent_chain\": \"Regtest\",
      \"l1_recipient_address\": \"$ALICE_PARENTCHAIN_ADDR\",
      \"l1_amount_sats\": $SWAP_L1_AMOUNT_SATS,
      \"l2_recipient\": null,
      \"l2_amount_sats\": $SWAP_L2_AMOUNT_SATS,
      \"required_confirmations\": $REQUIRED_CONFIRMATIONS,
      \"fee_sats\": $SWAP_FEE_SATS
    }
  }"
```

The response will contain the swap_id and transaction ID:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": [
    "swap_id_hex_string",
    "txid_hex_string"
  ]
}
```

Save the swap_id for later:
```bash
# Extract swap_id from response (adjust based on your curl output format)
SWAP_ID="swap_id_hex_string"  # Replace with actual swap_id from response
echo "Swap ID: $SWAP_ID"
```

After creating the swap, mine a block on the mainchain to include the swap transaction:

```bash
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$MAINCHAIN_RPC_PORT" \
  -datadir="$MAINCHAIN_DATADIR" \
  generatetoaddress 1 "$ALICE_MAINCHAIN_ADDR"
```

**Verify the swap was created**:
```bash
curl -X POST "$COINSHIFT_RPC_URL" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 2,
    \"method\": \"get_swap_status\",
    \"params\": {
      \"swap_id\": \"$SWAP_ID\"
    }
  }"
```

**After this step**: Alice's swap is created and L2 coins are locked in the swap. The swap state should be `Pending`.

#### 10c) Bob sends parentchain coins to Alice

Bob sends parentchain coins to Alice's parentchain address to fill the swap.

```bash
# Get Bob's parentchain address (for funding)
BOB_PARENTCHAIN_ADDR=$("$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$PARENTCHAIN_RPC_PORT" \
  -datadir="$PARENTCHAIN_DATADIR" \
  -rpcwallet="$PARENTCHAIN_WALLET" \
  getnewaddress)

# Fund Bob's address first (if needed)
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$PARENTCHAIN_RPC_PORT" \
  -datadir="$PARENTCHAIN_DATADIR" \
  -rpcwallet="$PARENTCHAIN_WALLET" \
  sendtoaddress "$BOB_PARENTCHAIN_ADDR" 1.0

# Mine a block to confirm Bob's funding
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$PARENTCHAIN_RPC_PORT" \
  -datadir="$PARENTCHAIN_DATADIR" \
  generatetoaddress 1 "$BOB_PARENTCHAIN_ADDR"

# Bob sends coins to Alice's parentchain address
# Adjust the amount to match the swap's l1_amount
SWAP_L1_AMOUNT=0.05  # Example: 0.05 BTC (5000000 sats)
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$PARENTCHAIN_RPC_PORT" \
  -datadir="$PARENTCHAIN_DATADIR" \
  -rpcwallet="$PARENTCHAIN_WALLET" \
  sendtoaddress "$ALICE_PARENTCHAIN_ADDR" "$SWAP_L1_AMOUNT"

# Mine a block to confirm Bob's payment
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$PARENTCHAIN_RPC_PORT" \
  -datadir="$PARENTCHAIN_DATADIR" \
  generatetoaddress 1 "$BOB_PARENTCHAIN_ADDR"
```

**After this step**: Bob's transaction is confirmed on the parentchain. The system should detect this transaction when processing 2WPD.

#### 10d) Validate the swap transaction on parentchain

Check that Bob's transaction exists and is confirmed:

```bash
# Get the transaction details
# First, find the transaction ID (you may need to list transactions or check the block)
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$PARENTCHAIN_RPC_PORT" \
  -datadir="$PARENTCHAIN_DATADIR" \
  -rpcwallet="$PARENTCHAIN_WALLET" \
  listtransactions "*" 100

# Check Alice's balance on parentchain (should have received the swap payment)
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$PARENTCHAIN_RPC_PORT" \
  -datadir="$PARENTCHAIN_DATADIR" \
  -rpcwallet="$PARENTCHAIN_WALLET" \
  getreceivedbyaddress "$ALICE_PARENTCHAIN_ADDR"
```

#### 10e) Trigger 2WPD processing (to detect swap transaction)

The system processes coinshift transactions when the mainchain tip changes. Mine a block on the mainchain to trigger 2WPD processing:

```bash
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$MAINCHAIN_RPC_PORT" \
  -datadir="$MAINCHAIN_DATADIR" \
  generatetoaddress 1 "$ALICE_MAINCHAIN_ADDR"
```

**Note**: The sidechain node should detect Bob's parentchain transaction during 2WPD processing and update the swap state to `ReadyToClaim`.

**Wait a moment for processing, then check swap status**:
```bash
# Wait a few seconds for the system to process the transaction
sleep 2

# Check swap status - should now be ReadyToClaim
curl -X POST "$COINSHIFT_RPC_URL" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 4,
    \"method\": \"get_swap_status\",
    \"params\": {
      \"swap_id\": \"$SWAP_ID\"
    }
  }"
```

The swap state should now be `ReadyToClaim` (or `WaitingConfirmations` if more confirmations are needed).

#### 10f) Bob claims the swap (gets L2 coins)

After the swap is detected and has sufficient confirmations, Bob can claim the L2 coins.

**Get Bob's L2 address** (for open swaps, Bob provides his address when claiming):
```bash
# Bob gets a new L2 address from the coinshift app
# This would typically be done via the app's RPC or GUI
BOB_L2_ADDRESS="bob_l2_address_here"  # Replace with actual L2 address from coinshift app
```

**Claim the swap via coinshift app RPC**:
```bash
curl -X POST "$COINSHIFT_RPC_URL" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 5,
    \"method\": \"claim_swap\",
    \"params\": {
      \"swap_id\": \"$SWAP_ID\",
      \"l2_claimer_address\": \"$BOB_L2_ADDRESS\"
    }
  }"
```

The response will contain the claim transaction ID:
```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "result": "claim_txid_hex_string"
}
```

After claiming, mine a block on the mainchain to include the claim transaction:

```bash
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$MAINCHAIN_RPC_PORT" \
  -datadir="$MAINCHAIN_DATADIR" \
  generatetoaddress 1 "$ALICE_MAINCHAIN_ADDR"
```

**Verify the swap is completed**:
```bash
curl -X POST "$COINSHIFT_RPC_URL" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 6,
    \"method\": \"get_swap_status\",
    \"params\": {
      \"swap_id\": \"$SWAP_ID\"
    }
  }"
```

The swap state should now be `Completed`.

**After this step**: Bob should have L2 coins, and the swap should be marked as `Completed`.

#### 10g) Final validation

Verify the final state:

**Check Alice's parentchain balance** (should have received Bob's payment):
```bash
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$PARENTCHAIN_RPC_PORT" \
  -datadir="$PARENTCHAIN_DATADIR" \
  -rpcwallet="$PARENTCHAIN_WALLET" \
  getbalance
```

**Check Bob's sidechain balance** (should have received L2 coins from the swap):
```bash
# Use the coinshift app RPC to check balance
curl -X POST "$COINSHIFT_RPC_URL" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 7,
    \"method\": \"get_balance\",
    \"params\": []
  }"
```

**Check swap status** (should be `Completed`):
```bash
curl -X POST "$COINSHIFT_RPC_URL" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 8,
    \"method\": \"get_swap_status\",
    \"params\": {
      \"swap_id\": \"$SWAP_ID\"
    }
  }"
```

**List all swaps**:
```bash
curl -X POST "$COINSHIFT_RPC_URL" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 9,
    \"method\": \"list_swaps\",
    \"params\": []
  }"
```

**Expected final state**:
- ✅ Alice has parentchain coins (received from Bob)
- ✅ Bob has sidechain coins (claimed from swap)
- ✅ Swap is marked as `Completed`
- ✅ Locked L2 outputs are released

---

### 11) Stop everything

```bash
# Stop mainchain
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$MAINCHAIN_RPC_PORT" \
  -datadir="$MAINCHAIN_DATADIR" \
  stop || true

# Stop parentchain
"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$PARENTCHAIN_RPC_PORT" \
  -datadir="$PARENTCHAIN_DATADIR" \
  stop || true

# Stop enforcer (best-effort)
pkill -f "bip300301_enforcer" || true
```

---

## Summary

This guide sets up:
1. **Mainchain regtest**: Where the sidechain is activated (BIP300 operations)
2. **Parentchain regtest**: Where swap transactions occur (coinshift operations)
3. **Enforcer**: Connected to mainchain for sidechain management
4. **Complete swap flow**: Alice deposits → creates swap → Bob fills → Bob claims

**Key points**:
- Mainchain handles deposits/withdrawals (BIP300)
- Parentchain handles swap transactions (coinshift)
- The system monitors the parentchain for swap transactions
- Both chains are regtest for easy local testing
- Final state: Alice has parentchain coins, Bob has sidechain coins

