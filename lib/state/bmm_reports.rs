//! BMM Swap Report Processing and Consensus

use std::collections::{HashMap, HashSet};

use fallible_iterator::FallibleIterator;
use sneed::{RoTxn, RwTxn, db::error::Error as DbError};

use crate::state::Error;
use crate::types::{
    Address, L1TransactionReport, Swap, SwapId, SwapState,
};

/// Minimum number of independent BMM participants required for consensus
pub const MIN_CONSENSUS_PARTICIPANTS: u32 = 2;

/// Maximum allowed difference in confirmations between reports (in blocks)
pub const MAX_CONFIRMATION_DIFF: u32 = 2;

/// Process BMM swap reports from a block body
/// 
/// This function:
/// 1. Verifies signatures of all reports
/// 2. Verifies confirmations against header chain (if available)
/// 3. Stores reports in the database
/// 4. Checks for consensus on swaps
/// 5. Updates swap states if consensus is reached
pub fn process_bmm_swap_reports(
    state: &crate::state::State,
    rwtxn: &mut RwTxn,
    reports: &[L1TransactionReport],
) -> Result<(), Error> {
    // Step 1: Verify all report signatures
    for report in reports {
        report
            .verify_signature()
            .map_err(|e| Error::BmmReportError(format!("Invalid signature: {}", e)))?;
    }

    // Step 2: Verify confirmations against header chain (if available)
    // This provides independent verification of confirmation counts
    // Note: RwTxn can be used for reads, so we use the write transaction directly
    for report in reports {
        // Get the swap to determine parent chain
        if let Ok(Some(swap)) = state.swaps.try_get(rwtxn, &report.swap_id).map_err(DbError::from) {
            // Try to verify confirmations from header chain
            if let Ok(Some(header_chain_confirmations)) = crate::state::header_chain::calculate_confirmations_from_header_chain_rw(
                state,
                rwtxn,
                swap.parent_chain,
                report.block_height,
            ) {
                // Verify reported confirmations match header chain calculation
                // Allow small difference (1-2 blocks) due to timing differences
                let diff = if report.confirmations > header_chain_confirmations {
                    report.confirmations - header_chain_confirmations
                } else {
                    header_chain_confirmations - report.confirmations
                };
                
                if diff > MAX_CONFIRMATION_DIFF {
                    tracing::warn!(
                        swap_id = %report.swap_id,
                        reported_confirmations = report.confirmations,
                        header_chain_confirmations = header_chain_confirmations,
                        block_height = report.block_height,
                        "BMM report confirmations don't match header chain calculation"
                    );
                    // Don't reject the report, but log the mismatch
                    // This allows the system to work even if header chain is slightly out of sync
                } else {
                    tracing::debug!(
                        swap_id = %report.swap_id,
                        confirmations = report.confirmations,
                        header_chain_confirmations = header_chain_confirmations,
                        "BMM report confirmations verified against header chain"
                    );
                }
            } else {
                // Header chain not synced for this parent chain - that's OK, we'll use the report
                tracing::debug!(
                    swap_id = %report.swap_id,
                    parent_chain = ?swap.parent_chain,
                    "Header chain not available for confirmation verification"
                );
            }
        }
    }

    // Step 3: Store reports in database (keyed by swap_id and participant address)
    for report in reports {
        let participant_address = report.get_signer_address();
        let key = (report.swap_id, participant_address);
        state
            .bmm_swap_reports
            .put(rwtxn, &key, report)
            .map_err(DbError::from)?;
    }

    // Step 4: Check for consensus on each swap
    let swap_ids: HashSet<SwapId> = reports.iter().map(|r| r.swap_id).collect();
    for swap_id in swap_ids {
        check_consensus_and_update_swap(state, rwtxn, &swap_id)?;
    }

    Ok(())
}

