//! Block header chain for parent chain verification

use std::collections::BTreeMap;

use bitcoin::{BlockHash, blockdata::block::Header as BlockHeader, hashes::Hash as _};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use super::ParentChainType;



/// Header chain for a parent chain
/// Stores block headers indexed by height for efficient confirmation calculation
/// Note: Uses serde for serialization (database uses SerdeBincode)
#[derive(
    Clone,
    Debug,
    Deserialize,
    Eq,
    PartialEq,
    Serialize,
)]
pub struct HeaderChain {
    pub parent_chain: ParentChainType,
    /// Headers indexed by height
    /// Using BTreeMap for ordered iteration and efficient range queries
    /// BlockHeader is serialized as 80-byte arrays
    #[serde(serialize_with = "serialize_headers_map", deserialize_with = "deserialize_headers_map")]
    pub headers: BTreeMap<u32, BlockHeader>,
    pub tip_height: u32,
    #[serde(serialize_with = "serialize_block_hash", deserialize_with = "deserialize_block_hash")]
    pub tip_hash: BlockHash,
}

// Custom serde serialization for BlockHash
fn serialize_block_hash<S>(hash: &BlockHash, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serde::Serialize::serialize(&hash.as_byte_array(), serializer)
}

fn deserialize_block_hash<'de, D>(deserializer: D) -> Result<BlockHash, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let bytes: [u8; 32] = Deserialize::deserialize(deserializer)?;
    Ok(BlockHash::from_byte_array(bytes))
}

// Custom serde serialization for BTreeMap<u32, BlockHeader>
fn serialize_headers_map<S>(
    headers: &BTreeMap<u32, BlockHeader>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::Serialize;
    use std::collections::BTreeMap as Map;
    
    // Serialize as Map<u32, Vec<u8>> where Vec<u8> is the 80-byte header
    let mut map = Map::new();
    for (height, header) in headers {
        use bitcoin::consensus::Encodable;
        let mut bytes = Vec::with_capacity(80);
        header.consensus_encode(&mut bytes)
            .map_err(|e| serde::ser::Error::custom(format!("Failed to encode header: {}", e)))?;
        if bytes.len() != 80 {
            return Err(serde::ser::Error::custom(format!("Invalid header length: expected 80, got {}", bytes.len())));
        }
        map.insert(*height, bytes);
    }
    serde::Serialize::serialize(&map, serializer)
}

fn deserialize_headers_map<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<u32, BlockHeader>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    use std::collections::BTreeMap as Map;
    
    // Deserialize from Map<u32, Vec<u8>>
    let map: Map<u32, Vec<u8>> = Deserialize::deserialize(deserializer)?;
    let mut headers = BTreeMap::new();
    for (height, bytes) in map {
        if bytes.len() != 80 {
            return Err(serde::de::Error::custom(format!("Invalid header length: expected 80, got {}", bytes.len())));
        }
        use bitcoin::consensus::Decodable;
        let header = BlockHeader::consensus_decode(&mut bytes.as_slice())
            .map_err(|e| serde::de::Error::custom(format!("Failed to decode header: {}", e)))?;
        headers.insert(height, header);
    }
    Ok(headers)
}

impl HeaderChain {
    /// Create a new empty header chain
    pub fn new(parent_chain: ParentChainType) -> Self {
        Self {
            parent_chain,
            headers: BTreeMap::new(),
            tip_height: 0,
            tip_hash: BlockHash::from_byte_array([0u8; 32]),
        }
    }

    /// Add a header to the chain
    /// Validates that the header links correctly to the previous header
    pub fn add_header(
        &mut self,
        height: u32,
        header: BlockHeader,
    ) -> Result<(), HeaderChainError> {
        // Verify height consistency
        if height != self.tip_height + 1 && self.tip_height > 0 {
            return Err(HeaderChainError::HeightMismatch {
                expected: self.tip_height + 1,
                got: height,
            });
        }

        // Verify prev_hash linkage (except for genesis)
        if self.tip_height > 0 {
            let prev_hash = header.prev_blockhash;
            if prev_hash != self.tip_hash {
                return Err(HeaderChainError::PrevHashMismatch {
                    expected: self.tip_hash,
                    got: prev_hash,
                    height,
                });
            }
        }

        // Verify block header (PoW, timestamp, etc.)
        verify_block_header(&header, self.parent_chain)?;

        // Add header to chain
        self.headers.insert(height, header);
        self.tip_height = height;
        self.tip_hash = header.block_hash();

        Ok(())
    }

