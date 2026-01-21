# Coinshift: How It Works - Complete Overview

## Overview

Coinshift is a trustless swap system for a BIP300-style sidechain that enables peer-to-peer exchanges between L2 (sidechain) coins and L1 (parent chain) assets. The system currently supports **L2 → L1 swaps** where Alice offers L2 coins in exchange for L1 assets (BTC, BCH, LTC, Signet, etc.) sent by Bob.

## Core Architecture

### System Components

```
┌─────────────────────────────────────────────────────────────┐
│                    Sidechain Node                            │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  State Management (lib/state/)                       │  │
│  │  - Swap storage (3 databases with indexes)           │  │
│  │  - Output locking/unlocking                          │  │
│  │  - Transaction validation                            │  │
│  └──────────────────────────────────────────────────────┘  │
│                          │                                   │
│                          ▼                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  Block Processing (lib/state/block.rs)               │  │
│  │  - SwapCreate: Create swap, lock outputs             │  │
│  │  - SwapClaim: Verify state, unlock outputs           │  │
│  └──────────────────────────────────────────────────────┘  │
│                          │                                   │
│                          ▼                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  L1 Monitoring (lib/state/two_way_peg_data.rs)       │  │
│  │  - Query L1 RPC for matching transactions            │  │
│  │  - Validate transaction structure                    │  │
│  │  - Update swap states                                │  │
│  └──────────────────────────────────────────────────────┘  │
│                          │                                   │
│                          ▼                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  RPC Client (lib/bitcoin_rpc.rs)                     │  │
│  │  - Query parent chain (Signet, Mainnet, etc.)        │  │
│  │  - Find transactions by address/amount               │  │
│  │  - Get transaction details                           │  │
│  └──────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────────┐
│              Parent Chain (L1)                               │
│              (Bitcoin, Signet, BCH, LTC, etc.)               │
└─────────────────────────────────────────────────────────────┘
```

## Complete Swap Flow

### 1. Swap Creation (Alice)

**Step 1: Alice Creates Swap Offer**
- Alice has L2 coins and wants L1 assets (e.g., BTC)
- Alice calls RPC: `create_swap()` with:
  - `l1_recipient_address`: Her BTC address
  - `l1_amount`: Amount of BTC she wants (e.g., 0.001 BTC)
  - `l2_amount`: Amount of L2 coins she's offering
  - `l2_recipient`: Optional - if specified, only that address can claim
  - `required_confirmations`: How many L1 confirmations needed (defaults to chain default)

