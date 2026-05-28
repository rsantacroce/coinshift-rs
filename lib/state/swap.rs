//! Swap validation and processing

use sneed::RoTxn;

use crate::{
    state::{Error, State},
    types::{
        FilledTransaction, SwapId, SwapState, SwapTxId, Transaction, TxData,
    },
};

/// Validate a SwapCreate transaction
pub fn validate_swap_create(
    state: &State,
    rotxn: &RoTxn,
    transaction: &Transaction,
    filled_transaction: &FilledTransaction,
) -> Result<(), Error> {
    let TxData::SwapCreate {
        swap_id,
        parent_chain: _,
        l1_txid_bytes: _,
        required_confirmations: _,
        l2_recipient,
        l2_amount,
        l1_recipient_address,
        l1_amount,
    } = &transaction.data
    else {
        return Err(Error::InvalidTransaction(
            "Expected SwapCreate transaction".to_string(),
        ));
    };

    // 1. Verify swap ID matches computed ID
    let computed_swap_id = {
        // L2 → L1 swap
        // We need the sender's address - get it from the first input
        let first_input =
            filled_transaction.spent_utxos.first().ok_or_else(|| {
                Error::InvalidTransaction(
                    "SwapCreate must have inputs".to_string(),
                )
            })?;
        let l2_sender_address = first_input.address;
        SwapId::from_l2_to_l1(
            l1_recipient_address,
            bitcoin::Amount::from_sat(*l1_amount),
            &l2_sender_address,
            l2_recipient.as_ref(), // Now optional
        )
    };

    if computed_swap_id.0 != *swap_id {
        return Err(Error::InvalidTransaction(format!(
            "Swap ID mismatch: expected {}, computed {}",
            hex::encode(swap_id),
            computed_swap_id
        )));
    }

    // 2. Verify swap doesn't already exist
    if state.get_swap(rotxn, &computed_swap_id)?.is_some() {
        return Err(Error::InvalidTransaction(format!(
            "Swap {} already exists",
            computed_swap_id
        )));
    }

    // 3. Verify l2_amount > 0
    if *l2_amount == 0 {
        return Err(Error::InvalidTransaction(
            "L2 amount must be greater than zero".to_string(),
        ));
    }

    // 4. Verify transaction has outputs
    if transaction.outputs.is_empty() {
        return Err(Error::InvalidTransaction(
            "Transaction must have at least one output".to_string(),
        ));
    }

    // 5. For L2 → L1 swaps, verify inputs aren't locked and sufficient funds
    // Check that no inputs are locked to another swap
    for (outpoint, _) in &transaction.inputs {
        if let Some(locked_swap_id) =
            state.is_output_locked_to_swap(rotxn, outpoint)?
            && locked_swap_id.0 != *swap_id
        {
            // Check if the locked swap exists and is valid
            match state.get_swap(rotxn, &locked_swap_id) {
                Ok(Some(_)) => {
                    // Swap exists and is valid - this is a real lock
                    return Err(Error::InvalidTransaction(format!(
                        "Input {} is locked to swap {}",
                        outpoint, locked_swap_id
                    )));
                }
                Ok(None) => {
                    // Swap doesn't exist - orphaned lock
                    return Err(Error::InvalidTransaction(format!(
                        "Input {} is locked to non-existent swap {} (orphaned lock). Please run cleanup_orphaned_locks to fix this.",
                        outpoint, locked_swap_id
                    )));
                }
                Err(err) => {
                    // Check if it's a deserialization error (corrupted swap)
                    let err_str = format!("{err:#}");
                    let err_debug = format!("{err:?}");
                    let is_deserialization_error = err_str.contains("Decoding")
                        || err_str.contains("InvalidTagEncoding")
                        || err_str.contains("deserialize")
                        || err_str.contains("bincode")
                        || err_str.contains("Borsh")
                        || err_debug.contains("Decoding")
                        || err_debug.contains("InvalidTagEncoding")
                        || err_debug.contains("deserialize");

                    if is_deserialization_error {
                        // Swap is corrupted - orphaned lock
                        return Err(Error::InvalidTransaction(format!(
                            "Input {} is locked to corrupted swap {} (orphaned lock). Please run cleanup_orphaned_locks to fix this.",
                            outpoint, locked_swap_id
                        )));
                    } else {
                        // Other database error - return original error
                        return Err(Error::InvalidTransaction(format!(
                            "Input {} is locked to swap {}, but error checking swap: {}",
                            outpoint, locked_swap_id, err
                        )));
                    }
                }
            }
        }
    }

    // Verify transaction spends at least l2_amount
    let total_input_value = filled_transaction
        .spent_utxos
        .iter()
        .map(crate::types::GetValue::get_value)
        .try_fold(bitcoin::Amount::ZERO, |acc, val| {
            acc.checked_add(val).ok_or(())
        })
        .map_err(|_| {
            Error::InvalidTransaction("Input value overflow".to_string())
        })?;

    let required_amount = bitcoin::Amount::from_sat(*l2_amount);
    if total_input_value < required_amount {
        return Err(Error::InvalidTransaction(format!(
            "Insufficient funds: need {}, have {}",
            required_amount, total_input_value
        )));
    }

    Ok(())
}

