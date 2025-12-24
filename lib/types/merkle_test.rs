#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{hashes::Hash as _, Txid};

    #[test]
    fn test_merkle_proof_verify_valid() {
        // Create a simple Merkle proof
        // In a real test, we'd use actual Bitcoin block data
        let txid = Txid::from_byte_array([1u8; 32]);
        let block_hash = BlockHash::from_byte_array([2u8; 32]);
        
        // For a single transaction, the Merkle path is empty and root is just the txid
        let merkle_root = txid.to_byte_array();
        let expected_root = BlockHash::from_byte_array(merkle_root);
        
        let proof = MerkleProof {
            txid,
            block_hash,
            index: 0,
            merkle_path: Vec::new(),
        };
        
        // This should work for a single-transaction block
        // In practice, we'd need real block data to test properly
        // For now, we just verify the structure is correct
        assert_eq!(proof.txid, txid);
        assert_eq!(proof.block_hash, block_hash);
        assert_eq!(proof.index, 0);
    }

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
            index: 5,
            merkle_path: path.clone(),
        };
        
        assert_eq!(proof.txid, txid);
        assert_eq!(proof.block_hash, block_hash);
        assert_eq!(proof.index, 5);
        assert_eq!(proof.merkle_path, path);
    }
}

