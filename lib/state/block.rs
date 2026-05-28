//! Connect and disconnect blocks

use rustreexo::accumulator::node_hash::BitcoinNodeHash;
use sneed::{RoTxn, RwTxn, db::error::Error as DbError};

use crate::{
    authorization::Authorization,
    state::{Error, PrevalidatedBlock, State, error},
    types::{
        AccumulatorDiff, AmountOverflowError, Body, FilledTransaction,
        GetAddress as _, GetValue as _, Header, InPoint, MerkleRoot, OutPoint,
        OutPointKey, PointedOutput, SpentOutput, Swap, SwapId, SwapState,
        SwapTxId, TxData, Verify as _,
    },
};

/// Prevalidate a block: compute and verify all read-only checks and
/// prepare data needed for fast connection.
pub fn prevalidate(
    state: &State,
    rotxn: &RoTxn,
    header: &Header,
    body: &Body,
) -> Result<PrevalidatedBlock, Error> {
    let tip_hash = state.try_get_tip(rotxn)?;
    if header.prev_side_hash != tip_hash {
        let err = error::InvalidHeader::PrevSideHash {
            expected: tip_hash,
            received: header.prev_side_hash,
        };
        return Err(Error::InvalidHeader(err));
    };
    let next_height = state.try_get_height(rotxn)?.map_or(0, |h| h + 1);
    if body.authorizations.len() > State::body_sigops_limit(next_height) {
        return Err(Error::TooManySigops);
    }
    let body_size =
        borsh::object_length(&body).map_err(Error::BorshSerialize)?;
    if body_size > State::body_size_limit(next_height) {
        return Err(Error::BodyTooLarge);
    }

    let mut accumulator = state
        .utreexo_accumulator
        .try_get(rotxn, &())
        .map_err(DbError::from)?
        .unwrap_or_default();

    // gather and verify transactions
    let mut total_inputs: usize = 0;
    let mut total_outputs: usize = 0;
    for tx in &body.transactions {
        total_inputs += tx.inputs.len();
        total_outputs += tx.outputs.len();
    }
    let mut all_input_keys: Vec<OutPointKey> = Vec::with_capacity(total_inputs);
    // Accumulator diff from txs. The bool value is true for insertions, false
    // for deletions.
    let mut accumulator_diff_txs =
        Vec::with_capacity(total_inputs + total_outputs);
    let mut filled_transactions: Vec<FilledTransaction> =
        Vec::with_capacity(body.transactions.len());
    let mut total_fees = bitcoin::Amount::ZERO;
    for transaction in &body.transactions {
        let txid = transaction.txid();
        let mut spent_utxos = Vec::with_capacity(transaction.inputs.len());
        let mut spent_utxo_hashes =
            Vec::<BitcoinNodeHash>::with_capacity(transaction.inputs.len());
        for (outpoint, utxo_hash) in &transaction.inputs {
            let key = OutPointKey::from(outpoint);
            let spent_output =
                state.utxos.try_get(rotxn, &key)?.ok_or(Error::NoUtxo {
                    outpoint: *outpoint,
                })?;
            all_input_keys.push(OutPointKey::from(outpoint));
            spent_utxos.push(spent_output);
            spent_utxo_hashes.push(utxo_hash.into());
            accumulator_diff_txs.push((false, utxo_hash.into()));
        }
        for (vout, output) in transaction.outputs.iter().enumerate() {
            let outpoint = OutPoint::Regular {
                txid,
                vout: vout as u32,
            };
            let pointed_output = PointedOutput {
                outpoint,
                output: output.clone(),
            };
            accumulator_diff_txs.push((true, (&pointed_output).into()));
        }
        if !accumulator.verify(&transaction.proof, &spent_utxo_hashes)? {
            return Err(Error::UtreexoProofFailed { txid });
        }
        let filled_tx = FilledTransaction {
            spent_utxos,
            transaction: transaction.clone(),
        };
        // Run swap-specific validation at block-connection time. The mempool
        // path (State::validate_transaction) runs these too, but blocks can be
        // produced/received without ever passing through the mempool, so the
        // checks MUST be enforced here as well. In particular, SwapClaim
        // transactions bypass the input-ownership check below (their
        // SwapPending inputs are owned by the swap creator but spent by the
        // claimer), so validate_swap_claim is the only thing ensuring such a
        // claim is legitimate and pays the full locked amount to the recipient.
        match &transaction.data {
            TxData::SwapCreate { .. } => {
                crate::state::swap::validate_swap_create(
                    state,
                    rotxn,
                    transaction,
                    &filled_tx,
                )?;
            }
            TxData::SwapClaim { .. } => {
                crate::state::swap::validate_swap_claim(
                    state,
                    rotxn,
                    transaction,
                    &filled_tx,
                )?;
            }
            TxData::Regular => {
                crate::state::swap::validate_no_locked_outputs(
                    state, rotxn, transaction,
                )?;
            }
        }
        total_fees = total_fees
            .checked_add(state.validate_filled_transaction(&filled_tx)?)
            .ok_or(AmountOverflowError)?;
        filled_transactions.push(filled_tx);
    }
    let computed_merkle_root = Body::compute_merkle_root(
        body.coinbase.as_slice(),
        filled_transactions.as_slice(),
    )?;
    if computed_merkle_root != header.merkle_root {
        let err = Error::InvalidBody {
            expected: header.merkle_root,
            computed: computed_merkle_root,
        };
        return Err(err);
    }
    {
        use rayon::prelude::ParallelSliceMut;
        all_input_keys.par_sort_unstable();
        if all_input_keys.windows(2).any(|w| w[0] == w[1]) {
            return Err(Error::UtxoDoubleSpent);
        }
    }
    let mut coinbase_value = bitcoin::Amount::ZERO;
    let mut accumulator_diff = AccumulatorDiff::with_capacity(
        body.coinbase.len() + accumulator_diff_txs.len(),
    );
    for (vout, output) in body.coinbase.iter().enumerate() {
        coinbase_value = coinbase_value
            .checked_add(output.get_value())
            .ok_or(AmountOverflowError)?;
        let outpoint = OutPoint::Coinbase {
            merkle_root: computed_merkle_root,
            vout: vout as u32,
        };
        let pointed_output = PointedOutput {
            outpoint,
            output: output.clone(),
        };
        accumulator_diff.insert((&pointed_output).into());
    }
    for (insert, utxo_hash) in accumulator_diff_txs {
        if insert {
            accumulator_diff.insert(utxo_hash);
        } else {
            accumulator_diff.remove(utxo_hash);
        }
    }
    if coinbase_value > total_fees {
        return Err(Error::NotEnoughFees);
    }
    // For SwapClaim transactions, SwapPending inputs are owned by the swap
    // creator but spent by the claimer (who may be a different wallet).
    // Skip the address-matching check for those specific inputs — swap
    // validation already ensures legitimacy.
    let mut auth_offset = 0usize;
    let mut swap_claim_pending = std::collections::HashSet::<usize>::new();
    for filled_tx in &filled_transactions {
        if matches!(filled_tx.transaction.data, TxData::SwapClaim { .. }) {
            for (i, utxo) in filled_tx.spent_utxos.iter().enumerate() {
                if utxo.content.is_swap_pending() {
                    swap_claim_pending.insert(auth_offset + i);
                }
            }
        }
        auth_offset += filled_tx.spent_utxos.len();
    }
    let spent_utxos = filled_transactions
        .iter()
        .flat_map(|t| t.spent_utxos.iter());
    for (idx, (authorization, spent_utxo)) in
        body.authorizations.iter().zip(spent_utxos).enumerate()
    {
        if swap_claim_pending.contains(&idx) {
            continue;
        }
        if authorization.get_address() != spent_utxo.address {
            return Err(Error::WrongPubKeyForAddress);
        }
    }
    if Authorization::verify_body(body).is_err() {
        return Err(Error::Authorization);
    }
    // Check root consistency without committing to DB
    let () = accumulator.apply_diff(accumulator_diff.clone())?;
    let roots: Vec<BitcoinNodeHash> = accumulator.get_roots();
    if roots != header.roots {
        return Err(Error::UtreexoRootsMismatch);
    }

    Ok(PrevalidatedBlock {
        filled_transactions,
        computed_merkle_root,
        total_fees,
        coinbase_value,
        next_height,
        accumulator_diff,
    })
}

