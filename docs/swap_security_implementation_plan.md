# Swap Security Improvements - Implementation Plan

This document maps out all the changes needed to implement the remaining security improvements from `swap_security_implemented.md`, using the BMM-based approach for multi-source RPC verification.

## Overview

We need to implement 4 major improvements:
1. **Multi-Source RPC Verification (BMM-Based)** - Use BMM participants as validation sources
2. **Merkle Proof Verification** - Cryptographically prove transaction inclusion in blocks
3. **Block Header Chain Verification** - Maintain light client header chains for independent verification
4. **Confirmation Count Verification** - Calculate confirmations from header chain

---

## 1. Multi-Source RPC Verification (BMM-Based)

### 1.1 Data Structures

**New Type: `L1TransactionReport`**
- Location: `lib/types/swap.rs` or new file `lib/types/l1_report.rs`
- Fields:
  ```rust
  pub struct L1TransactionReport {
      pub swap_id: SwapId,
      pub l1_txid: SwapTxId,
      pub confirmations: u32,
      pub block_height: u32,
      pub mainchain_block_hash: bitcoin::BlockHash,
      pub signed_by: Address,  // BMM participant address
      pub signature: Signature,  // ed25519 signature
  }
  ```

**Modify: `Body` structure**
- Location: `lib/types/mod.rs`
- Add field: `pub l1_transaction_reports: Vec<L1TransactionReport>`
- Or: Store in coinbase as special transaction type

### 1.2 BMM Participant Identification

**Note**: Since anyone can participate in BMM, we don't identify specific participants. Instead, we track reports by the signer's address (extracted from the signature/authorization).

**New Function: `extract_report_signer()`**
- Location: `lib/state/two_way_peg_data.rs` or new file `lib/state/bmm.rs`
- Purpose: Extract the address of the BMM participant who signed the report
- Implementation: 
  - Extract verifying key from report signature
  - Convert to address using `get_address()` (from authorization module)
  - This address uniquely identifies the BMM participant who created the report

### 1.3 Consensus Logic

**New Function: `check_consensus_on_swap_reports()`**
- Location: `lib/state/two_way_peg_data.rs`
- Purpose: Check if multiple independent BMM participants agree on swap transaction reports
- Consensus Requirements:
  - **Minimum participants**: Require reports from at least N different BMM participants (e.g., 2-3)
  - **Agreement**: All reports must agree on:
    - Same `l1_txid` for the swap
    - Confirmations within acceptable range (e.g., within 1-2 blocks of each other)
  - **Uniqueness**: Each report must be from a different BMM participant (different address)
- Logic:
  - Group reports by `swap_id`
  - For each swap, collect reports from different participants (by address)
  - Check if minimum number of independent participants agree
  - Use median or minimum confirmation count from agreeing reports
  - Track which participants have reported (to avoid double-counting)

**New Function: `process_bmm_swap_reports()`**
- Location: `lib/state/two_way_peg_data.rs`
- Purpose: Process and validate BMM swap reports from a block
- Steps:
  1. Extract reports from block body
  2. Verify signatures of each report
  3. Extract signer address for each report
  4. Check consensus (N independent participants agree)
  5. Update swap states if consensus reached
- Consensus Configuration:
  - Minimum participants: Configurable (default: 2)
  - Can accumulate reports across multiple blocks if needed
  - Track reports in database to build consensus over time

### 1.4 Integration Points

**Modify: `validate_block()` or `connect()`**
- Location: `lib/state/block.rs` or `lib/state/two_way_peg_data.rs`
- Add call to `process_bmm_swap_reports()` after block validation
- Process reports before updating swap states

**Modify: `process_coinshift_transactions()`**
- Location: `lib/state/two_way_peg_data.rs`
- Change: Instead of direct RPC calls, use BMM reports from blocks
- Fallback: Still allow RPC calls for backward compatibility or when no reports available

### 1.5 Database Changes

**New Database: `bmm_swap_reports`**
- Location: `lib/state/mod.rs`
- Key: `(SwapId, Address)` - Swap ID and BMM participant address
- Value: `L1TransactionReport` - Individual report from a participant
- Purpose: Track reports from different BMM participants for consensus checking
- Alternative: Could also key by `(BlockHash, SwapId, Address)` to track which block each report came from

---

## 2. Merkle Proof Verification

### 2.1 RPC Client Extensions