**Step 2: Swap ID Generation**
- Swap ID is computed deterministically from:
  - `l1_recipient_address`
  - `l1_amount`
  - `l2_sender_address` (Alice's L2 address from transaction input)
  - `l2_recipient_address` (if specified, or "OPEN_SWAP" marker)
- Uses BLAKE3 hash: `SwapId = blake3(l1_addr || l1_amt || l2_sender || l2_recipient)`
- **Security**: Deterministic ID prevents swap ID collisions and ensures uniqueness

**Step 3: SwapCreate Transaction**
- Wallet creates `SwapCreate` transaction:
  - Contains swap metadata (ID, parent chain, amounts, addresses)
  - Alice's L2 coins are locked as inputs
  - Transaction is signed and broadcast to sidechain

**Step 4: Block Processing - SwapCreate**
- When block containing `SwapCreate` is connected:
  1. **Validation** (`lib/state/swap.rs::validate_swap_create()`):
     - Verify computed swap ID matches transaction swap ID
     - Verify swap doesn't already exist
     - Verify transaction structure
  2. **Output Locking** (`lib/state/block.rs`):
     - All outputs from `SwapCreate` transaction are locked to the swap
     - Locked outputs cannot be spent except by `SwapClaim` transaction
     - Lock stored in `locked_outputs` database: `(OutPoint) -> SwapId`
  3. **Swap Storage**:
     - Swap saved to 3 databases with indexes:
       - `swaps`: `SwapId -> Swap` (primary)
       - `swaps_by_l1_txid`: `(ParentChain, SwapTxId) -> SwapId` (for uniqueness)
       - `swaps_by_state`: `(SwapState, SwapId) -> ()` (for querying by state)
  4. **Initial State**: Swap starts in `SwapState::Pending`

### 2. L1 Transaction Monitoring (Bob)

**Step 1: Bob Sends L1 Transaction**
- Bob sees Alice's swap offer
- Bob sends L1 transaction (e.g., BTC) to Alice's `l1_recipient_address`
- Transaction must match exact `l1_amount`

**Step 2: L1 Monitoring Trigger**
- Monitoring happens during 2WPD (two-way peg data) processing
- When sidechain's mainchain tip changes, `process_coinshift_transactions()` is called
- This queries L1 RPC for all pending swaps

**Step 3: Transaction Matching**
- For each pending swap, system queries L1 RPC:
  - Uses `find_transactions_by_address_and_amount()` 
  - Searches for transactions to `l1_recipient_address` with exact `l1_amount`
  - Returns matching transactions with confirmations

**Step 4: Transaction Validation** (`lib/state/two_way_peg_data.rs::query_and_update_swap()`)

**Security Checks Performed**:

1. **Transaction Uniqueness** ✅ **IMPLEMENTED**
   - Check if L1 transaction is already associated with another swap
   - Prevents same transaction from being used for multiple swaps
   - Error: `L1TransactionAlreadyUsed` if transaction already claimed
   - Code: `state.get_swap_by_l1_txid()` lookup

2. **Transaction Confirmation** ✅ **IMPLEMENTED**
   - Verify transaction is confirmed (not in mempool)
   - Reject if `confirmations == 0`
   - Only confirmed transactions are accepted

3. **Block Inclusion Verification** ✅ **IMPLEMENTED**
   - Verify transaction has `blockheight` (is actually in a block)
   - Reject transactions with confirmations but no block height (suspicious)
   - Ensures transaction is part of blockchain, not just reported

4. **Output Validation** ✅ **IMPLEMENTED**
   - Verify at least one output matches expected address exactly
   - Verify output amount matches expected amount exactly
   - Prevents partial matches or address mismatches

5. **Transaction ID Consistency** ✅ **IMPLEMENTED**
   - On confirmation updates, verify transaction ID matches
   - Prevents transaction replacement attacks
   - Ensures same transaction is being tracked

**Step 5: Swap State Update**
- If validation passes:
  - Update swap with `l1_txid`
  - Store sidechain block reference where validation occurred:
    - `l1_txid_validated_at_block_hash`
    - `l1_txid_validated_at_height`
  - Update state based on confirmations:
    - If `confirmations >= required_confirmations`: `SwapState::ReadyToClaim`
    - Otherwise: `SwapState::WaitingConfirmations(current, required)`

### 3. Swap Claiming (Bob)

**Step 1: Bob Creates SwapClaim Transaction**
- Bob waits for required confirmations
- Bob calls RPC: `claim_swap()` with:
  - `swap_id`: The swap to claim
  - `l2_claimer_address`: Bob's L2 address (for open swaps)
  - Transaction fee

**Step 2: SwapClaim Validation** (`lib/state/swap.rs::validate_swap_claim()`)
- Verify swap exists
- Verify swap is in `ReadyToClaim` state
- Verify at least one input is locked to this swap
- Verify all locked inputs are locked to the same swap
- Verify at least one output goes to `swap.l2_recipient` (or claimer for open swaps)

**Step 3: Block Processing - SwapClaim**
- When block containing `SwapClaim` is connected:
  1. Retrieve swap from database
  2. Verify state is `ReadyToClaim`
  3. **Unlock Outputs**:
     - Remove locks from all inputs locked to this swap
     - Outputs become spendable again
  4. **Mark Swap Complete**:
     - Update state to `SwapState::Completed`
     - Save swap to database

**Step 4: Bob Receives L2 Coins**
- Bob's L2 address receives the locked L2 coins
- Swap is complete

## Security Architecture

### Current Security Measures (✅ Implemented)

#### 1. Transaction Uniqueness Enforcement
**What**: Prevents the same L1 transaction from being used for multiple swaps.

**How**:
- Database index: `swaps_by_l1_txid: (ParentChain, SwapTxId) -> SwapId`
- Before accepting a transaction, check if it's already associated with another swap
- Error if transaction is already claimed: `L1TransactionAlreadyUsed`

**Security Impact**:
- Prevents double-spending of L1 transactions
- Ensures one-to-one mapping between L1 transactions and swaps

**Code Location**: `lib/state/two_way_peg_data.rs::query_and_update_swap()` (lines 580-595)

#### 2. Enhanced Transaction Validation
**What**: Full validation of transaction structure when first detected.

**How**:
- Verify transaction is confirmed (not in mempool)
- Verify transaction has block height (is in a block)
- Verify outputs match expected address and amount exactly
- Verify transaction ID matches on updates (prevents replacement)

**Security Impact**:
- Prevents mempool-only transactions from being accepted
- Prevents fake transactions without block inclusion
- Prevents partial matches or incorrect amounts
- Prevents transaction replacement attacks

**Code Location**: `lib/state/two_way_peg_data.rs::query_and_update_swap()` (lines 597-652)

#### 3. Output Locking Mechanism
**What**: Locks L2 outputs to swaps until claimed.

**How**:
- When `SwapCreate` is processed, all outputs are locked to the swap
- Locked outputs cannot be spent except by `SwapClaim` transaction
- Locks are stored in `locked_outputs` database
- Only `SwapClaim` transactions can unlock outputs

**Security Impact**:
- Prevents Alice from spending locked coins before swap completes
- Ensures coins are only released when L1 transaction is confirmed
- Prevents double-spending of locked outputs

**Code Location**: `lib/state/block.rs` (SwapCreate processing)

#### 4. Swap ID Deterministic Generation
**What**: Swap IDs are computed deterministically from swap parameters.

**How**:
- `SwapId = blake3(l1_addr || l1_amt || l2_sender || l2_recipient)`
- Same parameters always produce same swap ID
- Swap ID is verified during `SwapCreate` validation

**Security Impact**:
- Prevents swap ID collisions
- Ensures swap uniqueness
- Makes swap IDs verifiable

**Code Location**: `lib/types/swap.rs::SwapId::from_l2_to_l1()`

#### 5. State-Based Validation
**What**: Swaps progress through states with strict validation at each step.

**States**:
- `Pending`: Waiting for L1 transaction
- `WaitingConfirmations`: L1 transaction found, waiting for confirmations
- `ReadyToClaim`: Required confirmations reached, can be claimed
- `Completed`: Swap claimed and completed

**Security Impact**:
- Prevents claiming before confirmations
- Prevents invalid state transitions
- Ensures proper swap lifecycle

**Code Location**: `lib/types/swap.rs::SwapState`

#### 6. Block Reference Tracking
**What**: Tracks which sidechain block validated the L1 transaction.

**How**:
- Stores `l1_txid_validated_at_block_hash` and `l1_txid_validated_at_height`
- Provides audit trail of when validation occurred
- Links L1 validation to sidechain state

**Security Impact**:
- Provides auditability
- Enables replay protection
- Links L1 events to sidechain blocks

**Code Location**: `lib/types/swap.rs::Swap::set_l1_txid_validation_block()`

### Advanced Security Measures (✅ Implemented)

#### 1. Multi-Source RPC Verification (BMM-Based)
**Status**: ✅ **IMPLEMENTED**

**What**: BMM participants serve as validation sources instead of relying solely on a single RPC endpoint.

**Implementation**:
- BMM participants include `L1TransactionReport` in blocks they mine
- Reports are cryptographically signed by BMM participant
- Requires N independent participants (minimum 2) to agree before updating swap state
- Consensus built from reports across different blocks
- Reports verified against header chain when available

**Security Impact**:
- ✅ Eliminates single point of failure
- ✅ Requires compromising multiple independent BMM participants
- ✅ Provides cryptographic guarantees via signatures
- ✅ On-chain auditability (reports stored in blocks)

**Code Location**: 
- `lib/types/l1_report.rs` - Report structure and signature verification
- `lib/state/bmm_reports.rs` - Consensus checking and processing
- `lib/state/block.rs` - Integration into block validation

**Reference**: See `swap_security_implementation_status.md` Phase 3

#### 2. Merkle Proof Verification
**Status**: ⏳ **PARTIALLY IMPLEMENTED**

**What**: Cryptographically prove that a transaction is included in a specific block.

**Current Implementation**:
- ✅ `MerkleProof` data structure implemented (`lib/types/merkle.rs`)
- ✅ Verification logic implemented (`verify()` method)
- ✅ `merkle_proof_verified` field added to `Swap` struct
- ⏳ RPC integration for fetching proofs is pending (stub exists in `BitcoinRpcClient`)

**Security Impact** (when fully integrated):
- Cryptographic proof of transaction inclusion
- Independent of RPC trust
- Provides SPV-level verification

**Code Location**: 
- `lib/types/merkle.rs` - Merkle proof structure and verification
- `lib/bitcoin_rpc.rs` - RPC method stub (needs full implementation)

**Reference**: See `swap_security_implementation_status.md` Phase 2

#### 3. Block Header Chain Verification
**Status**: ✅ **IMPLEMENTED**

**What**: Maintain light client header chains for each parent chain.

**Implementation**:
- ✅ `HeaderChain` structure stores headers indexed by height
- ✅ Header sync function downloads and stores headers from RPC
- ✅ Headers verified for correct linkage (prev_hash)
- ✅ Proof-of-work verification (for Bitcoin-based chains)
- ✅ Chain tip tracking for confirmation calculation
- ⏳ Background sync task pending (manual trigger available)

**Security Impact**:
- ✅ Independent source of truth for confirmations
- ✅ SPV-level chain verification
- ✅ No trust in RPC for chain structure
- ✅ Lightweight (~80 bytes per header)

**Code Location**: 
- `lib/types/header_chain.rs` - Header chain structure
- `lib/state/header_chain.rs` - Sync and verification logic

**Reference**: See `swap_security_implementation_status.md` Phase 1

#### 4. Confirmation Count Verification
**Status**: ✅ **IMPLEMENTED**

**What**: Calculate confirmations from header chain instead of trusting RPC.

**Implementation**:
- ✅ Confirmation calculation: `confirmations = tip_height - block_height + 1`
- ✅ BMM reports verified against header chain calculations
- ✅ Allows small differences (MAX_CONFIRMATION_DIFF = 2 blocks) for timing
- ✅ Graceful degradation when header chain not synced

**Security Impact**:
- ✅ Independent confirmation calculation
- ✅ No trust in RPC for confirmation counts
- ✅ Verifiable against header chain
- ✅ BMM reports cross-validated against header chain

**Code Location**: 
- `lib/state/header_chain.rs` - Confirmation calculation functions
- `lib/state/bmm_reports.rs` - Report verification against header chain

**Reference**: See `swap_security_implementation_status.md` Phase 4

## Data Structures

### Swap Structure
```rust
pub struct Swap {
    pub id: SwapId,                          // 32-byte deterministic ID
    pub direction: SwapDirection,            // L2ToL1 or L1ToL2
    pub parent_chain: ParentChainType,        // BTC, Signet, BCH, LTC, etc.
    pub l1_txid: SwapTxId,                   // L1 transaction ID (when found)
    pub required_confirmations: u32,           // Required L1 confirmations
    pub state: SwapState,                     // Current swap state
    pub l2_recipient: Option<Address>,        // L2 recipient (None = open swap)
    pub l2_amount: bitcoin::Amount,           // L2 amount being swapped
    pub l1_recipient_address: Option<String>,  // L1 recipient address
    pub l1_amount: Option<bitcoin::Amount>,    // L1 amount expected
    pub l1_claimer_address: Option<String>,   // Who sent the L1 transaction
    pub created_at_height: u32,                // Sidechain block height
    pub expires_at_height: Option<u32>,        // Expiration (future)
    pub l1_txid_validated_at_block_hash: Option<BlockHash>,  // Audit trail
    pub l1_txid_validated_at_height: Option<u32>,            // Audit trail
    pub merkle_proof_verified: Option<bool>,  // Future: Merkle proof status
}
```

### Database Schema

**Primary Swap Database**:
- `swaps`: `SwapId -> Swap`
- Primary storage for all swaps

**Index: By L1 Transaction**:
- `swaps_by_l1_txid`: `(ParentChain, SwapTxId) -> SwapId`
- Used for transaction uniqueness checking
- Prevents double-spending

**Index: By State**:
- `swaps_by_state`: `(SwapState, SwapId) -> ()`
- Used for querying pending swaps
- Enables efficient state-based queries

**Output Locking**:
- `locked_outputs`: `OutPoint -> SwapId`
- Tracks which outputs are locked to which swaps
- Prevents spending locked outputs

## Trust Model

### Current Trust Assumptions

**Trusted** (with caveats):
- **BMM Consensus**: N independent BMM participants (minimum 2) for transaction reports ✅ **IMPLEMENTED**
- **Header Chain**: Cryptographically verified block headers for confirmation counts ✅ **IMPLEMENTED**
- **Merkle Proofs**: Data structure implemented, RPC integration pending ⏳ **PARTIAL**
- **RPC Endpoint**: Still used as fallback when BMM reports unavailable or header chain not synced

**Not Trusted** (Protected Against):
- ✅ Same transaction used for multiple swaps (uniqueness check)
- ✅ Mempool-only transactions (confirmation check)
- ✅ Fake transactions without block inclusion (block height check)
- ✅ Partial matches or wrong amounts (output validation)
- ✅ Transaction replacement (transaction ID consistency)
- ✅ Spending locked outputs (output locking)
- ✅ Single compromised RPC endpoint (BMM consensus requires N independent participants)
- ✅ Fake confirmation counts (verified against header chain)
- ✅ Fake transaction data (BMM consensus with cryptographic signatures)

**Trust Model Evolution**:
- **Before**: Single RPC endpoint was fully trusted
- **Now**: BMM consensus (2+ independent participants) + Header chain verification
- **Future**: Full Merkle proof integration will add cryptographic transaction inclusion proofs

## Attack Resistance

### Current Protection

**Attack: Double-Spending L1 Transaction**
- ✅ **Protected**: Transaction uniqueness enforcement prevents same transaction from matching multiple swaps

**Attack: Mempool-Only Transaction**
- ✅ **Protected**: Only confirmed transactions are accepted

**Attack: Fake Transaction Without Block**
- ✅ **Protected**: Transaction must have block height

**Attack: Wrong Amount or Address**
- ✅ **Protected**: Output validation ensures exact match

**Attack: Transaction Replacement**
- ✅ **Protected**: Transaction ID consistency check on updates

**Attack: Spending Locked Outputs**
- ✅ **Protected**: Output locking mechanism prevents spending except via SwapClaim

**Attack: Compromised RPC Endpoint**
- ✅ **Protected**: BMM consensus requires N independent participants (minimum 2) to agree
- **How**: BMM participants cryptographically sign transaction reports; consensus required before swap state updates
- **Fallback**: RPC still used when BMM reports unavailable, but BMM consensus is primary

**Attack: Fake Confirmation Counts**
- ✅ **Protected**: Header chain provides independent verification of confirmation counts
- **How**: BMM reports are verified against header chain calculations; mismatches are logged
- **Fallback**: System gracefully degrades if header chain not synced

**Attack: Fake Transaction Inclusion**
- ⏳ **Partially Protected**: Merkle proof data structure and verification logic implemented
- **Limitation**: RPC integration for fetching Merkle proofs is pending (stub implementation)
- **Current**: BMM consensus + header chain verification provide strong protection
- **Future**: Full Merkle proof integration will add cryptographic transaction inclusion proofs

**Attack: Compromising BMM Participants**
- ✅ **Protected**: Requires compromising N independent participants (minimum 2)
- **How**: Open participation in BMM makes it difficult for attackers to control consensus
- **Security**: Each participant independently queries L1 chain and signs reports cryptographically

## Integration Points

### Block Processing
- **Location**: `lib/state/block.rs`
- **SwapCreate**: Creates swap, locks outputs
- **SwapClaim**: Verifies state, unlocks outputs, marks complete

### L1 Monitoring
- **Location**: `lib/state/two_way_peg_data.rs::process_coinshift_transactions()`
- **Trigger**: Called during 2WPD processing when mainchain tip changes
- **Function**: Queries L1 RPC, validates transactions, updates swap states

### RPC Client
- **Location**: `lib/bitcoin_rpc.rs`
- **Methods**:
  - `find_transactions_by_address_and_amount()`: Find matching transactions
  - `get_transaction()`: Get full transaction details
  - `get_transaction_confirmations()`: Get confirmation count

### State Management
- **Location**: `lib/state/mod.rs`
- **Databases**: 3 swap databases + locked outputs database
- **Indexes**: By swap ID, by L1 transaction, by state

## Error Handling

### Error Types

**Swap Errors**:
- `SwapNotFound`: Swap doesn't exist
- `L1TransactionAlreadyUsed`: Transaction already claimed by another swap
- `L1TransactionValidationFailed`: Transaction validation failed
- `InvalidTransaction`: Transaction structure invalid

**State Errors**:
- Swap not in correct state for operation
- Locked output cannot be spent
- Swap already exists

## Performance Considerations

### Database Efficiency
- Multiple indexes for efficient queries:
  - By swap ID (primary lookup)
  - By L1 transaction (uniqueness check)
  - By state (pending swap queries)
- Indexed lookups are O(log n)

### RPC Efficiency
- Batch queries when possible
- Cache results where appropriate
- Query only pending swaps (not all swaps)

### Block Processing
- Swap processing happens during block connection
- No separate polling task needed
- Integrated with 2WPD processing

## Current Limitations

1. **Merkle Proof RPC Integration**: Merkle proof data structure and verification logic are implemented, but full RPC integration for fetching proofs from Bitcoin nodes is pending (stub implementation exists)
2. **Header Chain Background Sync**: Header chain sync is implemented but requires manual triggering; automatic background sync task is pending
3. **BMM Report Cleanup**: No automatic cleanup of old BMM reports; database will grow over time (can be addressed with periodic cleanup)

**Note**: The core security features (BMM consensus, header chain verification, confirmation verification) are fully implemented and operational. The remaining limitations are primarily around automation and RPC integration completeness.

## Summary

Coinshift provides a trustless swap system with the following key features:

**Current Security** (✅ Implemented):
- ✅ Transaction uniqueness enforcement
- ✅ Enhanced transaction validation
- ✅ Output locking mechanism
- ✅ Deterministic swap IDs
- ✅ State-based validation
- ✅ Block reference tracking
- ✅ **BMM-based multi-source verification** (requires 2+ independent participants)
- ✅ **Header chain verification** (independent confirmation calculation)
- ✅ **Confirmation count verification** (verified against header chain)
- ⏳ **Merkle proof verification** (data structure implemented, RPC integration pending)

**Architecture**:
- Integrated with BIP300 sidechain
- Uses existing block processing infrastructure
- Leverages 2WPD processing for L1 monitoring
- Multiple database indexes for efficiency

The system is designed to be secure, efficient, and auditable, with a clear path for additional security improvements.