/// Connect a block using the provided prevalidated data.
pub fn connect_prevalidated(
    state: &State,
    rwtxn: &mut RwTxn,
    header: &Header,
    body: &Body,
    pre: PrevalidatedBlock,
) -> Result<crate::types::MerkleRoot, Error> {
    let tip_hash = state.try_get_tip(rwtxn)?;
    if tip_hash != header.prev_side_hash {
        let err = error::InvalidHeader::PrevSideHash {
            expected: tip_hash,
            received: header.prev_side_hash,
        };
        return Err(Error::InvalidHeader(err));
    }
    if pre.computed_merkle_root != header.merkle_root {
        let err = Error::InvalidBody {
            expected: pre.computed_merkle_root,
            computed: header.merkle_root,
        };
        return Err(err);
    }

    // Apply UTXO set changes
    for (vout, output) in body.coinbase.iter().enumerate() {
        let outpoint = OutPoint::Coinbase {
            merkle_root: pre.computed_merkle_root,
            vout: vout as u32,
        };
        state
            .utxos
            .put(rwtxn, &OutPointKey::from(&outpoint), output)
            .map_err(DbError::from)?;
    }

    for filled in &pre.filled_transactions {
        let txid = filled.transaction.txid();
        for (vin, (outpoint, _)) in filled.transaction.inputs.iter().enumerate()
        {
            let spent_output = state
                .utxos
                .try_get(rwtxn, &OutPointKey::from(outpoint))
                .map_err(DbError::from)?
                .ok_or(Error::NoUtxo {
                    outpoint: *outpoint,
                })?;
            state
                .utxos
                .delete(rwtxn, &OutPointKey::from(outpoint))
                .map_err(DbError::from)?;
            let spent_output = SpentOutput {
                output: spent_output,
                inpoint: InPoint::Regular {
                    txid,
                    vin: vin as u32,
                },
            };
            state
                .stxos
                .put(rwtxn, &OutPointKey::from(outpoint), &spent_output)
                .map_err(DbError::from)?;
        }
        for (vout, output) in filled.transaction.outputs.iter().enumerate() {
            let outpoint = OutPoint::Regular {
                txid,
                vout: vout as u32,
            };
            state
                .utxos
                .put(rwtxn, &OutPointKey::from(&outpoint), output)
                .map_err(DbError::from)?;
        }

        // Process swap transactions
        match &filled.transaction.data {
            TxData::SwapCreate {
                swap_id,
                parent_chain,
                l1_txid_bytes,
                required_confirmations,
                l2_recipient,
                l2_amount,
                l1_recipient_address,
                l1_amount,
            } => {
                let swap_id = SwapId(*swap_id);
                let current_height = pre.next_height;

                // Reconstruct L1 txid
                let l1_txid = SwapTxId::from_bytes(l1_txid_bytes);

                // Check if swap already exists (might be from mempool or previous block)
                // If it exists but is corrupted, delete it first to avoid issues
                match state.get_swap(rwtxn, &swap_id) {
                    Ok(Some(ref existing)) => {
                        tracing::warn!(
                            swap_id = %swap_id,
                            existing_state = ?existing.state,
                            "Swap already exists in database, will overwrite during block connection"
                        );
                    }
                    Ok(None) => {
                        // Swap doesn't exist, that's fine
                    }
                    Err(_) => {
                        // Swap exists but is corrupted - delete it first
                        tracing::warn!(
                            swap_id = %swap_id,
                            "Existing swap is corrupted, deleting before saving new one"
                        );
                        // Try to delete the corrupted swap
                        drop(state.swaps.delete(rwtxn, &swap_id));
                    }
                }

                // L2 creator = first input's address (only they may cancel/delete)
                let l2_creator_address =
                    filled.spent_utxos.first().map(|o| o.address);

                // Reconstruct swap object
                let swap = Swap::new(
                    swap_id,
                    crate::types::SwapDirection::L2ToL1,
                    *parent_chain,
                    l1_txid,
                    Some(*required_confirmations),
                    *l2_recipient, // Now optional
                    bitcoin::Amount::from_sat(*l2_amount),
                    l1_recipient_address.clone(),
                    bitcoin::Amount::from_sat(*l1_amount),
                    current_height,
                    Some(
                        current_height
                            + parent_chain.default_swap_expiration_blocks(),
                    ),
                    l2_creator_address,
                );

                // Verify swap ID matches
                if swap.id.0 != swap_id.0 {
                    return Err(Error::InvalidTransaction(
                        "Swap ID mismatch in SwapCreate".to_owned(),
                    ));
                }

                tracing::debug!(
                    swap_id = %swap_id,
                    l2_recipient = ?swap.l2_recipient,
                    l2_amount = %swap.l2_amount,
                    l1_amount = ?swap.l1_amount,
                    state = ?swap.state,
                    "Reconstructed swap from SwapCreate transaction, about to save"
                );

                // Lock outputs for L2 → L1 swaps
                // Only lock outputs with SwapPending content, not change outputs
                for (vout, output) in
                    filled.transaction.outputs.iter().enumerate()
                {
                    // Only lock SwapPending outputs, not regular Value outputs (change)
                    if matches!(
                        output.content,
                        crate::types::OutputContent::SwapPending { .. }
                    ) {
                        let outpoint = OutPoint::Regular {
                            txid,
                            vout: vout as u32,
                        };
                        state
                            .lock_output_to_swap(rwtxn, &outpoint, &swap_id)?;
                    }
                }

                // Save swap - this is where corruption might happen
                tracing::debug!(
                    swap_id = %swap_id,
                    "About to save swap during block connection"
                );
                state.save_swap(rwtxn, &swap)?;
                tracing::debug!(
                    swap_id = %swap_id,
                    "Swap saved during block connection"
                );
            }
            TxData::SwapClaim {
                swap_id,
                l2_claimer_address,
                ..
            } => {
                let swap_id = SwapId(*swap_id);

                // Get swap
                let mut swap = state
                    .get_swap(rwtxn, &swap_id)?
                    .ok_or_else(|| Error::SwapNotFound { swap_id })?;

                // If this node hasn't yet observed the L1 fill (swap
                // state set by local, non-deterministic L1 monitoring),
                // trust the block and advance the swap to ReadyToClaim.
                // The miner who produced the block already validated the
                // L1 side before including this SwapClaim.
                if !matches!(swap.state, SwapState::ReadyToClaim) {
                    tracing::warn!(
                        %swap_id,
                        state = ?swap.state,
                        "SwapClaim in block but local swap not ReadyToClaim; \
                         advancing state to match block"
                    );
                    swap.state = SwapState::ReadyToClaim;
                    state.save_swap(rwtxn, &swap)?;
                }

                // For open swaps, verify claimer address is provided
                if swap.l2_recipient.is_none() && l2_claimer_address.is_none() {
                    return Err(Error::InvalidTransaction(
                        "Open swap claim requires l2_claimer_address"
                            .to_string(),
                    ));
                }

                // Unlock outputs
                for (outpoint, _) in &filled.transaction.inputs {
                    if state.is_output_locked_to_swap(rwtxn, outpoint)?
                        == Some(swap_id)
                    {
                        state.unlock_output_from_swap(rwtxn, outpoint)?;
                    }
                }

                // Mark swap as completed
                swap.mark_completed();
                state.save_swap(rwtxn, &swap)?;
            }
            TxData::Regular => {}
        }
    }

    // Update tip/height
    let block_hash = header.hash();
    state
        .tip
        .put(rwtxn, &(), &block_hash)
        .map_err(DbError::from)?;
    state
        .height
        .put(rwtxn, &(), &pre.next_height)
        .map_err(DbError::from)?;

    // Apply accumulator diff
    let mut accumulator = state
        .utreexo_accumulator
        .try_get(rwtxn, &())
        .map_err(DbError::from)?
        .unwrap_or_default();
    let () = accumulator.apply_diff(pre.accumulator_diff)?;
    state
        .utreexo_accumulator
        .put(rwtxn, &(), &accumulator)
        .map_err(DbError::from)?;

    Ok(pre.computed_merkle_root)
}