**New Method: `get_merkle_proof()`**
- Location: `lib/bitcoin_rpc.rs`
- Purpose: Request Merkle proof from Bitcoin RPC
- Implementation:
  ```rust
  pub fn get_merkle_proof(
      &self,
      txid: &str,
      block_hash: Option<&str>,
  ) -> Result<MerkleProof, Error>
  ```
  - Uses `gettxoutproof` RPC method
  - Returns Merkle proof structure

**New Type: `MerkleProof`**
- Location: `lib/types/merkle.rs` (new file)
- Contains: Merkle tree path, transaction index, block hash

### 2.2 Verification Logic

**New Function: `verify_merkle_proof()`**
- Location: `lib/types/merkle.rs`
- Purpose: Verify Merkle proof against block header
- Steps:
  1. Recompute Merkle root from proof path
  2. Compare with block header's Merkle root
  3. Verify transaction is at correct index
- Returns: `Result<bool, MerkleProofError>`

### 2.3 Integration

**Modify: `query_and_update_swap()` or BMM report validation**
- Location: `lib/state/two_way_peg_data.rs`
- Add: Request and verify Merkle proof when transaction is first detected
- Store: Merkle proof verification result in swap state

**New Field: `merkle_proof_verified`**
- Location: `lib/types/swap.rs` in `Swap` struct
- Type: `Option<bool>`
- Purpose: Track if Merkle proof has been verified

---

## 3. Block Header Chain Verification

### 3.1 Data Structures

**New Type: `HeaderChain`**
- Location: `lib/types/header_chain.rs` (new file)
- Purpose: Store and manage block headers for a parent chain
- Fields:
  ```rust
  pub struct HeaderChain {
      pub parent_chain: ParentChainType,
      pub headers: BTreeMap<u32, bitcoin::BlockHeader>,  // height -> header
      pub tip_height: u32,
      pub tip_hash: bitcoin::BlockHash,
  }
  ```

**New Database: `header_chains`**
- Location: `lib/state/mod.rs`
- Key: `ParentChainType`
- Value: `HeaderChain`
- Purpose: Store header chain per parent chain

### 3.2 Header Sync Logic

**New Function: `sync_header_chain()`**
- Location: `lib/state/header_chain.rs` (new file)
- Purpose: Download and store headers for a parent chain
- Implementation:
  - Similar to existing header sync in `bip300301_enforcer`
  - But for swap target chains (Signet, etc.), not just sidechain mainchain
  - Uses RPC to fetch headers in batches
  - Stores headers in database

**New Function: `add_header_to_chain()`**
- Location: `lib/state/header_chain.rs`
- Purpose: Add a single header to the chain
- Validates: prev_hash linkage, height consistency

### 3.3 Header Verification

**New Function: `verify_block_header()`**
- Location: `lib/state/header_chain.rs`
- Purpose: Verify a block header is valid
- Checks:
  - prev_hash matches previous header
  - Proof-of-work (for Bitcoin)
  - Timestamp is reasonable
  - Merkle root is valid format

**New Function: `verify_header_chain()`**
- Location: `lib/state/header_chain.rs`
- Purpose: Verify entire header chain is valid
- Checks: All headers link correctly, no gaps, valid PoW

### 3.4 Integration

**Modify: `process_coinshift_transactions()` or BMM report validation**
- Location: `lib/state/two_way_peg_data.rs`
- Add: Sync header chain for swap target chain if needed
- Use: Header chain to verify block heights and confirmations

**New Background Task: Header Chain Sync**
- Location: `lib/node/mod.rs` or new task file
- Purpose: Periodically sync header chains for all configured parent chains
- Runs: Similar to existing mainchain sync task

---

## 4. Confirmation Count Verification

### 4.1 Confirmation Calculation

**New Function: `calculate_confirmations_from_header_chain()`**
- Location: `lib/state/header_chain.rs`
- Purpose: Calculate confirmations using header chain
- Implementation:
  ```rust
  pub fn calculate_confirmations(
      &self,
      block_height: u32,
  ) -> Result<u32, Error> {
      if block_height > self.tip_height {
          return Err(Error::BlockHeightTooHigh);
      }
      Ok(self.tip_height - block_height + 1)
  }
  ```

### 4.2 Integration

**Modify: Swap validation logic**
- Location: `lib/state/two_way_peg_data.rs`
- Change: Use header chain for confirmation counts instead of RPC
- Steps:
  1. Get block height from transaction (from RPC or BMM report)
  2. Look up header chain for parent chain
  3. Calculate confirmations from header chain tip
  4. Use calculated confirmations for swap state updates

