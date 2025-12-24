# Swap Confirmation Process Security Improvements

## Current Vulnerabilities

The current swap confirmation process has several security vulnerabilities:

1. **Single RPC Source Trust**: The system relies on a single RPC endpoint. If compromised, an attacker could:
   - Return fake transaction data
   - Return fake confirmation counts
   - Return fake transaction IDs

2. **No Merkle Proof Verification**: Transactions aren't verified to be actually included in blocks using Merkle proofs.

3. **No Block Header Verification**: No SPV-style verification of block headers and chain work.

4. **No Multi-Source Consensus**: No cross-checking between multiple independent sources.

5. **Transaction Collision Risk**: Multiple swaps with the same address and amount could match the same transaction.

6. **No Transaction Immutability**: Once a transaction is accepted, there's no verification it hasn't been replaced or double-spent.

## Proposed Improvements

### 1. Multi-Source RPC Verification

**Problem**: Single RPC endpoint can be compromised or malicious.

**Solution Options**: 

**Option A (Traditional Multi-RPC)**:
- Support multiple RPC endpoints per parent chain
- Require consensus (e.g., 2 out of 3) before accepting transaction data
- Cross-validate transaction details across sources

**Option B (BMM-Based - Recommended)**:
- Use BMM participants (Alice, Bob, Charles) as validation sources
- BMM participants report L1 transaction confirmations as part of block validation
- Reports are cryptographically signed and included in sidechain blocks
- Require consensus (e.g., 2 out of 3) from BMM participants
- More decentralized and integrated with existing consensus

**Implementation**:
- **Option A**: Add `Vec<RpcConfig>` support for multiple endpoints
- **Option B**: Add `L1TransactionReport` structure to BMM blocks, verify signatures, check consensus
- Create `verify_transaction_multi_source()` function
- Require matching transaction data from multiple sources

**Note**: BMM-based approach is more elegant because it leverages existing infrastructure and is cryptographically verifiable. See `merkle_vs_header_chain_explanation.md` for details.

### 2. Merkle Proof Verification

**Problem**: RPC can claim a transaction is in a block without proof.

**What it does**: Proves that a **specific transaction** is included in a **specific block** using a cryptographic Merkle proof.

**Solution**:
- Request Merkle proof from RPC (using `gettxoutproof` or similar)
- The proof is a path through the Merkle tree from transaction to block's Merkle root
- Verify Merkle proof by recomputing the Merkle root and comparing to block header

**Implementation**:
- Add `get_merkle_proof()` to BitcoinRpcClient
- Add `verify_merkle_proof()` function
- Store verified block headers in database

**Use case**: "Is transaction `txid_xyz` actually in block `100000`?"

### 3. Block Header Chain Verification

**Problem**: No verification that blocks are actually part of the chain, and confirmation counts aren't independently verified.

**What it does**: Maintains a **light client header chain** to verify blockchain structure and calculate confirmations independently.

**Solution**:
- Download and store block headers (not full blocks - just ~80 bytes each)
- Verify headers link correctly (each header's `prev_hash` matches previous header's hash)
- Verify proof-of-work (for Bitcoin)
- Track chain tip to calculate confirmations: `confirmations = current_height - block_height + 1`

**Implementation**:
- Add `HeaderChain` structure
- Add `verify_block_header()` function
- Store header chain in database per parent chain

**Use case**: "How many confirmations does block `100000` have?" and "Is this the longest valid chain?"

**Relationship**: 
- Merkle Proof proves: "This transaction is in this block"
- Header Chain proves: "This block is at height X and has Y confirmations"
- Together they provide complete independent verification

**Note**: See `merkle_vs_header_chain_explanation.md` for detailed explanation.

### 4. Transaction Uniqueness Enforcement

**Problem**: Same L1 transaction could match multiple swaps.

**Solution**:
- When a transaction is first detected, check if it's already associated with another swap
- Prevent double-matching of transactions
- Add transaction-to-swap mapping index

**Implementation**:
- Add `swaps_by_l1_txid` index (already exists, but needs stricter enforcement)
- Check for existing swap before accepting new match
- Return error if transaction already claimed

### 5. Enhanced Transaction Validation

**Problem**: Initial detection only checks address and amount.

**Solution**:
- Full transaction validation when first detected:
  - Verify transaction structure
  - Verify outputs match exactly
  - Verify transaction is confirmed (not just in mempool)
  - Verify block inclusion
  - Verify no double-spend

**Implementation**:
- Enhance `query_and_update_swap()` with full validation
- Add `validate_transaction_structure()` function
- Require minimum confirmations before initial acceptance

### 6. Replay Protection

**Problem**: Same transaction could be reused across different swaps.

**Solution**:
- Track all used L1 transactions
- Prevent reusing transactions that have already been claimed
- Add transaction usage history

**Implementation**:
- Add `used_l1_transactions` index
- Check before accepting transaction
- Mark as used when swap completes

### 7. Confirmation Count Verification

**Problem**: Confirmation counts from RPC aren't verified against actual block chain.

**Solution**:
- Calculate confirmations from header chain height
- Verify against RPC-reported confirmations
- Use header chain as source of truth

**Implementation**:
- Calculate confirmations: `current_height - block_height + 1`
- Cross-check with RPC confirmations
- Use header chain height as authoritative

## Implementation Priority

1. **High Priority** (Critical Security):
   - Transaction uniqueness enforcement
   - Enhanced transaction validation
   - Multi-source RPC verification

2. **Medium Priority** (Important Security):
   - Merkle proof verification
   - Block header chain verification
   - Confirmation count verification

3. **Low Priority** (Nice to Have):
   - Replay protection (if not already handled by uniqueness)

## Migration Strategy

1. Add new fields to Swap struct (optional initially)
2. Implement new verification alongside old code
3. Add feature flags to enable new verification
4. Gradually migrate swaps to use new verification
5. Remove old code once migration complete