pub fn validate(
    state: &State,
    rotxn: &RoTxn,
    header: &Header,
    body: &Body,
) -> Result<(bitcoin::Amount, MerkleRoot), Error> {
    let tip_hash = state.try_get_tip(rotxn)?;
    if header.prev_side_hash != tip_hash {
        let err = error::InvalidHeader::PrevSideHash {
            expected: tip_hash,
            received: header.prev_side_hash,
        };
        return Err(Error::InvalidHeader(err));
    };
    let height = state.try_get_height(rotxn)?.map_or(0, |height| height + 1);
    if body.authorizations.len() > State::body_sigops_limit(height) {
        return Err(Error::TooManySigops);
    }
    let body_size =
        borsh::object_length(&body).map_err(Error::BorshSerialize)?;
    if body_size > State::body_size_limit(height) {
        return Err(Error::BodyTooLarge);
    }
    let mut accumulator = state
        .utreexo_accumulator
        .try_get(rotxn, &())
        .map_err(DbError::from)?
        .unwrap_or_default();
    let filled_transactions: Vec<_> = body
        .transactions
        .iter()
        .map(|t| state.fill_transaction(rotxn, t))
        .collect::<Result<_, _>>()?;
    let merkle_root = Body::compute_merkle_root(
        body.coinbase.as_slice(),
        filled_transactions.as_slice(),
    )?;
    if merkle_root != header.merkle_root {
        let err = Error::InvalidBody {
            expected: header.merkle_root,
            computed: merkle_root,
        };
        return Err(err);
    }
    let mut accumulator_diff = AccumulatorDiff::default();
    let mut coinbase_value = bitcoin::Amount::ZERO;
    for (vout, output) in body.coinbase.iter().enumerate() {
        coinbase_value = coinbase_value
            .checked_add(output.get_value())
            .ok_or(AmountOverflowError)?;
        let outpoint = OutPoint::Coinbase {
            merkle_root,
            vout: vout as u32,
        };
        let pointed_output = PointedOutput {
            outpoint,
            output: output.clone(),
        };
        accumulator_diff.insert((&pointed_output).into());
    }
    let mut total_fees = bitcoin::Amount::ZERO;
    // Gather all input keys to check double-spends via sort-and-scan
    let total_inputs = body.inputs_len();
    let mut all_input_keys = Vec::with_capacity(total_inputs);
    for filled_transaction in &filled_transactions {
        let txid = filled_transaction.transaction.txid();
        // hashes of spent utxos, used to verify the utreexo proof
        let mut spent_utxo_hashes = Vec::<BitcoinNodeHash>::with_capacity(
            filled_transaction.transaction.inputs.len(),
        );
        for (outpoint, utxo_hash) in &filled_transaction.transaction.inputs {
            all_input_keys.push(OutPointKey::from(outpoint));
            spent_utxo_hashes.push(utxo_hash.into());
            accumulator_diff.remove(utxo_hash.into());
        }
        for (vout, output) in
            filled_transaction.transaction.outputs.iter().enumerate()
        {
            let outpoint = OutPoint::Regular {
                txid,
                vout: vout as u32,
            };
            let pointed_output = PointedOutput {
                outpoint,
                output: output.clone(),
            };
            accumulator_diff.insert((&pointed_output).into());
        }
        // Swap-specific validation (see the matching block in `prevalidate`).
        // Without this, SwapClaim transactions — whose SwapPending inputs skip
        // the authorization/ownership check below — would be entirely
        // unvalidated at block time, letting anyone drain locked swap funds.
        match &filled_transaction.transaction.data {
            TxData::SwapCreate { .. } => {
                crate::state::swap::validate_swap_create(
                    state,
                    rotxn,
                    &filled_transaction.transaction,
                    filled_transaction,
                )?;
            }
            TxData::SwapClaim { .. } => {
                crate::state::swap::validate_swap_claim(
                    state,
                    rotxn,
                    &filled_transaction.transaction,
                    filled_transaction,
                )?;
            }
            TxData::Regular => {
                crate::state::swap::validate_no_locked_outputs(
                    state,
                    rotxn,
                    &filled_transaction.transaction,
                )?;
            }
        }
        total_fees = total_fees
            .checked_add(state.validate_filled_transaction(filled_transaction)?)
            .ok_or(AmountOverflowError)?;
        // verify utreexo proof
        if !accumulator
            .verify(&filled_transaction.transaction.proof, &spent_utxo_hashes)?
        {
            return Err(Error::UtreexoProofFailed { txid });
        }
    }
    // Sort and check for duplicate outpoints (double-spend detection)
    {
        use rayon::prelude::ParallelSliceMut;
        all_input_keys.par_sort_unstable();
        if all_input_keys.windows(2).any(|w| w[0] == w[1]) {
            return Err(Error::UtxoDoubleSpent);
        }
    }
    if coinbase_value > total_fees {
        return Err(Error::NotEnoughFees);
    }
    // Same SwapClaim + SwapPending skip as in prevalidate.
    let mut auth_offset = 0usize;
    let mut swap_claim_pending = std::collections::HashSet::<usize>::new();
    for filled_tx in &filled_transactions {
        if matches!(filled_tx.transaction.data, TxData::SwapClaim { .. }) {
            for (i, utxo) in filled_tx.spent_utxos.iter().enumerate() {
                if utxo.content.is_swap_pending() {
                    swap_claim_pending.insert(auth_offset + i);
                }
            }
        }
        auth_offset += filled_tx.spent_utxos.len();
    }
    let spent_utxos = filled_transactions
        .iter()
        .flat_map(|t| t.spent_utxos.iter());
    for (idx, (authorization, spent_utxo)) in
        body.authorizations.iter().zip(spent_utxos).enumerate()
    {
        if swap_claim_pending.contains(&idx) {
            continue;
        }
        if authorization.get_address() != spent_utxo.address {
            return Err(Error::WrongPubKeyForAddress);
        }
    }
    if Authorization::verify_body(body).is_err() {
        return Err(Error::Authorization);
    }
    // Check root consistency without committing to DB
    let () = accumulator.apply_diff(accumulator_diff)?;
    let roots: Vec<BitcoinNodeHash> = accumulator.get_roots();
    if roots != header.roots {
        return Err(Error::UtreexoRootsMismatch);
    }
    Ok((total_fees, merkle_root))
}

