//! Build Litecoin `SpvProof::LtcV2` proofs from a Litecoin Core JSON-RPC endpoint.
//!
//! Security note: this module is **not** trusted by consensus; it just fetches data needed
//! to construct a proof that consensus will verify deterministically.

use bitcoin::{consensus::encode::deserialize, hashes::{sha256d, Hash as _}, Txid};

use crate::{
    bitcoin_rpc::{BitcoinRpcClient, RpcConfig},
    spv::{SpvProof, ltc::LtcSpvProofV2},
};

/// Supported LTC checkpoint heights (must match `ltc_checkpoint_bytes()` in `lib/spv/ltc.rs`).
/// These are in descending order (most recent first).
const LTC_CHECKPOINTS: &[u32] = &[3_000_000u32, 2_520_000u32, 721_000u32];

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error(transparent)]
    Rpc(#[from] crate::bitcoin_rpc::Error),
    #[error("invalid hex: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("transaction is not confirmed (no blockhash)")]
    NotConfirmed,
    #[error("txid not found in block tx list")]
    TxNotInBlock,
    #[error("blockchain tip {tip} is below required height {required}")]
    TipTooLow { tip: u32, required: u32 },
    #[error("cannot choose a supported checkpoint <= tx height {tx_height} (supported checkpoints: {checkpoints:?}, tip: {tip})")]
    NoCheckpoint { 
        tx_height: u32,
        checkpoints: Vec<u32>,
        tip: u32,
    },
    #[error("invalid header length")]
    InvalidHeaderLength,
}

/// Build a borsh-encoded `SpvProof::LtcV2` blob suitable for `TxData::SwapSubmitProof`.
pub fn build_ltc_spv_proof_v2_bytes(
    rpc: &RpcConfig,
    txid_hex: &str,
    min_confirmations: u32,
) -> Result<Vec<u8>, BuildError> {
    let client = BitcoinRpcClient::new(rpc.clone());

    // Fetch tx metadata to learn blockhash (must be confirmed)
    let tx_info = client.get_transaction(txid_hex)?;
    let blockhash = tx_info.blockhash.ok_or(BuildError::NotConfirmed)?;
    let raw_tx_hex = tx_info
        .hex
        .unwrap_or(client.get_raw_transaction_hex(txid_hex)?);
    let tx_bytes = hex::decode(raw_tx_hex)?;

    // Parse txid to confirm it matches the raw tx
    let tx: bitcoin::Transaction = deserialize(&tx_bytes)
        .map_err(|_| crate::bitcoin_rpc::Error::InvalidResponse)?;
    let computed_txid = tx.compute_txid();
    let provided_txid: Txid = txid_hex.parse().map_err(|_| {
        crate::bitcoin_rpc::Error::InvalidResponse
    })?;
    if computed_txid != provided_txid {
        return Err(crate::bitcoin_rpc::Error::InvalidResponse.into());
    }

    // Determine tx block height
    let header_info = client.get_block_header_info(&blockhash)?;
    let tx_height = header_info.height;

    // Determine required tip height for min confirmations
    let required_tip = tx_height.saturating_add(min_confirmations.saturating_sub(1));
    let tip = client.get_block_count()?;
    if tip < required_tip {
        return Err(BuildError::TipTooLow { tip, required: required_tip });
    }

    // Choose checkpoint start height (must be embedded in verifier).
    let start_height = choose_checkpoint_start_height(tx_height)
        .ok_or_else(|| BuildError::NoCheckpoint { 
            tx_height,
            checkpoints: LTC_CHECKPOINTS.to_vec(),
            tip,
        })?;

    // Build header chain from checkpoint to required_tip
    let mut headers = Vec::with_capacity((required_tip - start_height + 1) as usize);
    for h in start_height..=required_tip {
        let bh = client.get_block_hash(h)?;
        let hdr_hex = client.get_block_header_hex(&bh)?;
        let hdr_bytes = hex::decode(hdr_hex)?;
        if hdr_bytes.len() != 80 {
            return Err(BuildError::InvalidHeaderLength);
        }
        let mut arr = [0u8; 80];
        arr.copy_from_slice(&hdr_bytes);
        headers.push(arr);
    }

    // Get txids in tx's block to compute merkle path + tx index
    let txids = client.get_block_txids(&blockhash)?;
    let tx_index = txids
        .iter()
        .position(|t| t == txid_hex)
        .ok_or(BuildError::TxNotInBlock)? as u32;

    let (merkle_path, _root) = build_merkle_path(&txids, tx_index)?;

    // tx_header_index within `headers`
    let tx_header_index = tx_height - start_height;

    let proof = LtcSpvProofV2 {
        tx: tx_bytes,
        merkle_path,
        tx_index,
        start_height,
        headers,
        tx_header_index,
        min_confirmations,
    };

    let spv = SpvProof::LtcV2(proof);
    Ok(borsh::to_vec(&spv).expect("borsh serialize should work"))
}

fn choose_checkpoint_start_height(tx_height: u32) -> Option<u32> {
    // Must match `ltc_checkpoint_bytes()` in `lib/spv/ltc.rs`.
    // Prefer the most recent checkpoint <= tx_height.
    for h in LTC_CHECKPOINTS {
        if *h <= tx_height {
            return Some(*h);
        }
    }
    None
}

fn build_merkle_path(
    txids: &[String],
    index: u32,
) -> Result<(Vec<[u8; 32]>, [u8; 32]), BuildError> {
    let mut layer: Vec<sha256d::Hash> = txids
        .iter()
        .map(|s| {
            let txid: Txid = s.parse().map_err(|_| crate::bitcoin_rpc::Error::InvalidResponse)?;
            Ok(sha256d::Hash::from_byte_array(txid.to_byte_array()))
        })
        .collect::<Result<_, crate::bitcoin_rpc::Error>>()?;

    let mut idx = index as usize;
    let mut path = Vec::new();
    while layer.len() > 1 {
        if layer.len() % 2 == 1 {
            let last = *layer.last().unwrap();
            layer.push(last);
        }
        let sibling_idx = if idx % 2 == 0 { idx + 1 } else { idx - 1 };
        path.push(layer[sibling_idx].to_byte_array());

        let mut next = Vec::with_capacity(layer.len() / 2);
        for pair in layer.chunks(2) {
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(pair[0].as_ref());
            buf[32..].copy_from_slice(pair[1].as_ref());
            next.push(sha256d::Hash::hash(&buf));
        }
        layer = next;
        idx /= 2;
    }
    Ok((path, layer[0].to_byte_array()))
}