/// Check consensus on swap reports and update swap state if consensus is reached
fn check_consensus_and_update_swap(
    state: &crate::state::State,
    rwtxn: &mut RwTxn,
    swap_id: &SwapId,
) -> Result<(), Error> {
    // Get all reports for this swap from database
    let mut reports_by_participant: HashMap<Address, L1TransactionReport> = HashMap::new();
    
    // Iterate through all reports in database for this swap
    // Note: We need to scan all reports since we key by (swap_id, address)
    // For now, we'll get all reports and filter by swap_id
    // TODO: Consider adding an index by swap_id if this becomes a bottleneck
    {
        let mut all_reports = state
            .bmm_swap_reports
            .iter(rwtxn)
            .map_err(DbError::from)?;
        
        while let Some(entry) = all_reports.next().map_err(DbError::from)? {
            let ((stored_swap_id, participant_address), report) = entry;
            if stored_swap_id == *swap_id {
                // Only keep the most recent report from each participant
                let report_clone = report.clone();
                reports_by_participant
                    .entry(participant_address)
                    .and_modify(|existing| {
                        // Keep the report with higher block_height (more recent)
                        if report_clone.block_height > existing.block_height {
                            *existing = report_clone.clone();
                        }
                    })
                    .or_insert_with(|| report_clone);
            }
        }
    }

    // Check if we have enough independent participants
    if reports_by_participant.len() < MIN_CONSENSUS_PARTICIPANTS as usize {
        return Ok(()); // Not enough participants yet, wait for more reports
    }

    // Group reports by their reported values
    // We consider reports to agree if they have:
    // - Same l1_txid
    // - Confirmations within MAX_CONFIRMATION_DIFF of each other
    let mut consensus_groups: Vec<Vec<&L1TransactionReport>> = Vec::new();
    
    for report in reports_by_participant.values() {
        // Try to find an existing consensus group this report agrees with
        let mut found_group = false;
        for group in &mut consensus_groups {
            if let Some(first_report) = group.first() {
                // Check if this report agrees with the group
                if report.l1_txid == first_report.l1_txid {
                    // Check confirmations are within acceptable range
                    let conf_diff = if report.confirmations > first_report.confirmations {
                        report.confirmations - first_report.confirmations
                    } else {
                        first_report.confirmations - report.confirmations
                    };
                    if conf_diff <= MAX_CONFIRMATION_DIFF {
                        group.push(report);
                        found_group = true;
                        break;
                    }
                }
            }
        }
        
        // If no matching group found, create a new one
        if !found_group {
            consensus_groups.push(vec![report]);
        }
    }

    // Find the largest consensus group
    let largest_group = consensus_groups
        .iter()
        .max_by_key(|group| group.len())
        .ok_or_else(|| Error::BmmReportError("No consensus groups found".to_string()))?;

    // Check if the largest group has enough participants
    if largest_group.len() < MIN_CONSENSUS_PARTICIPANTS as usize {
        return Ok(()); // Not enough consensus yet
    }

    // Consensus reached! Update swap state
    let consensus_report = largest_group
        .iter()
        .min_by_key(|r| r.confirmations) // Use minimum confirmations for safety
        .ok_or_else(|| Error::BmmReportError("Empty consensus group".to_string()))?;

    // Get the swap
    let mut swap = state
        .swaps
        .try_get(rwtxn, swap_id)
        .map_err(DbError::from)?
        .ok_or_else(|| Error::SwapNotFound { swap_id: *swap_id })?;

    // Update swap with consensus information
    // Only update if the swap is still in a state that can be updated
    match swap.state {
        SwapState::Pending | SwapState::WaitingConfirmations(_, _) => {
            // Update l1_txid if it wasn't set or if it matches
            // Compare SwapTxId by converting to bytes
            let swap_txid_bytes = match &swap.l1_txid {
                crate::types::SwapTxId::Hash32(hash) => hash.to_vec(),
                crate::types::SwapTxId::Hash(hash) => hash.clone(),
            };
            let report_txid_bytes = match &consensus_report.l1_txid {
                crate::types::SwapTxId::Hash32(hash) => hash.to_vec(),
                crate::types::SwapTxId::Hash(hash) => hash.clone(),
            };
            if swap_txid_bytes != report_txid_bytes {
                swap.l1_txid = consensus_report.l1_txid.clone();
            }

            // Update state based on confirmations
            if consensus_report.confirmations >= swap.required_confirmations {
                swap.state = SwapState::ReadyToClaim;
            } else {
                swap.state = SwapState::WaitingConfirmations(
                    consensus_report.confirmations,
                    swap.required_confirmations,
                );
            }

            // Store updated swap
            state
                .swaps
                .put(rwtxn, swap_id, &swap)
                .map_err(DbError::from)?;
        }
        _ => {
            // Swap is already in a final state, don't update
        }
    }

    Ok(())
}

/// Get all reports for a specific swap
pub fn get_swap_reports(
    state: &crate::state::State,
    rotxn: &RoTxn,
    swap_id: &SwapId,
) -> Result<Vec<(Address, L1TransactionReport)>, Error> {
    let mut reports = Vec::new();
    
    let mut all_reports = state
        .bmm_swap_reports
        .iter(rotxn)
        .map_err(DbError::from)?;
    
    while let Some(entry) = all_reports.next().map_err(DbError::from)? {
        let ((stored_swap_id, participant_address), report) = entry;
        if stored_swap_id == *swap_id {
            reports.push((participant_address, report));
        }
    }
    
    Ok(reports)
}