pub fn connect(
    state: &State,
    rwtxn: &mut RwTxn,
    header: &Header,
    body: &Body,
) -> Result<MerkleRoot, Error> {
    let tip_hash = state.try_get_tip(rwtxn)?;
    if tip_hash != header.prev_side_hash {
        let err = error::InvalidHeader::PrevSideHash {
            expected: tip_hash,
            received: header.prev_side_hash,
        };
        return Err(Error::InvalidHeader(err));
    }
    let mut accumulator = state
        .utreexo_accumulator
        .try_get(rwtxn, &())?
        .unwrap_or_default();
    let mut accumulator_diff = AccumulatorDiff::default();
    for (vout, output) in body.coinbase.iter().enumerate() {
        let outpoint = OutPoint::Coinbase {
            merkle_root: header.merkle_root,
            vout: vout as u32,
        };
        let pointed_output = PointedOutput {
            outpoint,
            output: output.clone(),
        };
        accumulator_diff.insert((&pointed_output).into());
        let key = OutPointKey::from(&outpoint);
        state.utxos.put(rwtxn, &key, output)?;
    }
    let mut filled_txs: Vec<FilledTransaction> =
        Vec::with_capacity(body.transactions.len());
    for transaction in &body.transactions {
        let mut spent_utxos = Vec::with_capacity(transaction.inputs.len());
        let txid = transaction.txid();
        for (vin, (outpoint, utxo_hash)) in
            transaction.inputs.iter().enumerate()
        {
            let key = OutPointKey::from(outpoint);
            let spent_output =
                state.utxos.try_get(rwtxn, &key)?.ok_or(Error::NoUtxo {
                    outpoint: *outpoint,
                })?;

            accumulator_diff.remove(utxo_hash.into());
            let key = OutPointKey::from(outpoint);
            state.utxos.delete(rwtxn, &key)?;
            let spent_output = SpentOutput {
                output: spent_output,
                inpoint: InPoint::Regular {
                    txid,
                    vin: vin as u32,
                },
            };
            state
                .stxos
                .put(rwtxn, &OutPointKey::from(outpoint), &spent_output)
                .map_err(DbError::from)?;
            spent_utxos.push(spent_output.output);
        }
        for (vout, output) in transaction.outputs.iter().enumerate() {
            let outpoint = OutPoint::Regular {
                txid,
                vout: vout as u32,
            };
            let pointed_output = PointedOutput {
                outpoint,
                output: output.clone(),
            };
            accumulator_diff.insert((&pointed_output).into());
            let key = OutPointKey::from(&outpoint);
            state.utxos.put(rwtxn, &key, output)?;
        }
        let filled_tx = FilledTransaction {
            spent_utxos,
            transaction: transaction.clone(),
        };
        filled_txs.push(filled_tx);
    }
    let merkle_root = Body::compute_merkle_root(
        body.coinbase.as_slice(),
        filled_txs.as_slice(),
    )?;
    if merkle_root != header.merkle_root {
        let err = Error::InvalidBody {
            expected: header.merkle_root,
            computed: merkle_root,
        };
        return Err(err);
    }
    let block_hash = header.hash();
    let height = state.try_get_height(rwtxn)?.map_or(0, |height| height + 1);
    state.tip.put(rwtxn, &(), &block_hash)?;
    state.height.put(rwtxn, &(), &height)?;
    let () = accumulator.apply_diff(accumulator_diff)?;
    state.utreexo_accumulator.put(rwtxn, &(), &accumulator)?;
    Ok(merkle_root)
}