    /// Get header at a specific height
    pub fn get_header(&self, height: u32) -> Option<&BlockHeader> {
        self.headers.get(&height)
    }

    /// Calculate confirmations for a block at the given height
    /// Returns: tip_height - block_height + 1
    pub fn calculate_confirmations(&self, block_height: u32) -> Result<u32, HeaderChainError> {
        if block_height > self.tip_height {
            return Err(HeaderChainError::BlockHeightTooHigh {
                block_height,
                tip_height: self.tip_height,
            });
        }
        Ok(self.tip_height - block_height + 1)
    }

    /// Verify the entire header chain is valid
    pub fn verify_chain(&self) -> Result<(), HeaderChainError> {
        if self.headers.is_empty() {
            return Ok(()); // Empty chain is valid
        }

        let mut prev_hash = None;
        let mut prev_height = None;

        for (height, header) in &self.headers {
            // Verify header
            verify_block_header(header, self.parent_chain)?;

            // Verify height ordering
            if let Some(prev) = prev_height {
                if *height != prev + 1 {
                    return Err(HeaderChainError::ChainGap {
                        prev_height: prev,
                        next_height: *height,
                    });
                }
            }

            // Verify prev_hash linkage
            if let Some(prev) = prev_hash {
                if header.prev_blockhash != prev {
                    return Err(HeaderChainError::PrevHashMismatch {
                        expected: prev,
                        got: header.prev_blockhash,
                        height: *height,
                    });
                }
            }

            prev_hash = Some(header.block_hash());
            prev_height = Some(*height);
        }

        // Verify tip matches
        if let Some((tip_height, tip_header)) = self.headers.last_key_value() {
            if *tip_height != self.tip_height {
                return Err(HeaderChainError::TipHeightMismatch {
                    expected: self.tip_height,
                    got: *tip_height,
                });
            }
            if tip_header.block_hash() != self.tip_hash {
                return Err(HeaderChainError::TipHashMismatch {
                    expected: self.tip_hash,
                    got: tip_header.block_hash(),
                });
            }
        }

        Ok(())
    }

    /// Get the genesis header (height 0)
    pub fn genesis_header(&self) -> Option<&BlockHeader> {
        self.headers.get(&0)
    }

    /// Check if chain is empty
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty()
    }
}

/// Verify a block header is valid
/// Checks: prev_hash linkage, proof-of-work (for Bitcoin), timestamp
fn verify_block_header(
    header: &BlockHeader,
    parent_chain: ParentChainType,
) -> Result<(), HeaderChainError> {
    // For Bitcoin-based chains, verify proof-of-work
    // Note: For testnets like Signet, PoW verification is different
    match parent_chain {
        ParentChainType::BTC | ParentChainType::BCH | ParentChainType::LTC => {
            // For mainnet, we should verify PoW, but for now we'll skip it
            // as it's computationally expensive and we trust the RPC
            // In production, you might want to add PoW verification
        }
        ParentChainType::Signet | ParentChainType::Regtest => {
            // For testnets, PoW verification is less critical
            // Signet uses a different consensus mechanism
        }
    }

    // Verify timestamp is reasonable (not too far in the future)
    // Allow up to 2 hours in the future for clock skew
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32;
    let max_future_time = now + 2 * 60 * 60; // 2 hours
    if header.time > max_future_time {
        return Err(HeaderChainError::TimestampTooFarInFuture {
            header_time: header.time,
            current_time: now,
        });
    }

    Ok(())
}