/// Validate a SwapClaim transaction
pub fn validate_swap_claim(
    state: &State,
    rotxn: &RoTxn,
    transaction: &Transaction,
    filled_transaction: &FilledTransaction,
) -> Result<(), Error> {
    let TxData::SwapClaim { swap_id, .. } = &transaction.data else {
        return Err(Error::InvalidTransaction(
            "Expected SwapClaim transaction".to_string(),
        ));
    };

    let swap_id = SwapId(*swap_id);

    // 1. Verify swap exists
    let swap = state
        .get_swap(rotxn, &swap_id)?
        .ok_or_else(|| Error::SwapNotFound { swap_id })?;

    // 2. Verify swap is in ReadyToClaim state
    if !matches!(swap.state, SwapState::ReadyToClaim) {
        return Err(Error::InvalidTransaction(format!(
            "Swap {} is not ready to claim (state: {:?})",
            swap_id, swap.state
        )));
    }

    // 2.5. For open swaps, verify L1 transaction exists (someone filled it)
    if swap.l2_recipient.is_none() {
        // Open swap - verify L1 transaction was detected
        let zero_hash32 = [0u8; 32];
        let has_l1_tx = !matches!(swap.l1_txid, SwapTxId::Hash32(h) if h == zero_hash32)
            && !matches!(swap.l1_txid, SwapTxId::Hash(ref v) if v.is_empty() || v.iter().all(|&b| b == 0));

        if !has_l1_tx {
            return Err(Error::InvalidTransaction(
                "Open swap cannot be claimed until L1 transaction is detected"
                    .to_string(),
            ));
        }
    }

    // 3. Verify at least one input is locked to this swap, and accumulate the
    //    total value of the locked (SwapPending) outputs being claimed. This
    //    is the amount that MUST be paid through to the swap recipient — see
    //    step 5. `filled_transaction.spent_utxos` is index-aligned with
    //    `transaction.inputs`.
    use crate::types::GetValue as _;
    let mut found_locked_input = false;
    let mut locked_input_value = bitcoin::Amount::ZERO;
    for (idx, (outpoint, _)) in transaction.inputs.iter().enumerate() {
        if let Some(locked_swap_id) =
            state.is_output_locked_to_swap(rotxn, outpoint)?
        {
            if locked_swap_id != swap_id {
                return Err(Error::InvalidTransaction(format!(
                    "Input {} is locked to different swap {}",
                    outpoint, locked_swap_id
                )));
            }
            found_locked_input = true;

            let spent_utxo =
                filled_transaction.spent_utxos.get(idx).ok_or_else(|| {
                    Error::InvalidTransaction(
                        "SwapClaim inputs do not match spent UTXOs".to_string(),
                    )
                })?;
            locked_input_value = locked_input_value
                .checked_add(spent_utxo.get_value())
                .ok_or_else(|| {
                    Error::InvalidTransaction(
                        "Locked input value overflow".to_string(),
                    )
                })?;
        }
    }

    if !found_locked_input {
        return Err(Error::InvalidTransaction(
            "SwapClaim must spend at least one output locked to the swap"
                .to_string(),
        ));
    }

    // 4. Verify output goes to correct recipient
    let TxData::SwapClaim {
        l2_claimer_address, ..
    } = &transaction.data
    else {
        unreachable!()
    };

    let expected_recipient = if let Some(recipient) = swap.l2_recipient {
        // Pre-specified swap: must go to specified recipient
        recipient
    } else {
        // Open swap
        if let Some(stored_l2) = swap.l2_claimer_address {
            // Claim only valid for the L2 address the filler declared when providing L1 tx details
            if l2_claimer_address.as_ref() != Some(&stored_l2) {
                return Err(Error::InvalidTransaction(
                    "Open swap claim must use the L2 address declared when the L1 transaction was submitted".to_string(),
                ));
            }
            stored_l2
        } else if let Some(claimer_addr) = l2_claimer_address {
            // No stored L2 (e.g. auto-detected L1 tx): accept claimer address from tx
            *claimer_addr
        } else {
            return Err(Error::InvalidTransaction(
                "Open swap claim requires l2_claimer_address".to_string(),
            ));
        }
    };

    // 5. Bind the payout AMOUNT, not just the recipient address. The claimer
    //    must pay through the full value of the locked outputs to the swap
    //    recipient. Without this, anyone could spend the locked funds while
    //    paying the recipient a token amount (e.g. 1 sat) and keep the rest.
    let recipient_total = transaction
        .outputs
        .iter()
        .filter(|output| output.address == expected_recipient)
        .map(crate::types::GetValue::get_value)
        .try_fold(bitcoin::Amount::ZERO, |acc, val| acc.checked_add(val))
        .ok_or_else(|| {
            Error::InvalidTransaction(
                "Recipient output value overflow".to_string(),
            )
        })?;

    if recipient_total < locked_input_value {
        return Err(Error::InvalidTransaction(format!(
            "SwapClaim must pay the full locked amount to {}: required {}, paid {}",
            expected_recipient, locked_input_value, recipient_total
        )));
    }

    Ok(())
}