pub fn disconnect_tip(
    state: &State,
    rwtxn: &mut RwTxn,
    header: &Header,
    body: &Body,
) -> Result<(), Error> {
    let tip_hash = state
        .tip
        .try_get(rwtxn, &())
        .map_err(DbError::from)?
        .ok_or(Error::NoTip)?;
    if tip_hash != header.hash() {
        let err = error::InvalidHeader::BlockHash {
            expected: tip_hash,
            computed: header.hash(),
        };
        return Err(Error::InvalidHeader(err));
    }
    let mut accumulator = state
        .utreexo_accumulator
        .try_get(rwtxn, &())
        .map_err(DbError::from)?
        .unwrap_or_default();
    tracing::debug!("Got acc");
    let mut accumulator_diff = AccumulatorDiff::default();
    // revert txs, last-to-first
    body.transactions.iter().rev().try_for_each(|tx| {
        let txid = tx.txid();

        // Rollback swap transactions
        match &tx.data {
            TxData::SwapCreate { swap_id, .. } => {
                let swap_id = SwapId(*swap_id);

                // Unlock outputs for L2 → L1 swaps
                // Only unlock SwapPending outputs that were locked
                for (vout, output) in tx.outputs.iter().enumerate().rev() {
                    if matches!(
                        output.content,
                        crate::types::OutputContent::SwapPending { .. }
                    ) {
                        let outpoint = OutPoint::Regular {
                            txid,
                            vout: vout as u32,
                        };
                        if state.is_output_locked_to_swap(rwtxn, &outpoint)?
                            == Some(swap_id)
                        {
                            state.unlock_output_from_swap(rwtxn, &outpoint)?;
                        }
                    }
                }

                // Delete swap (rollback: no creator check)
                state.delete_swap_unchecked(rwtxn, &swap_id)?;
            }
            TxData::SwapClaim { swap_id, .. } => {
                let swap_id = SwapId(*swap_id);

                // Get swap
                let mut swap = state
                    .get_swap(rwtxn, &swap_id)?
                    .ok_or_else(|| Error::SwapNotFound { swap_id })?;

                // Re-lock outputs
                for (outpoint, _) in tx.inputs.iter().rev() {
                    if state
                        .is_output_locked_to_swap(rwtxn, outpoint)?
                        .is_none()
                    {
                        state.lock_output_to_swap(rwtxn, outpoint, &swap_id)?;
                    }
                }

                // Revert swap state
                if matches!(swap.state, SwapState::Completed) {
                    swap.state = SwapState::ReadyToClaim;
                    state.save_swap(rwtxn, &swap)?;
                }
            }
            TxData::Regular => {}
        }

        // delete UTXOs, last-to-first
        tx.outputs.iter().enumerate().rev().try_for_each(
            |(vout, output)| {
                let outpoint = OutPoint::Regular {
                    txid,
                    vout: vout as u32,
                };
                let pointed_output = PointedOutput {
                    outpoint,
                    output: output.clone(),
                };
                accumulator_diff.remove((&pointed_output).into());
                let key = OutPointKey::from(&outpoint);
                if state.utxos.delete(rwtxn, &key).map_err(DbError::from)? {
                    Ok(())
                } else {
                    Err(Error::NoUtxo { outpoint })
                }
            },
        )?;
        // unspend STXOs, last-to-first
        tx.inputs
            .iter()
            .rev()
            .try_for_each(|(outpoint, utxo_hash)| {
                let key = OutPointKey::from(outpoint);
                if let Some(spent_output) = state.stxos.try_get(rwtxn, &key)? {
                    accumulator_diff.insert(utxo_hash.into());
                    state.stxos.delete(rwtxn, &key)?;
                    state.utxos.put(rwtxn, &key, &spent_output.output)?;
                    Ok(())
                } else {
                    Err(Error::NoStxo {
                        outpoint: *outpoint,
                    })
                }
            })
    })?;
    // delete coinbase UTXOs, last-to-first
    body.coinbase
        .iter()
        .enumerate()
        .rev()
        .try_for_each(|(vout, output)| {
            let outpoint = OutPoint::Coinbase {
                merkle_root: header.merkle_root,
                vout: vout as u32,
            };
            let pointed_output = PointedOutput {
                outpoint,
                output: output.clone(),
            };
            accumulator_diff.remove((&pointed_output).into());
            let key = OutPointKey::from(&outpoint);
            if state.utxos.delete(rwtxn, &key).map_err(DbError::from)? {
                Ok(())
            } else {
                Err(Error::NoUtxo { outpoint })
            }
        })?;
    let height = state
        .try_get_height(rwtxn)?
        .expect("Height should not be None");
    match (header.prev_side_hash, height) {
        (None, 0) => {
            state.tip.delete(rwtxn, &()).map_err(DbError::from)?;
            state.height.delete(rwtxn, &()).map_err(DbError::from)?;
        }
        (None, _) | (_, 0) => return Err(Error::NoTip),
        (Some(prev_side_hash), height) => {
            state
                .tip
                .put(rwtxn, &(), &prev_side_hash)
                .map_err(DbError::from)?;
            state
                .height
                .put(rwtxn, &(), &(height - 1))
                .map_err(DbError::from)?;
        }
    }
    let () = accumulator.apply_diff(accumulator_diff)?;
    state
        .utreexo_accumulator
        .put(rwtxn, &(), &accumulator)
        .map_err(DbError::from)?;
    Ok(())
}

