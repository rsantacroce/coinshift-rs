## Phase 1 (LTC): Permissionless dispute game + SPV proofs (design + decision)

### Context: why we’re changing swap progression
Today the swap implementation can move a swap into `ReadyToClaim` based on a **locally configured parent-chain RPC node** (e.g., `BitcoinRpcClient`). This creates two serious issues:

- **Bob can spoof**: if Bob can influence which RPC endpoint Alice/validators consult (or if the endpoint is malicious), he can fake tx existence, outputs, and “confirmations”.
- **Consensus divergence**: different Coinshift nodes may consult different RPC endpoints and reach different conclusions about whether a swap is claimable.

The Phase 1 goal is to make swap progression **deterministic and permissionless** for **Litecoin (LTC)** without relying on:

- RPC trust
- watcher quorums / attestations
- validator committees

### High-level idea
We implement an **optimistic dispute game** where *anyone* can propose “payment happened”, but the swap only becomes claimable when a **deterministically verifiable proof** is posted on-chain.

Key property: verifying the proof must depend **only on on-chain data + the submitted proof**, so all Coinshift nodes agree.

### Actors and incentives
- **Alice**: creates swap, locks L2 funds, expects L1 payment.
- **Bob**: pays on L1, wants to unlock L2 funds.
- **Anyone**: can help by posting proofs (including Bob, Alice, or third parties).

We also use an **Alice-only veto** mechanism:
- Alice can dispute an optimistic payment registration during a dispute window.
- A dispute is resolved by a **proof**, not by human arbitration.

### Phase 1 flow (deterministic)
#### 1) Swap creation (unchanged)
Alice creates `SwapCreate` which locks L2 funds as `SwapPending`.

#### 2) Register payment (optimistic, cheap)
Someone (usually Bob) submits an on-chain transaction:

- `SwapRegisterPayment { swap_id, tx_ref }`

This does **not** prove the payment; it just starts the dispute clock and records `tx_ref` as the candidate payment.

State transition:
- `Pending` → `WaitingConfirmations(0, dispute_blocks)`

Where “confirmations” are redefined for Phase 1 as **sidechain dispute blocks elapsed** (not L1 confirmations).

#### 3) Optional: Alice disputes (Alice-only veto)
Within `dispute_blocks` after registration, Alice may submit:

- `SwapDisputePayment { swap_id }`

This marks the swap as “disputed” (implementation may use a dedicated state or reuse `WaitingConfirmations` with an auxiliary flag in swap data).

Important: **dispute does not cancel immediately**. It triggers the requirement that a proof must be posted by the deadline.

#### 4) Proof submission (permissionless)
Anyone can submit:

- `SwapSubmitProof { swap_id, proof }`

The chain verifies `proof` deterministically. If valid, the swap becomes claimable:
- `WaitingConfirmations(..)` → `ReadyToClaim`

If Alice disputed and no valid proof is submitted by the deadline:
- swap → `Cancelled`

### What counts as a “proper proof” for LTC
For PoW UTXO chains, “tx + block height” is not sufficient. A deterministic proof requires:

- **The transaction** (or enough data to compute the txid and validate outputs)
- **A Merkle inclusion proof** that the txid is included in a particular block’s merkle root
- **A header chain** (multiple headers) so the verifier can:
  - validate Litecoin difficulty rules (retarget)
  - validate scrypt PoW for each header
  - compute confirmation depth relative to the chain tip included in the proof
- **An anchor checkpoint**: the proof’s first header must match an embedded checkpoint height/hash

We call this an **SPV proof**.

### Deterministic verification requirements
Coinshift must implement, for each supported chain:

#### A) Header verification
Verify the header chain is internally consistent and that each header satisfies Litecoin PoW.

- **LTC**: scrypt PoW + Bitcoin-like header structure.
- Enforce Litecoin **difficulty adjustment** rules (Bitcoin-style retarget every 2016 blocks).
- **TODO (future hardening)**: broader checkpoint set / more recent anchors to keep proofs small over time.

#### B) Inclusion verification
Verify the merkle proof from txid to header.merkle_root.

#### C) Swap-term matching
Verify the proved transaction pays:
- **recipient** == `swap.l1_recipient_address`
- **amount** == `swap.l1_amount`
- **chain** == `swap.parent_chain`

### Why this is “super decentralized”
- No fixed watcher set.
- No attestations.
- No trusted RPC endpoint.
- Any third party can provide proofs (markets will emerge: proof relayers).
- Alice can dispute, but she can’t “win” if a valid proof exists.

### Practical notes / limitations
- Phase 1 requires payments that are publicly verifiable.
- ZEC (transparent) support is deferred to a later release (Equihash verification is significantly more complex).
- This approach does not generalize to private-by-default chains like Monero or shielded Zcash without extra disclosure.

### Protocol changes (Phase 1)
We will add new transaction types (names indicative):

- `SwapRegisterPayment { swap_id, tx_ref }`
- `SwapDisputePayment { swap_id }`
- `SwapSubmitProof { swap_id, proof }`

Where:
- `tx_ref` is chain-specific (for LTC: txid bytes).
- `proof` is a chain-specific SPV proof blob (structured + versioned).

### Consensus rules changes (Phase 1)
- A swap may enter `ReadyToClaim` **only** via `SwapSubmitProof` (valid LTC SPV proof).
- The dispute window is measured in **sidechain blocks** and is deterministic.
- Local RPC is **never** used to progress LTC swaps in consensus.

### Implementation plan inside this repo (Phase 1)
#### Step 1 — Data model & Tx types
- Extend `TxData` with the Phase 1 swap txs.
- Extend swap state machine to represent:
  - registered payment (txid, registered_at_height)
  - dispute flag and/or disputed_at_height

#### Step 2 — SPV verifier
Implement:
- merkle inclusion verification
- tx parsing and output match checks
- LTC header-chain PoW verification (scrypt) and min confirmations (via included header chain)
- TODO: difficulty retarget + chainwork/best-chain selection

#### Step 4 — Swap validation hooks
Add consensus validation in `lib/state/swap.rs`:
- `SwapRegisterPayment` validity checks
- `SwapDisputePayment` checks (Alice-only; see below)
- `SwapSubmitProof` checks (SPV verification, term match)

#### Step 5 — Alice-only veto wiring
We must define “Alice identity” on L2 deterministically. For Phase 1 we bind it as:
- the L2 sender address inferred from the `SwapCreate` inputs (already used in swap-id derivation)

Then enforce:
- only that address may submit `SwapDisputePayment`.

### Decision: start Phase 1 with LTC
We will start Phase 1 now with LTC because it gives a **permissionless** path that does not rely on attestations/quorums, and it directly mitigates the “Bob spoofs parent chain node” problem by removing RPC trust.

We acknowledge:
- LTC PoW verification (scrypt) is feasible but requires careful implementation and testing.
- Difficulty retarget + chainwork/best-chain selection is the next hardening step.
- ZEC (transparent) will come later after LTC is well-tested.

### Acceptance criteria for Phase 1
- A Coinshift node can validate `SwapSubmitProof` for LTC and ZEC transparently and deterministically.
- No swap becomes claimable based on local RPC for these chains.
- Alice can dispute and be refuted by a valid proof.
- Two independent Coinshift nodes reach identical results on the same proof inputs.


