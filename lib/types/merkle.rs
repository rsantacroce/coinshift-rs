//! Merkle proof verification for transaction inclusion

use bitcoin::{BlockHash, Txid, hashes::Hash as _};
use serde::{Deserialize, Serialize};

// Custom serde serialization for Txid
fn serialize_txid<S>(txid: &Txid, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serde::Serialize::serialize(&txid.as_byte_array(), serializer)
}

fn deserialize_txid<'de, D>(deserializer: D) -> Result<Txid, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let bytes: [u8; 32] = Deserialize::deserialize(deserializer)?;
    Ok(Txid::from_byte_array(bytes))
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
    let bytes: [u8; 32] = Deserialize::deserialize(deserializer)?;
    Ok(BlockHash::from_byte_array(bytes))
}

// Custom serialization for Vec<BlockHash>
fn serialize_block_hash_vec<S>(vec: &Vec<BlockHash>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(vec.len()))?;
    for hash in vec {
        seq.serialize_element(&hash.as_byte_array())?;
    }
    seq.end()
}

fn deserialize_block_hash_vec<'de, D>(deserializer: D) -> Result<Vec<BlockHash>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{Deserialize, Visitor};
    use std::fmt;
    
    struct BlockHashVecVisitor;
    
    impl<'de> Visitor<'de> for BlockHashVecVisitor {
        type Value = Vec<BlockHash>;
        
        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a sequence of 32-byte arrays")
        }
        
        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let mut vec = Vec::new();
            while let Some(bytes) = seq.next_element::<[u8; 32]>()? {
                vec.push(BlockHash::from_byte_array(bytes));
            }
            Ok(vec)
        }
    }
    
    deserializer.deserialize_seq(BlockHashVecVisitor)
}

/// Merkle proof structure from Bitcoin RPC
/// Contains the Merkle tree path proving transaction inclusion
/// Note: Uses serde for serialization (not stored in database, used in-memory)
#[derive(
    Clone,
    Debug,
    Deserialize,
    Eq,
    PartialEq,
    Serialize,
)]
pub struct MerkleProof {
    /// Transaction ID being proven
    #[serde(serialize_with = "serialize_txid", deserialize_with = "deserialize_txid")]
    pub txid: Txid,
    /// Block hash containing the transaction
    #[serde(serialize_with = "serialize_block_hash", deserialize_with = "deserialize_block_hash")]
    pub block_hash: BlockHash,
    /// Merkle tree path (hashes of sibling nodes)
    #[serde(serialize_with = "serialize_block_hash_vec", deserialize_with = "deserialize_block_hash_vec")]
    pub merkle_path: Vec<BlockHash>,
    /// Transaction index in the block
    pub tx_index: u32,
}

impl MerkleProof {
    /// Verify the Merkle proof against a block header's Merkle root
    pub fn verify(&self, merkle_root: &BlockHash) -> Result<bool, MerkleProofError> {
        // Start with the transaction hash
        let mut current_hash = self.txid.to_byte_array();

        // Traverse the Merkle tree path
        for (i, sibling_hash) in self.merkle_path.iter().enumerate() {
            // Determine if we're left or right child based on tx_index
            let is_left = (self.tx_index >> i) & 1 == 0;

            // Combine with sibling to compute parent hash
            let parent_hash = if is_left {
                // We're left child, sibling is right
                compute_merkle_parent(&current_hash, sibling_hash.as_byte_array())
            } else {
                // We're right child, sibling is left
                compute_merkle_parent(sibling_hash.as_byte_array(), &current_hash)
            };

            current_hash = parent_hash;
        }

        // Compare computed root with block header's Merkle root
        let computed_root = BlockHash::from_byte_array(current_hash);
        Ok(computed_root == *merkle_root)
    }
}

/// Compute parent hash in Merkle tree
/// parent = SHA256(SHA256(left || right))
fn compute_merkle_parent(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    use bitcoin::hashes::sha256d::Hash as Sha256dHash;
    use bitcoin::hashes::Hash;
    
    let mut combined = Vec::with_capacity(64);
    combined.extend_from_slice(left);
    combined.extend_from_slice(right);
    
    // Double SHA256
    Sha256dHash::hash(&combined).to_byte_array()
}

/// Merkle proof errors
#[derive(Debug, thiserror::Error)]
pub enum MerkleProofError {
    #[error("Merkle proof verification failed: {0}")]
    VerificationFailed(String),
    #[error("Invalid Merkle proof structure: {0}")]
    InvalidStructure(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::Txid;

    #[test]
    fn test_merkle_proof_structure() {
        let txid = Txid::from_byte_array([1u8; 32]);
        let block_hash = BlockHash::from_byte_array([2u8; 32]);
        let path = vec![
            BlockHash::from_byte_array([3u8; 32]),
            BlockHash::from_byte_array([4u8; 32]),
        ];
        
        let proof = MerkleProof {
            txid,
            block_hash,
            tx_index: 5,
            merkle_path: path.clone(),
        };
        
        assert_eq!(proof.txid, txid);
        assert_eq!(proof.block_hash, block_hash);
        assert_eq!(proof.tx_index, 5);
        assert_eq!(proof.merkle_path, path);
    }

    #[test]
    fn test_merkle_proof_serialization() {
        let txid = Txid::from_byte_array([1u8; 32]);
        let block_hash = BlockHash::from_byte_array([2u8; 32]);
        let proof = MerkleProof {
            txid,
            block_hash,
            tx_index: 0,
            merkle_path: vec![BlockHash::from_byte_array([3u8; 32])],
        };
        
        // Test serde serialization
        let json = serde_json::to_string(&proof).unwrap();
        let deserialized: MerkleProof = serde_json::from_str(&json).unwrap();
        assert_eq!(proof, deserialized);
    }
}