#[cfg(test)]
mod block_swap_claim_tests {
    //! End-to-end block-level tests for the SwapClaim payout-binding fix.
    //!
    //! These build a real block — utreexo accumulator + proof, signed
    //! authorization, computed merkle root and roots — containing a single
    //! SwapClaim that spends a locked SwapPending output, and drive it through
    //! `prevalidate` (the live block-validation entrypoint used by the node).
    //!
    //! The exploit being guarded against: a block producer spends the locked
    //! funds (the SwapPending input's ownership check is skipped for claims),
    //! pays the swap recipient a token amount, and keeps the rest. Before the
    //! fix, `prevalidate` never ran `validate_swap_claim`, so such a block was
    //! accepted.

    use super::*;
    use crate::authorization::{self, SigningKey};
    use crate::types::{
        Accumulator, Address, Output, OutputContent, ParentChainType,
        SwapDirection, Transaction, Txid, hash,
    };
    use bitcoin::hashes::Hash as _;

    const LOCKED: u64 = 1_000_000;

    fn open_env() -> (tempfile::TempDir, sneed::Env) {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut opts = heed::EnvOpenOptions::new();
        opts.map_size(64 * 1024 * 1024).max_dbs(State::NUM_DBS);
        let env =
            unsafe { sneed::Env::open(&opts, dir.path()) }.expect("open env");
        (dir, env)
    }