**Modify: BMM report validation**
- Location: `lib/state/two_way_peg_data.rs`
- Add: Verify reported confirmations match header chain calculation
- Reject: Reports with incorrect confirmation counts

---

## 5. Error Types

### New Error Variants

**Location: `lib/state/error.rs`**

```rust
#[error("Merkle proof verification failed: {0}")]
MerkleProofFailed(String),

#[error("Header chain error: {0}")]
HeaderChainError(String),

#[error("Consensus not reached for swap {swap_id}: {reason}")]
ConsensusNotReached {
    swap_id: SwapId,
    reason: String,
},

#[error("Insufficient BMM participants for consensus: have {have}, need {need}")]
InsufficientBmmParticipants { have: u32, need: u32 },

#[error("Header chain not synced for parent chain: {0:?}")]
HeaderChainNotSynced(ParentChainType),

#[error("Block header verification failed: {0}")]
BlockHeaderVerificationFailed(String),
```

---

## 6. Database Schema Changes

### New Databases

**In `lib/state/mod.rs`:**

```rust
pub struct State {
    // ... existing databases ...
    
    /// BMM swap reports by swap and participant address
    pub bmm_swap_reports: DatabaseUnique<
        SerdeBincode<(SwapId, Address)>,
        SerdeBincode<L1TransactionReport>,
    >,
    
    /// Header chains per parent chain
    pub header_chains: DatabaseUnique<
        SerdeBincode<ParentChainType>,
        SerdeBincode<HeaderChain>,
    >,
}
```

### Update `NUM_DBS`

Change `State::NUM_DBS` from 15 to 17 (add 2 new databases).

---

## 7. Configuration

### Consensus Configuration

**New Config: `bmm_consensus_min_participants`**
- Location: Configuration file or environment
- Purpose: Minimum number of independent BMM participants required for consensus
- Default: 2
- Format: `u32`
- Note: Since anyone can BMM, we don't need a whitelist of participants

---

## 8. Migration Strategy

### Backward Compatibility

1. **Gradual Rollout**: 
   - Keep existing RPC-based validation as fallback
   - Use BMM reports when available, fall back to RPC otherwise

2. **Database Migration**:
   - New databases are optional (can be empty initially)
   - No migration needed for existing swaps

3. **Header Chain Sync**:
   - Start syncing headers from current tip
   - Don't require full historical sync initially

---

## 9. Testing Strategy

### Unit Tests

1. **L1TransactionReport**: Serialization, signature verification
2. **Consensus Logic**: Test 2-out-of-3 consensus scenarios
3. **Merkle Proof**: Verify proof verification logic
4. **Header Chain**: Test header linking, verification, confirmation calculation

### Integration Tests

1. **BMM Report Flow**: Create block with reports, verify consensus, update swaps
2. **Merkle Proof**: Request proof from RPC, verify against header
3. **Header Chain Sync**: Sync headers, verify chain integrity
4. **End-to-End**: Create swap, BMM participants report, consensus reached, swap updated

---

## 10. Implementation Order

### Phase 1: Foundation
1. Header Chain structures and database
2. Header sync logic
3. Header verification

### Phase 2: Merkle Proofs
4. Merkle proof RPC methods
5. Merkle proof verification
6. Integration into swap validation

### Phase 3: BMM Reports
7. L1TransactionReport structure
8. BMM participant identification
9. Report storage in blocks
10. Consensus logic
11. Integration into block validation

### Phase 4: Confirmation Verification
12. Confirmation calculation from header chain
13. Integration into swap updates
14. BMM report confirmation verification

### Phase 5: Testing & Documentation
15. Comprehensive testing
16. Update documentation
17. Migration guide

---

## 11. Files to Create/Modify

### New Files
- `lib/types/l1_report.rs` - L1TransactionReport structure
- `lib/types/merkle.rs` - Merkle proof types and verification
- `lib/state/header_chain.rs` - Header chain management
- `lib/state/bmm.rs` - BMM participant utilities

### Modified Files
- `lib/types/mod.rs` - Add L1TransactionReport to exports, modify Body
- `lib/types/swap.rs` - Add merkle_proof_verified field
- `lib/state/mod.rs` - Add new databases, update NUM_DBS
- `lib/state/error.rs` - Add new error types
- `lib/state/block.rs` - Integrate BMM report processing
- `lib/state/two_way_peg_data.rs` - Update swap validation logic
- `lib/bitcoin_rpc.rs` - Add get_merkle_proof method
- `docs/swap_security_implemented.md` - Update status

