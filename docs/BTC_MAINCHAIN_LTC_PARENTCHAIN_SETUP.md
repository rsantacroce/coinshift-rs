## Bitcoin mainchain (regtest) + Litecoin parent chain (regtest) — full setup

This guide uses the **repo scripts in `docs/`** to bring up:
- **Bitcoin regtest mainchain** (BIP300 activation + enforcer + sidechain mining)
- **Litecoin regtest** as the swap **parent chain** (`ParentChainType::LTC`)

Key point: for **LTC swaps**, Coinshift does **not** trust RPC as an oracle. The Litecoin RPC is used only to **fetch data to build an SPV proof**, and the sidechain verifies the proof deterministically (Phase 1).

---

### Prereqs

- Built + available:
  - Bitcoin Core (patched): `bitcoind`, `bitcoin-cli`
  - Litecoin Core: `litecoind`, `litecoin-cli`
  - `bip300301_enforcer`
  - `coinshift_app`
- `grpcurl` installed (needed by enforcer scripts)
- Scripts executable: `chmod +x docs/*.sh`

---

### 1) Start Bitcoin regtest mainchain

From the repo:

```bash
cd docs
./1_start_mainchain.sh
```

This starts Bitcoin regtest on the ports from `docs/1_start_mainchain.sh` and mines 101 blocks.

---

### 2) Start the enforcer (+ proposal)

```bash
./3_start_enforcer.sh
```

If it fails with a “wallet does not exist” / “wallet required” error, do:

```bash
./create_enforcer_wallet.sh ""
./unlock_enforcer_wallet.sh ""
./3_start_enforcer.sh --skip-proposal
```

---

### 3) Start Litecoin regtest (swap parent chain)

```bash
./5_start_litecoin.sh
```

Helper scripts:

```bash
./5a_litecoin_generate_address.sh
./5b_litecoin_check_balance.sh
./5c_mine_litecoin.sh 10
```

---

### 4) Start `coinshift_app` in GUI mode (recommended for LTC)

You need GUI mode because (today) the “Build & Submit LTC Proof” action is exposed in the swap UI.

Example (adapt paths/ports to your environment):

```bash
export COINSHIFT_DATADIR="/tmp/coinshift-btc-main-ltc-parent"
mkdir -p "$COINSHIFT_DATADIR"

./target/debug/coinshift_app \
  --datadir "$COINSHIFT_DATADIR" \
  --mainchain-grpc-url "http://127.0.0.1:50051" \
  --net-addr "127.0.0.1:8333" \
  --rpc-addr "127.0.0.1:6255" \
  --log-level info
```

---

### 5) Configure Litecoin RPC in the Coinshift GUI (L1 Config)

Open the **L1 Config** tab and set:
- **Parent Chain**: `LTC`
- **RPC URL**: `http://127.0.0.1:19443` (or your `LITECOIN_RPC_PORT`)
- **RPC User / Password**: match `docs/5_start_litecoin.sh` (`user` / `passwordDC` by default)

This is stored at `dirs::data_dir()/coinshift/l1_rpc_configs.json` (e.g. macOS: `~/Library/Application Support/coinshift/l1_rpc_configs.json`).

---

### 6) Fund your sidechain wallet (BTC mainchain deposit)

Use your existing deposit workflow (see:
- `docs/SETUP_ORDER.md`
- `docs/MANUAL_SWAP_TESTING.md` (shows `coinshift_app` + enforcer wiring)
).

Once you have spendable L2 funds, proceed to swaps.

---

### 7) Create an LTC swap (L2 → LTC)

1) Generate the **LTC recipient** address (Alice’s address to receive LTC):

```bash
LTC_ADDR="$(./5a_litecoin_generate_address.sh)"
echo "$LTC_ADDR"
```

2) Create a swap via Coinshift JSON-RPC:

```bash
curl -s -X POST "http://127.0.0.1:6255" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 1,
    \"method\": \"create_swap\",
    \"params\": {
      \"parent_chain\": \"LTC\",
      \"l1_recipient_address\": \"$LTC_ADDR\",
      \"l1_amount_sats\": 1000000,
      \"l2_recipient\": null,
      \"l2_amount_sats\": 1100000,
      \"required_confirmations\": 6,
      \"fee_sats\": 1000
    }
  }"
```

Note: `parent_chain` values are the enum variants from `coinshift::types::ParentChainType` (e.g. `LTC`, `BTC`, `Signet`, `Regtest`).

3) Mine a Coinshift block (BMM) so the swap is confirmed on L2:
- In the GUI, click **Mine / Refresh Block**
- Or via RPC:

```bash
curl -s -X POST "http://127.0.0.1:6255" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"mine","params":[null]}'
```

---

### 8) Bob pays on Litecoin regtest, then submit proof on L2

1) Bob sends LTC to the swap’s `l1_recipient_address`:

```bash
TXID="$(
  source ./_litecoin_env.sh
  ltc_cli -rpcwallet="$LITECOIN_WALLET" sendtoaddress "$LTC_ADDR" 0.01
)"
echo "$TXID"
```

2) Mine LTC blocks so the payment has confirmations:

```bash
./5c_mine_litecoin.sh 10
```

3) In the Coinshift GUI swap view:
- paste the LTC `TXID` into **L1 Transaction ID (hex)**
- click **Build & Submit LTC Proof**

After the proof is accepted and included in a block, the swap should move to **ReadyToClaim**.

---

### 9) Claim the swap on L2

Once **ReadyToClaim**, claim via GUI or RPC:

```bash
curl -s -X POST "http://127.0.0.1:6255" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 3,
    \"method\": \"claim_swap\",
    \"params\": { \"swap_id\": \"<SWAP_ID>\", \"l2_claimer_address\": null }
  }"
```

Then mine another Coinshift block (BMM) to confirm the claim.

---

### Related docs/scripts

- `docs/SETUP_ORDER.md` (baseline regtest setup order)
- `docs/SWAP_PHASE1_LTC_ZEC_SPV.md` (why LTC uses SPV proofs)
- `docs/1_start_mainchain.sh`, `docs/3_start_enforcer.sh`
- `docs/5_start_litecoin.sh` + helpers


