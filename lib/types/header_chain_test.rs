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
        
        // Empty chain should error
        assert!(chain.calculate_confirmations(0).is_err());
        
        // Set tip height to 100
        chain.tip_height = 100;
        chain.tip_hash = BlockHash::from_byte_array([1u8; 32]);
        
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
        chain.tip_height = 1;
        assert!(!chain.is_empty());
    }
}

