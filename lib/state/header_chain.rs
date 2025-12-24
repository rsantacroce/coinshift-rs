//! Header chain sync and management

use std::collections::BTreeMap;

use bitcoin::blockdata::block::Header as BlockHeader;
use sneed::{RoTxn, RwTxn, db::error::Error as DbError};

use crate::{
    bitcoin_rpc::{BitcoinRpcClient, RpcConfig},
    state::Error,
    types::{HeaderChain, ParentChainType},
};

/// Sync header chain for a parent chain
/// Downloads headers from RPC and stores them in the database
pub fn sync_header_chain(
    state: &crate::state::State,
    rwtxn: &mut RwTxn,
    parent_chain: ParentChainType,
    rpc_config: &RpcConfig,
) -> Result<(), Error> {
    let client = BitcoinRpcClient::new(rpc_config.clone());

    // Get current tip from RPC
    let current_height = client
        .get_block_height()
        .map_err(|e| Error::HeaderChainError(format!("Failed to get block height: {}", e)))?;

    // Get current header chain from database
    let mut header_chain = state
        .header_chains
        .try_get(rwtxn, &parent_chain)
        .map_err(DbError::from)?
        .unwrap_or_else(|| HeaderChain::new(parent_chain));

    // If chain is empty, start from genesis
    let start_height = if header_chain.is_empty() {
        0
    } else {
        header_chain.tip_height + 1
    };

    if start_height > current_height {
        // Already synced
        return Ok(());
    }

    tracing::info!(
        parent_chain = ?parent_chain,
        start_height = start_height,
        current_height = current_height,
        "Syncing header chain"
    );

    // Fetch headers in batches (max 2000 per request for Bitcoin Core)
    const BATCH_SIZE: u32 = 2000;
    let mut current_height_sync = start_height;

    while current_height_sync <= current_height {
        let end_height = std::cmp::min(current_height_sync + BATCH_SIZE - 1, current_height);

        // Fetch headers for this batch
        let headers = fetch_headers_batch(&client, current_height_sync, end_height)?;

        // Add headers to chain
        for (height, header) in headers {
            header_chain
                .add_header(height, header)
                .map_err(|e| Error::HeaderChainError(format!("Failed to add header: {}", e)))?;
        }

        // Update database
        state
            .header_chains
            .put(rwtxn, &parent_chain, &header_chain)
            .map_err(DbError::from)?;

        current_height_sync = end_height + 1;

        tracing::debug!(
            parent_chain = ?parent_chain,
            synced_to = end_height,
            total_height = current_height,
            "Header sync progress"
        );
    }

    tracing::info!(
        parent_chain = ?parent_chain,
        tip_height = header_chain.tip_height,
        "Header chain sync completed"
    );

    Ok(())
}

/// Fetch a batch of headers from RPC
fn fetch_headers_batch(
    client: &BitcoinRpcClient,
    start_height: u32,
    end_height: u32,
) -> Result<BTreeMap<u32, BlockHeader>, Error> {
    let mut headers = BTreeMap::new();

    // Use getblockheader to get headers
    // Note: Bitcoin Core doesn't have a direct "get headers by range" RPC
    // So we'll fetch them one by one (or use getblockheader in a loop)
    for height in start_height..=end_height {
        // Get block hash for this height
        let hash = get_block_hash_at_height(client, height)?;
        let header = get_block_header(client, &hash)?;
        headers.insert(height, header);
    }

    Ok(headers)
}

/// Get block hash at a specific height
fn get_block_hash_at_height(
    client: &BitcoinRpcClient,
    height: u32,
) -> Result<String, Error> {
    // Use getblockhash RPC
    let params = serde_json::json!([height]);
    let hash: String = client
        .call("getblockhash", params)
        .map_err(|e| Error::HeaderChainError(format!("Failed to get block hash: {}", e)))?;
    Ok(hash)
}

/// Get block header by hash
fn get_block_header(
    client: &BitcoinRpcClient,
    hash: &str,
) -> Result<BlockHeader, Error> {
    // Use getblockheader RPC with verbose=false to get raw header
    let params = serde_json::json!([hash, false]);
    let header_hex: String = client
        .call("getblockheader", params)
        .map_err(|e| Error::HeaderChainError(format!("Failed to get block header: {}", e)))?;

    // Decode hex to bytes
    let header_bytes = hex::decode(header_hex)
        .map_err(|e| Error::HeaderChainError(format!("Failed to decode header hex: {}", e)))?;

    // Parse BlockHeader (80 bytes for Bitcoin)
    if header_bytes.len() != 80 {
        return Err(Error::HeaderChainError(format!(
            "Invalid header length: expected 80 bytes, got {}",
            header_bytes.len()
        )));
    }

    use bitcoin::consensus::Decodable;
    let mut reader = std::io::Cursor::new(&header_bytes);
    let header = BlockHeader::consensus_decode(&mut reader)
        .map_err(|e| Error::HeaderChainError(format!("Failed to parse header: {}", e)))?;

    Ok(header)
}

/// Get header chain for a parent chain
pub fn get_header_chain(
    state: &crate::state::State,
    rotxn: &RoTxn,
    parent_chain: ParentChainType,
) -> Result<Option<HeaderChain>, Error> {
    let chain = state
        .header_chains
        .try_get(rotxn, &parent_chain)
        .map_err(DbError::from)?;
    Ok(chain)
}

/// Calculate confirmations using header chain (read transaction)
pub fn calculate_confirmations_from_header_chain(
    state: &crate::state::State,
    rotxn: &RoTxn,
    parent_chain: ParentChainType,
    block_height: u32,
) -> Result<Option<u32>, Error> {
    let chain = state
        .header_chains
        .try_get(rotxn, &parent_chain)
        .map_err(DbError::from)?;
    
    let Some(header_chain) = chain else {
        return Ok(None);
    };

    let confirmations = header_chain
        .calculate_confirmations(block_height)
        .map_err(|e| Error::HeaderChainError(format!("Failed to calculate confirmations: {}", e)))?;

    Ok(Some(confirmations))
}

/// Calculate confirmations using header chain (write transaction)
pub fn calculate_confirmations_from_header_chain_rw(
    state: &crate::state::State,
    rwtxn: &RwTxn,
    parent_chain: ParentChainType,
    block_height: u32,
) -> Result<Option<u32>, Error> {
    let chain = state
        .header_chains
        .try_get(rwtxn, &parent_chain)
        .map_err(DbError::from)?;
    
    let Some(header_chain) = chain else {
        return Ok(None);
    };

    let confirmations = header_chain
        .calculate_confirmations(block_height)
        .map_err(|e| Error::HeaderChainError(format!("Failed to calculate confirmations: {}", e)))?;

    Ok(Some(confirmations))
}