    fn recipient_addr() -> Address {
        Address([0x11; 20])
    }

    fn attacker_addr() -> Address {
        Address([0x22; 20])
    }

    /// Holds everything needed to assemble a claim block against a state that
    /// already contains one ReadyToClaim swap with a locked SwapPending UTXO.
    struct Fixture {
        _dir: tempfile::TempDir,
        env: sneed::Env,
        state: State,
        swap_id: SwapId,
        key: SigningKey,
        outpoint: OutPoint,
        locked_output: Output,
        utxo_hash: crate::types::Hash,
        leaf: BitcoinNodeHash,
        /// Pre-block accumulator (single leaf), used to generate proofs.
        acc: Accumulator,
    }

    fn setup() -> Fixture {
        let (dir, env) = open_env();
        let state = State::new(&env).expect("state");
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let swap_id = SwapId([0x42; 32]);

        let outpoint = OutPoint::Regular {
            txid: Txid([0xAB; 32]),
            vout: 0,
        };
        let locked_output = Output {
            address: attacker_addr(),
            content: OutputContent::SwapPending {
                value: bitcoin::Amount::from_sat(LOCKED),
                swap_id: swap_id.0,
            },
        };
        let pointed = PointedOutput {
            outpoint,
            output: locked_output.clone(),
        };
        let utxo_hash = hash(&pointed);
        let leaf: BitcoinNodeHash = (&pointed).into();

        // Build the pre-block accumulator with the single locked leaf.
        let mut acc = Accumulator::default();
        let mut add = AccumulatorDiff::default();
        add.insert(leaf);
        acc.apply_diff(add).expect("seed accumulator");

        // Persist UTXO, accumulator, swap, and lock.
        let mut swap = Swap::new(
            swap_id,
            SwapDirection::L2ToL1,
            ParentChainType::Regtest,
            SwapTxId::Hash32([0u8; 32]),
            None,
            Some(recipient_addr()),
            bitcoin::Amount::from_sat(LOCKED),
            "tb1qtest".to_string(),
            bitcoin::Amount::from_sat(500_000),
            0,
            None,
            None,
        );
        swap.state = SwapState::ReadyToClaim;

        let mut rwtxn = env.write_txn().expect("write txn");
        state
            .utxos
            .put(&mut rwtxn, &OutPointKey::from(&outpoint), &locked_output)
            .expect("put utxo");
        state
            .utreexo_accumulator
            .put(&mut rwtxn, &(), &acc)
            .expect("put accumulator");
        state.save_swap(&mut rwtxn, &swap).expect("save swap");
        state
            .lock_output_to_swap(&mut rwtxn, &outpoint, &swap_id)
            .expect("lock output");
        rwtxn.commit().expect("commit");

        Fixture {
            _dir: dir,
            env,
            state,
            swap_id,
            key,
            outpoint,
            locked_output,
            utxo_hash,
            leaf,
            acc,
        }
    }

