# Swap Save and Restore Process

This document explains how swaps are saved and restored in different scenarios: when included on-chain, during rescans, when local-only, and during bootstrap/recovery after a crash.

## Overview

Swaps are persisted in the state database using the `sneed` (LMDB) database. The main storage is:
- **Primary storage**: `swaps` database (key: `SwapId`, value: `Swap`)
- **Indexes**:
  - `swaps_by_l1_txid`: Lookup by parent chain + L1 transaction ID
  - `swaps_by_recipient`: Lookup by L2 recipient address
  - `locked_swap_outputs`: Tracks which outputs are locked to which swap

## 1. When a Swap is Included on the Chain

### Process Flow

When a `SwapCreate` transaction is included in a block:

1. **Block Connection** (`lib/state/block.rs:connect()`)
   - The block is validated and connected
   - `SwapCreate` transactions are detected in the block body

2. **Swap Creation** (`lib/state/block.rs:254-348`)
   ```rust
   // Swap is reconstructed from transaction data
   let swap = Swap::new(
       swap_id,
       SwapDirection::L2ToL1,
       parent_chain,
       l1_txid,  // May be zero/empty initially
       required_confirmations,
       l2_recipient,
       l2_amount,
       l1_recipient_address,
       l1_amount,
       current_height,  // Block height where created
       None,
   );
   ```

3. **Output Locking** (for L2 → L1 swaps)
   - All outputs from the `SwapCreate` transaction are locked to the swap
   - Locked in `locked_swap_outputs` database

4. **Swap Persistence** (`lib/state/mod.rs:save_swap()`)
   - **Corruption Check**: If swap already exists and is corrupted, it's deleted first
   - **Save to Database**: Swap is serialized and saved to `swaps` database
   - **Verification**: Immediately reads back to verify serialization worked
   - **Index Updates**:
     - Updates `swaps_by_l1_txid` index
     - Updates `swaps_by_recipient` index (if L2 recipient specified)

5. **State After Inclusion**
   - Swap is stored in database with `created_at_height` set
   - State is typically `SwapState::Pending` (waiting for L1 transaction)
   - Outputs are locked and tracked

### Key Code Locations
- `lib/state/block.rs:254-348` - Swap creation during block connection
- `lib/state/mod.rs:552-656` - `save_swap()` method

## 2. When a Swap is Rescanned

### Process Flow

Swaps are rescanned during Two-Way Peg Data (2WPD) processing when blocks are connected:

1. **Trigger** (`lib/state/two_way_peg_data.rs:process_coinshift_transactions()`)
   - Called during 2WPD processing for each block
   - Scans all pending/waiting swaps to check for L1 transactions

2. **Swap Loading**
   ```rust
   let swaps = state.load_all_swaps(rwtxn)?;
   ```
   - Loads all swaps from database
   - Filters for `Pending` or `WaitingConfirmations` state

3. **L1 Chain Query** (`lib/state/two_way_peg_data.rs:query_and_update_swap()`)
   - For each pending swap:
     - Queries the **swap target chain** (e.g., Signet) - NOT the sidechain's mainchain
     - Searches for transactions matching:
       - `l1_recipient_address`
       - `l1_amount`
   - Uses RPC client to query the parent chain blockchain

4. **State Updates**
   - **New Transaction Detected**:
     - Updates `swap.l1_txid` with found transaction ID
     - Updates state based on confirmations:
       - If `confirmations >= required_confirmations`: `ReadyToClaim`
       - Otherwise: `WaitingConfirmations { current_confirmations, required_confirmations }`
   - **Existing Transaction**:
     - Updates confirmation count if it increased
     - Transitions to `ReadyToClaim` if threshold reached

5. **Persistence**
   - If swap state changed, `save_swap()` is called
   - Updates are persisted to database

### Key Code Locations
- `lib/state/two_way_peg_data.rs:653-795` - `process_coinshift_transactions()`
- `lib/state/two_way_peg_data.rs:565-650` - `query_and_update_swap()`

## 3. When a Swap is Just Local (Mempool)

### Process Flow

Swaps in mempool (not yet included in a block) are handled differently:

1. **Creation**
   - User creates a `SwapCreate` transaction
   - Transaction is added to mempool
   - **NOT saved to state database yet**

2. **Display in UI** (`app/gui/swap/list.rs:refresh_swaps()`)
   ```rust
   // Get swaps from database
   let swaps_result = state.load_all_swaps(&rotxn)?;
   
   // Also check mempool for pending swaps
   for tx in mempool_txs {
       if let TxData::SwapCreate { swap_id, ... } = &tx.transaction.data {
           // Create temporary swap object for display
           let swap = Swap::new(..., 0, ...);  // Height 0 = pending
           swaps_result.push(swap);
       }
   }
   ```
   - Swaps from mempool are shown with `created_at_height = 0`
   - They appear in the UI but aren't persisted

3. **When Included in Block**
   - When the block containing the swap is connected:
     - Swap is created and saved to database (see section 1)
     - `created_at_height` is set to actual block height
   - Mempool transaction is removed

4. **Cancellation**
   - If user cancels a pending swap:
     - Transaction is removed from mempool
     - No database cleanup needed (was never saved)

### Key Code Locations
- `app/gui/swap/list.rs:29-93` - `refresh_swaps()` - Shows mempool swaps
- `app/gui/swap/list.rs:823-821` - `delete_swap()` - Handles mempool vs database swaps

## 4. Bootstrap/Recovery After Crash

### Process Flow

When a node starts up after a crash (or from scratch with an existing wallet):

1. **Node Initialization** (`lib/node/mod.rs:new()`)
   - Database environment is opened
   - State and Archive are initialized
   - **Corruption Detection** runs automatically:

2. **Corruption Detection** (`lib/node/mod.rs:224-263`)
   ```rust
   let corrupted_swaps = state.find_corrupted_swaps(&rotxn)?;
   
   if !corrupted_swaps.is_empty() {
       // Automatically reconstruct from blockchain
       state.reconstruct_swaps_from_blockchain(&mut rwtxn, &archive, None)?;
   }
   ```

3. **Corruption Detection Details** (`lib/state/mod.rs:find_corrupted_swaps()`)
   - Iterates through all swaps in database
   - Attempts to deserialize each swap
   - Identifies swaps that cannot be deserialized
   - Returns list of corrupted swap IDs

4. **Reconstruction from Blockchain** (`lib/state/mod.rs:reconstruct_swaps_from_blockchain()`)
   - **Full Blockchain Scan**:
     - Collects all block hashes from genesis to tip
     - Processes blocks in forward order (genesis → tip)
   
   - **SwapCreate Transactions**:
     ```rust
     // Reconstruct swap from transaction
     let swap = Swap::new(...);
     
     // Lock outputs
     state.lock_output_to_swap(rwtxn, &outpoint, &swap_id)?;
     
     // Save to database
     state.save_swap(rwtxn, &swap)?;
     ```
   
   - **SwapClaim Transactions**:
     ```rust
     // Get existing swap
     let mut swap = state.get_swap(rwtxn, &swap_id)?;
     
     // Unlock outputs
     state.unlock_output_from_swap(rwtxn, &outpoint)?;
     
     // Mark as completed
     swap.mark_completed();
     state.save_swap(rwtxn, &swap)?;
     ```
   
   - **Result**: All swaps are reconstructed from blockchain data
   - **Note**: This rebuilds the entire swap database from scratch

5. **State After Recovery**
   - All swaps that were in blocks are restored
   - Swap states are restored (Pending, WaitingConfirmations, ReadyToClaim, Completed)
   - Output locks are restored
   - **Mempool swaps are lost** (they were never in blocks)

6. **L1 Transaction State**
   - After reconstruction, swaps may have:
     - `l1_txid` set if it was in the SwapCreate transaction
     - State may be `Pending` if L1 transaction wasn't detected yet
   - On next block connection, `process_coinshift_transactions()` will:
     - Query L1 chains for matching transactions
     - Update swap states accordingly

### Key Code Locations
- `lib/node/mod.rs:224-263` - Corruption detection and reconstruction on startup
- `lib/state/mod.rs:1148-1230` - `find_corrupted_swaps()`
- `lib/state/mod.rs:1408-1584` - `reconstruct_swaps_from_blockchain()`

## Save Process Details

### `save_swap()` Method (`lib/state/mod.rs:552-656`)

1. **Corruption Handling**:
   - Checks if swap already exists
   - If corrupted, deletes it first
   - Logs warnings for corruption

2. **Serialization**:
   - Serializes swap using `SerdeBincode`
   - Saves to `swaps` database

3. **Verification**:
   - Immediately reads back the saved swap
   - Verifies it can be deserialized
   - If verification fails:
     - Logs error with full error chain
     - Deletes the corrupted swap
     - Returns error

4. **Index Updates**:
   - Updates `swaps_by_l1_txid` index
   - Updates `swaps_by_recipient` index (if applicable)

5. **Error Handling**:
   - All database errors are logged
   - Corrupted swaps are automatically cleaned up
   - Serialization errors are caught and reported

## Restore Process Details

### `load_all_swaps()` Method (`lib/state/mod.rs:957-1060`)

1. **Iteration**:
   - Iterates through all swaps in database
   - Attempts to deserialize each swap

2. **Error Handling**:
   - Corrupted swaps are skipped (logged but not fatal)
   - Continues loading other swaps
   - Tracks corrupted count

3. **Individual Loading**:
   - If errors detected, tries loading swaps individually
   - Helps identify which specific swaps are corrupted

4. **Result**:
   - Returns vector of successfully loaded swaps
   - Corrupted swaps are excluded but logged

## State Transitions

### Swap States

1. **Pending**: 
   - Swap created, waiting for L1 transaction
   - Saved when included in block
   - Rescanned during 2WPD processing

2. **WaitingConfirmations**:
   - L1 transaction detected
   - Waiting for required confirmations
   - Updated during rescan as confirmations increase

3. **ReadyToClaim**:
   - Required confirmations reached
   - Can be claimed by user
   - Persisted in database

4. **Completed**:
   - Swap was claimed
   - Outputs unlocked
   - Final state

5. **Cancelled**:
   - Swap expired or was cancelled
   - Outputs unlocked
   - Final state

## Important Notes

1. **Mempool Swaps**:
   - Never persisted to database
   - Lost on crash/restart
   - Only saved when included in a block

2. **L1 Transaction Detection**:
   - Happens during rescan (2WPD processing)
   - Requires RPC config for parent chain
   - Queries the swap target chain, not sidechain mainchain

3. **Corruption Recovery**:
   - Automatic on node startup
   - Reconstructs from blockchain
   - May lose L1 transaction state if not yet detected

4. **Output Locking**:
   - Tracks which outputs are locked to swaps
   - Restored during reconstruction
   - Critical for preventing double-spending

5. **Database Transactions**:
   - All saves happen within write transactions
   - Committed atomically
   - Rollback on error

## Summary

- **On-Chain**: Swaps are saved during block connection, with full persistence and indexing
- **Rescan**: Swaps are queried against L1 chains and updated during 2WPD processing
- **Local/Mempool**: Swaps exist only in memory, not persisted until included in block
- **Bootstrap/Recovery**: Automatic corruption detection and reconstruction from blockchain on startup