---

## 12. Dependencies

### New Dependencies (if needed)
- Merkle proof verification library (or implement manually)
- Header verification utilities (may already exist in bitcoin crate)

### Existing Dependencies
- `bitcoin` crate - Already used for block headers
- `ed25519_dalek` - Already used for signatures
- `borsh` - Already used for serialization

---

## Notes

- **BMM Approach**: This leverages existing BMM infrastructure, making it more elegant than traditional multi-RPC
- **Migration**: No migration needed the system is in development now, so no real users.
- **Performance**: Header chains are lightweight (~80 bytes per header), sync is efficient
- **Security**: Multiple layers of verification (BMM consensus, Merkle proofs, header chain) provide strong security guarantees

---

## 13. Codebase Alignment Review

### ✅ Verified Alignments

1. **Body Structure** (`lib/types/mod.rs`):
   - Current: `pub struct Body { coinbase, transactions, authorizations }`
   - Plan: Add `l1_transaction_reports: Vec<L1TransactionReport>` field
   - Status: ✅ Compatible - can add field without breaking changes

2. **Swap Structure** (`lib/types/swap.rs`):
   - Current: Has `l1_txid_validated_at_block_hash` and `l1_txid_validated_at_height` fields
   - Plan: Add `merkle_proof_verified: Option<bool>` field
   - Status: ✅ Compatible - can add optional field

3. **State Database** (`lib/state/mod.rs`):
   - Current: `NUM_DBS = 15` with 15 databases defined
   - Plan: Add 2 new databases (`header_chains`, `bmm_swap_reports`), update to `NUM_DBS = 17`
   - Status: ✅ Compatible - need to update constant and add database creation

4. **Error Types** (`lib/state/error.rs`):
   - Current: Has `L1TransactionAlreadyUsed` and `L1TransactionValidationFailed` (already implemented)
   - Plan: Add new error variants for Merkle proofs, header chains, consensus
   - Status: ✅ Compatible - can add new variants

5. **BitcoinRpcClient** (`lib/bitcoin_rpc.rs`):
   - Current: Has `get_transaction()`, `get_transaction_confirmations()`, `list_transactions()`
   - Plan: Add `get_merkle_proof()` method
   - Status: ✅ Compatible - can add new method

6. **ParentChainType** (`lib/types/swap.rs`):
   - Current: Enum with `BTC`, `BCH`, `LTC`, `Signet`, `Regtest`
   - Plan: Use as key for header chains database
   - Status: ✅ Compatible - already has `Hash` and `Eq` derives

### 📋 Reference Implementation

**Header Chain Sync Pattern** (from `bip300301_enforcer`):
- Location: `bip300301_enforcer/lib/validator/task/mod.rs::sync_headers()`
- Pattern: Batch fetching (2000 headers max), height tracking, prev_hash verification
- Storage: `bip300301_enforcer/lib/validator/dbs/block_hashes.rs::put_headers()`
- Can adapt this pattern for swap target chains (Signet, etc.)

### ⚠️ Considerations

1. **Body Modification**: Adding `l1_transaction_reports` to `Body` requires:
   - Updating `Body::new()` constructor (or making field optional initially)
   - Updating serialization/deserialization if needed
   - Consider backward compatibility during rollout

2. **Database Migration**: Adding 2 new databases requires:
   - Updating `State::NUM_DBS` from 15 to 17
   - Adding database creation in `State::new()`
   - No data migration needed (new databases start empty)

3. **BMM Participant Identification**: 
   - Plan mentions extracting signer from signature/authorization
   - Need to verify how BMM participants are currently identified in blocks
   - May need to check `lib/authorization.rs` for address extraction patterns

4. **Header Chain Storage**:
   - Plan uses `BTreeMap<u32, bitcoin::BlockHeader>` for in-memory structure
   - Database storage should use efficient key-value format
   - Consider using height as key (similar to `bip300301_enforcer` pattern)

### 🔍 Files to Review Before Implementation

1. `lib/authorization.rs` - Understand how BMM participant addresses are extracted
2. `lib/state/block.rs` - Understand block validation flow for integration point
3. `lib/node/mainchain_task.rs` - Understand background task patterns for header sync
4. `bip300301_enforcer/lib/validator/task/mod.rs` - Reference implementation for header sync