    impl Fixture {
        /// Assemble a fully-valid block (proof, signature, merkle root, roots)
        /// whose single SwapClaim pays the given outputs.
        fn build_block(&self, outputs: Vec<Output>) -> (Header, Body) {
            let proof =
                self.acc.prove(&[self.leaf]).expect("prove locked leaf");
            let tx = Transaction {
                inputs: vec![(self.outpoint, self.utxo_hash)],
                proof,
                outputs,
                data: TxData::SwapClaim {
                    swap_id: self.swap_id.0,
                    l2_claimer_address: None,
                    proof_data: None,
                },
            };

            // One authorization per input, signed by the attacker key. The
            // signature is valid ed25519; only the address->utxo match is
            // skipped for SwapPending inputs.
            let signature =
                authorization::sign(&self.key, &tx).expect("sign tx");
            let authorizations = vec![Authorization {
                verifying_key: self.key.verifying_key(),
                signature,
            }];

            let coinbase: Vec<Output> = Vec::new();
            let filled = FilledTransaction {
                spent_utxos: vec![self.locked_output.clone()],
                transaction: tx.clone(),
            };
            let merkle_root =
                Body::compute_merkle_root(&coinbase, std::slice::from_ref(&filled))
                    .expect("merkle root");

            // Compute post-block roots: rebuild a single-leaf accumulator and
            // apply the same diff prevalidate will (remove input, insert
            // outputs). Coinbase is empty.
            let mut post = Accumulator::default();
            let mut seed = AccumulatorDiff::default();
            seed.insert(self.leaf);
            post.apply_diff(seed).expect("seed post acc");
            let mut diff = AccumulatorDiff::default();
            diff.remove(self.leaf);
            let txid = tx.txid();
            for (vout, output) in tx.outputs.iter().enumerate() {
                let pointed = PointedOutput {
                    outpoint: OutPoint::Regular {
                        txid,
                        vout: vout as u32,
                    },
                    output: output.clone(),
                };
                diff.insert((&pointed).into());
            }
            post.apply_diff(diff).expect("apply block diff");
            let roots = post.get_roots();

            let header = Header {
                merkle_root,
                prev_side_hash: None,
                prev_main_hash: bitcoin::BlockHash::all_zeros(),
                roots,
            };
            let body = Body {
                coinbase,
                transactions: vec![tx],
                authorizations,
            };
            (header, body)
        }

        fn value_out(addr: Address, sats: u64) -> Output {
            Output {
                address: addr,
                content: OutputContent::Value(bitcoin::Amount::from_sat(sats)),
            }
        }
    }

    /// The exploit at the block level: pay recipient 1 sat, keep the rest.
    /// `prevalidate` must reject it.
    #[test]
    fn prevalidate_rejects_underpaying_claim_block() {
        let fx = setup();
        let (header, body) = fx.build_block(vec![
            Fixture::value_out(recipient_addr(), 1),
            Fixture::value_out(attacker_addr(), LOCKED - 1),
        ]);
        let rotxn = fx.env.read_txn().expect("read txn");
        let err = prevalidate(&fx.state, &rotxn, &header, &body)
            .expect_err("underpaying claim block must be rejected");
        assert!(
            format!("{err}").contains("must pay the full locked amount"),
            "unexpected error: {err}"
        );
    }

    /// An honest claim block paying the full locked amount to the recipient
    /// must pass full prevalidation (proof, swap rules, merkle root, auth,
    /// roots).
    #[test]
    fn prevalidate_accepts_full_payout_claim_block() {
        let fx = setup();
        let (header, body) =
            fx.build_block(vec![Fixture::value_out(recipient_addr(), LOCKED)]);
        let rotxn = fx.env.read_txn().expect("read txn");
        prevalidate(&fx.state, &rotxn, &header, &body)
            .expect("full-payout claim block must be accepted");
    }
}