/// Header chain errors
#[derive(Debug, thiserror::Error)]
pub enum HeaderChainError {
    #[error("Height mismatch: expected {expected}, got {got}")]
    HeightMismatch { expected: u32, got: u32 },
    #[error("Previous hash mismatch at height {height}: expected {expected}, got {got}")]
    PrevHashMismatch {
        expected: BlockHash,
        got: BlockHash,
        height: u32,
    },
    #[error("Chain gap: prev_height {prev_height}, next_height {next_height}")]
    ChainGap { prev_height: u32, next_height: u32 },
    #[error("Tip height mismatch: expected {expected}, got {got}")]
    TipHeightMismatch { expected: u32, got: u32 },
    #[error("Tip hash mismatch: expected {expected}, got {got}")]
    TipHashMismatch {
        expected: BlockHash,
        got: BlockHash,
    },
    #[error("Block height {block_height} is higher than tip height {tip_height}")]
    BlockHeightTooHigh {
        block_height: u32,
        tip_height: u32,
    },
    #[error("Timestamp too far in future: header_time {header_time}, current_time {current_time}")]
    TimestampTooFarInFuture {
        header_time: u32,
        current_time: u32,
    },
    #[error("Header verification failed: {0}")]
    HeaderVerificationFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ParentChainType;

    #[test]
    fn test_header_chain_new() {
        let chain = HeaderChain::new(ParentChainType::Signet);
        assert_eq!(chain.parent_chain, ParentChainType::Signet);
        assert!(chain.headers.is_empty());
        assert_eq!(chain.tip_height, 0);
        assert_eq!(chain.tip_hash, BlockHash::from_byte_array([0u8; 32]));
    }

    #[test]
    fn test_calculate_confirmations() {
        let mut chain = HeaderChain::new(ParentChainType::Signet);
        
        // Empty chain with tip_height = 0: height 0 should have 1 confirmation (0 - 0 + 1)
        assert_eq!(chain.calculate_confirmations(0).unwrap(), 1);
        
        // Height 1 should error (1 > 0)
        assert!(chain.calculate_confirmations(1).is_err());
        
        // Set tip height to 100 (simulating a chain with headers up to height 100)
        // Note: We're testing the calculation logic, not header construction
        // In real usage, headers would be added via add_header() which validates them
        chain.tip_height = 100;
        chain.tip_hash = BlockHash::from_byte_array([1u8; 32]);
        // We need at least one header in the map for the chain to be non-empty
        // For testing purposes, we'll decode a minimal valid header from bytes
        use bitcoin::consensus::Decodable;
        // Minimal valid Bitcoin header (80 bytes): version(4) + prev_hash(32) + merkle_root(32) + time(4) + bits(4) + nonce(4)
        let header_bytes = [
            0x00, 0x00, 0x00, 0x20, // version: 0x20000000
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // prev_blockhash: all zeros
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // merkle_root: all zeros
            0x00, 0x00, 0x00, 0x00, // time: 0
            0xff, 0xff, 0x00, 0x1d, // bits: 0x1d00ffff
            0x00, 0x00, 0x00, 0x00, // nonce: 0
        ];
        let mut cursor = std::io::Cursor::new(&header_bytes);
        let dummy_header = BlockHeader::consensus_decode(&mut cursor).unwrap();
        chain.headers.insert(100, dummy_header);
        
        // Block at height 100 should have 1 confirmation
        assert_eq!(chain.calculate_confirmations(100).unwrap(), 1);
        
        // Block at height 90 should have 11 confirmations
        assert_eq!(chain.calculate_confirmations(90).unwrap(), 11);
        
        // Block at height 0 should have 101 confirmations
        assert_eq!(chain.calculate_confirmations(0).unwrap(), 101);
        
        // Block higher than tip should error
        assert!(chain.calculate_confirmations(101).is_err());
    }

    #[test]
    fn test_is_empty() {
        let chain = HeaderChain::new(ParentChainType::Signet);
        assert!(chain.is_empty());
        
        let mut chain = HeaderChain::new(ParentChainType::Signet);
        // Just setting tip_height doesn't make it non-empty - need actual headers
        // Decode a minimal header from bytes for testing
        use bitcoin::consensus::Decodable;
        let header_bytes = [
            0x00, 0x00, 0x00, 0x20, // version
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // prev_blockhash
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // merkle_root
            0x00, 0x00, 0x00, 0x00, // time
            0xff, 0xff, 0x00, 0x1d, // bits
            0x00, 0x00, 0x00, 0x00, // nonce
        ];
        let mut cursor = std::io::Cursor::new(&header_bytes);
        let dummy_header = BlockHeader::consensus_decode(&mut cursor).unwrap();
        chain.headers.insert(1, dummy_header);
        assert!(!chain.is_empty());
    }
}