/// Validate that non-SwapClaim transactions don't spend locked outputs
pub fn validate_no_locked_outputs(
    state: &State,
    rotxn: &RoTxn,
    transaction: &Transaction,
) -> Result<(), Error> {
    // Skip validation for SwapClaim transactions
    if matches!(transaction.data, TxData::SwapClaim { .. }) {
        return Ok(());
    }

    // Check that no inputs are locked
    for (outpoint, _) in &transaction.inputs {
        if let Some(locked_swap_id) =
            state.is_output_locked_to_swap(rotxn, outpoint)?
        {
            return Err(Error::InvalidTransaction(format!(
                "Cannot spend locked output {} (locked to swap {})",
                outpoint, locked_swap_id
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod claim_amount_tests {
    //! Tests for the SwapClaim payout-amount binding (the fix for the
    //! "SwapClaim does not bind the payout amount" vulnerability).
    //!
    //! Before the fix, `validate_swap_claim` only checked that *some* output
    //! paid the recipient address, ignoring the amount. Anyone could spend the
    //! locked funds while paying the recipient a token amount (e.g. 1 sat) and
    //! keep the rest. These tests assert that the recipient must receive at
    //! least the full value of the locked (SwapPending) inputs.

    use super::*;
    use crate::types::{
        Address, FilledTransaction, OutPoint, Output, OutputContent as Content,
        ParentChainType, Swap, SwapDirection, SwapId, SwapState, SwapTxId,
        Transaction, TxData, Txid,
    };

    const LOCKED_VALUE: u64 = 1_000_000;

    fn test_env() -> (tempfile::TempDir, sneed::Env) {
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

    /// The single outpoint that is locked to the swap in every test.
    fn locked_outpoint() -> OutPoint {
        OutPoint::Regular {
            txid: Txid([0xAB; 32]),
            vout: 0,
        }
    }

    /// A ReadyToClaim, pre-specified L2→L1 swap with one locked SwapPending
    /// output worth `LOCKED_VALUE`. Returns the constructed `State`.
    fn setup_ready_swap(env: &sneed::Env) -> (State, SwapId) {
        let state = State::new(env).expect("state");
        let swap_id = SwapId([0x42; 32]);

        let mut swap = Swap::new(
            swap_id,
            SwapDirection::L2ToL1,
            ParentChainType::Regtest,
            SwapTxId::Hash32([0u8; 32]),
            None,
            Some(recipient_addr()), // pre-specified recipient
            bitcoin::Amount::from_sat(LOCKED_VALUE),
            "tb1qtest".to_string(),
            bitcoin::Amount::from_sat(500_000),
            0,
            None,
            None,
        );
        swap.state = SwapState::ReadyToClaim;

        let mut rwtxn = env.write_txn().expect("write txn");
        state.save_swap(&mut rwtxn, &swap).expect("save swap");
        state
            .lock_output_to_swap(&mut rwtxn, &locked_outpoint(), &swap_id)
            .expect("lock output");
        rwtxn.commit().expect("commit");

        (state, swap_id)
    }

    /// Build a SwapClaim transaction + matching FilledTransaction. The single
    /// input is the locked SwapPending output worth `LOCKED_VALUE`; `outputs`
    /// are the claim's payout outputs.
    fn build_claim(
        swap_id: SwapId,
        outputs: Vec<Output>,
    ) -> (Transaction, FilledTransaction) {
        let tx = Transaction {
            inputs: vec![(locked_outpoint(), [0u8; 32])],
            proof: Default::default(),
            outputs,
            data: TxData::SwapClaim {
                swap_id: swap_id.0,
                l2_claimer_address: None,
                proof_data: None,
            },
        };
        let spent = Output {
            address: attacker_addr(), // creator-owned in reality; irrelevant here
            content: Content::SwapPending {
                value: bitcoin::Amount::from_sat(LOCKED_VALUE),
                swap_id: swap_id.0,
            },
        };
        let filled = FilledTransaction {
            spent_utxos: vec![spent],
            transaction: tx.clone(),
        };
        (tx, filled)
    }

    fn value_out(addr: Address, sats: u64) -> Output {
        Output {
            address: addr,
            content: Content::Value(bitcoin::Amount::from_sat(sats)),
        }
    }

    /// The exploit: pay the recipient 1 sat, keep the rest. Must be rejected.
    #[test]
    fn rejects_underpaying_recipient() {
        let (_dir, env) = test_env();
        let (state, swap_id) = setup_ready_swap(&env);
        let rotxn = env.read_txn().expect("read txn");

        let (tx, filled) = build_claim(
            swap_id,
            vec![
                value_out(recipient_addr(), 1),
                value_out(attacker_addr(), LOCKED_VALUE - 1),
            ],
        );
        let result = validate_swap_claim(&state, &rotxn, &tx, &filled);
        let err = result.expect_err("underpaying claim must be rejected");
        assert!(
            format!("{err}").contains("must pay the full locked amount"),
            "unexpected error: {err}"
        );
    }

    /// Recipient gets nothing at all. Must be rejected.
    #[test]
    fn rejects_recipient_with_no_output() {
        let (_dir, env) = test_env();
        let (state, swap_id) = setup_ready_swap(&env);
        let rotxn = env.read_txn().expect("read txn");

        let (tx, filled) =
            build_claim(swap_id, vec![value_out(attacker_addr(), LOCKED_VALUE)]);
        assert!(validate_swap_claim(&state, &rotxn, &tx, &filled).is_err());
    }

    /// Honest claim paying the full locked amount in one output. Must pass.
    #[test]
    fn accepts_full_payout() {
        let (_dir, env) = test_env();
        let (state, swap_id) = setup_ready_swap(&env);
        let rotxn = env.read_txn().expect("read txn");

        let (tx, filled) = build_claim(
            swap_id,
            vec![value_out(recipient_addr(), LOCKED_VALUE)],
        );
        validate_swap_claim(&state, &rotxn, &tx, &filled)
            .expect("full payout must be accepted");
    }

    /// Honest claim paying the full amount split across multiple outputs to the
    /// recipient. Must pass (amounts are summed).
    #[test]
    fn accepts_full_payout_split_across_outputs() {
        let (_dir, env) = test_env();
        let (state, swap_id) = setup_ready_swap(&env);
        let rotxn = env.read_txn().expect("read txn");

        let (tx, filled) = build_claim(
            swap_id,
            vec![
                value_out(recipient_addr(), 400_000),
                value_out(recipient_addr(), LOCKED_VALUE - 400_000),
            ],
        );
        validate_swap_claim(&state, &rotxn, &tx, &filled)
            .expect("split full payout must be accepted");
    }

    /// Overpaying the recipient (more than locked) is allowed.
    #[test]
    fn accepts_overpayment() {
        let (_dir, env) = test_env();
        let (state, swap_id) = setup_ready_swap(&env);
        let rotxn = env.read_txn().expect("read txn");

        let (tx, filled) = build_claim(
            swap_id,
            vec![value_out(recipient_addr(), LOCKED_VALUE + 50_000)],
        );
        validate_swap_claim(&state, &rotxn, &tx, &filled)
            .expect("overpayment must be accepted");
    }
}
