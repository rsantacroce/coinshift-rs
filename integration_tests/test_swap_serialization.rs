//! Test swap serialization and deserialization to identify and fix serialization issues

use coinshift::types::{Address, Swap, SwapDirection, SwapId, SwapState, SwapTxId, ParentChainType};
use bitcoin::Amount;
use bincode;
use std::error::Error;

/// Test serialization of individual enum types first
fn test_enum_serialization() -> Result<(), Box<dyn Error>> {
    println!("Testing enum serialization individually...");
    
    // Test SwapState
    println!("Testing SwapState::Pending...");
    let state = SwapState::Pending;
    let bytes = bincode::serialize(&state)?;
    println!("  SwapState::Pending serialized to {} bytes: {:?}", bytes.len(), bytes);
    let deserialized: SwapState = bincode::deserialize(&bytes)?;
    assert_eq!(state, deserialized);
    println!("  ✓ SwapState::Pending works");
    
    // Test SwapDirection
    println!("Testing SwapDirection::L2ToL1...");
    let direction = SwapDirection::L2ToL1;
    let bytes = bincode::serialize(&direction)?;
    println!("  SwapDirection::L2ToL1 serialized to {} bytes: {:?}", bytes.len(), bytes);
    let deserialized: SwapDirection = bincode::deserialize(&bytes)?;
    assert_eq!(direction, deserialized);
    println!("  ✓ SwapDirection::L2ToL1 works");
    
    // Test ParentChainType
    println!("Testing ParentChainType::Signet...");
    let parent_chain = ParentChainType::Signet;
    let bytes = bincode::serialize(&parent_chain)?;
    println!("  ParentChainType::Signet serialized to {} bytes: {:?}", bytes.len(), bytes);
    let deserialized: ParentChainType = bincode::deserialize(&bytes)?;
    assert_eq!(parent_chain, deserialized);
    println!("  ✓ ParentChainType::Signet works");
    
    // Test SwapTxId
    println!("Testing SwapTxId::Hash32...");
    let txid = SwapTxId::Hash32([0u8; 32]);
    let bytes = bincode::serialize(&txid)?;
    println!("  SwapTxId::Hash32 serialized to {} bytes: {:?}", bytes.len(), bytes);
    let deserialized: SwapTxId = bincode::deserialize(&bytes)?;
    assert_eq!(txid, deserialized);
    println!("  ✓ SwapTxId::Hash32 works");
    
    Ok(())
}

/// Test serialization and deserialization of Swap with different states
fn test_swap_serialization() -> Result<(), Box<dyn Error>> {
    // Create a test swap ID
    let swap_id = SwapId([0u8; 32]);
    
    // Test 1: Swap with Pending state
    println!("Testing Swap with Pending state...");
    let swap_pending = Swap::new(
        swap_id,
        SwapDirection::L2ToL1,
        ParentChainType::Signet,
        SwapTxId::Hash32([0u8; 32]),
        None,
        Some(Address([1u8; 20])),
        Amount::from_sat(100000),
        Some("bc1qtest".to_string()),
        Some(Amount::from_sat(50000)),
        100,
        Some(200),
    );
    test_serialize_deserialize(&swap_pending, "Pending")?;
    
    // Test 2: Swap with WaitingConfirmations state
    println!("Testing Swap with WaitingConfirmations state...");
    let mut swap_waiting = swap_pending.clone();
    swap_waiting.state = SwapState::WaitingConfirmations(1, 3);
    test_serialize_deserialize(&swap_waiting, "WaitingConfirmations")?;
    
    // Test 3: Swap with ReadyToClaim state
    println!("Testing Swap with ReadyToClaim state...");
    let mut swap_ready = swap_pending.clone();
    swap_ready.state = SwapState::ReadyToClaim;
    test_serialize_deserialize(&swap_ready, "ReadyToClaim")?;
    
    // Test 4: Swap with Completed state
    println!("Testing Swap with Completed state...");
    let mut swap_completed = swap_pending.clone();
    swap_completed.state = SwapState::Completed;
    test_serialize_deserialize(&swap_completed, "Completed")?;
    
    // Test 5: Swap with Cancelled state
    println!("Testing Swap with Cancelled state...");
    let mut swap_cancelled = swap_pending.clone();
    swap_cancelled.state = SwapState::Cancelled;
    test_serialize_deserialize(&swap_cancelled, "Cancelled")?;
    
    // Test 6: Open swap (no l2_recipient)
    println!("Testing open swap (no l2_recipient)...");
    let swap_open = Swap::new(
        swap_id,
        SwapDirection::L2ToL1,
        ParentChainType::Signet,
        SwapTxId::Hash32([0u8; 32]),
        None,
        None, // Open swap
        Amount::from_sat(100000),
        Some("bc1qtest".to_string()),
        Some(Amount::from_sat(50000)),
        100,
        Some(200),
    );
    test_serialize_deserialize(&swap_open, "Open swap")?;
    
    // Test 7: Swap with Hash variant (not Hash32)
    println!("Testing Swap with Hash variant (not Hash32)...");
    let mut swap_hash = swap_pending.clone();
    swap_hash.l1_txid = SwapTxId::Hash(vec![1, 2, 3, 4, 5]);
    test_serialize_deserialize(&swap_hash, "Hash variant")?;
    
    // Test 8: Swap with all optional fields set
    println!("Testing Swap with all optional fields set...");
    let mut swap_full = swap_pending.clone();
    swap_full.l1_claimer_address = Some("bc1qclaimer".to_string());
    test_serialize_deserialize(&swap_full, "All fields")?;
    
    // Test 9: Swap with all optional fields None
    println!("Testing Swap with all optional fields None...");
    let swap_minimal = Swap::new(
        swap_id,
        SwapDirection::L1ToL2,
        ParentChainType::Regtest,
        SwapTxId::Hash32([0u8; 32]),
        Some(6),
        None,
        Amount::from_sat(1),
        None,
        None,
        0,
        None,
    );
    test_serialize_deserialize(&swap_minimal, "Minimal fields")?;
    
    println!("All serialization tests passed!");
    Ok(())
}

fn test_serialize_deserialize(swap: &Swap, test_name: &str) -> Result<(), Box<dyn Error>> {
    println!("  {}: Testing swap: id={:?}, direction={:?}, state={:?}", 
             test_name, swap.id, swap.direction, swap.state);
    
    // Serialize using bincode legacy API (same as heed::SerdeBincode uses)
    // heed::SerdeBincode uses bincode::serialize()/deserialize() which has:
    // - Fixed-length integer encoding
    // - Little-endian
    // - No limit
    let serialized = match bincode::serialize(swap) {
        Ok(bytes) => bytes,
        Err(e) => {
            println!("  {}: Serialization failed: {}", test_name, e);
            return Err(e.into());
        }
    };
    println!("  {}: Serialized {} bytes", test_name, serialized.len());
    
    // Print all bytes for debugging to see the structure
    println!("  {}: Full serialized data ({} bytes):", test_name, serialized.len());
    for (i, chunk) in serialized.chunks(16).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{:02x}", b)).collect();
        let offset = i * 16;
        println!("    {:04x}: {}", offset, hex.join(" "));
    }
    
    // Try to manually deserialize to find where it fails
    // Let's try deserializing just the first few fields
    println!("  {}: Attempting partial deserialization to find failure point...", test_name);
    
    // Try deserializing just the id
    if let Ok(id) = bincode::deserialize::<SwapId>(&serialized[0..32]) {
        println!("  {}: ✓ SwapId deserialized: {:?}", test_name, id);
    }
    
    // Try deserializing direction (bytes 32-35)
    if serialized.len() >= 36 {
        if let Ok(dir) = bincode::deserialize::<SwapDirection>(&serialized[32..36]) {
            println!("  {}: ✓ SwapDirection deserialized: {:?}", test_name, dir);
        }
    }
    
    // Try deserializing parent_chain (bytes 36-39)
    if serialized.len() >= 40 {
        if let Ok(pc) = bincode::deserialize::<ParentChainType>(&serialized[36..40]) {
            println!("  {}: ✓ ParentChainType deserialized: {:?}", test_name, pc);
        }
    }
    
    // Try deserializing l1_txid (bytes 40-76 for Hash32)
    if serialized.len() >= 76 {
        if let Ok(txid) = bincode::deserialize::<SwapTxId>(&serialized[40..76]) {
            println!("  {}: ✓ SwapTxId deserialized: {:?}", test_name, txid);
        } else {
            println!("  {}: ✗ SwapTxId deserialization failed", test_name);
        }
    }
    
    // Try deserializing required_confirmations (bytes 76-79)
    if serialized.len() >= 80 {
        if let Ok(rc) = bincode::deserialize::<u32>(&serialized[76..80]) {
            println!("  {}: ✓ required_confirmations deserialized: {}", test_name, rc);
        }
    }
    
    // Try deserializing state (bytes 80-83)
    if serialized.len() >= 84 {
        println!("  {}: Bytes 80-83: {:?}", test_name, &serialized[80..84]);
        match bincode::deserialize::<SwapState>(&serialized[80..84]) {
            Ok(state) => println!("  {}: ✓ SwapState deserialized: {:?}", test_name, state),
            Err(e) => println!("  {}: ✗ SwapState deserialization failed: {}", test_name, e),
        }
    }
    
    // Try deserializing Option<Address> (bytes 84-105 if Some)
    if serialized.len() >= 85 {
        println!("  {}: Byte 84 (Option tag): 0x{:02x}", test_name, serialized[84]);
        if serialized.len() >= 105 {
            println!("  {}: Bytes 84-105 (Option<Address>): {:?}", test_name, &serialized[84..105]);
            // Option<Address> is serialized as: 1 byte (0 or 1) + 20 bytes if Some
            if serialized[84] == 1 {
                let addr_bytes = &serialized[85..105];
                println!("  {}: Address bytes: {:?}", test_name, addr_bytes);
            }
        }
    }
    
    // Try to deserialize a minimal struct with just the first 6 fields
    println!("  {}: Testing if issue is with later fields...", test_name);
    // Create a test struct with just: id, direction, parent_chain, l1_txid, required_confirmations, state
    // We can't easily test this without creating a new type, so let's just see where the error occurs
    
    // Try to deserialize field by field to see where it fails
    println!("  {}: Attempting deserialization...", test_name);
    let deserialized: Swap = match bincode::deserialize(&serialized) {
        Ok(swap) => swap,
        Err(e) => {
            println!("  {}: Deserialization failed: {}", test_name, e);
            println!("  {}: Error details: {:?}", test_name, e);
            return Err(e.into());
        }
    };
    
    // Verify all fields match
    assert_eq!(swap.id, deserialized.id, "{}: id mismatch", test_name);
    assert_eq!(swap.direction, deserialized.direction, "{}: direction mismatch", test_name);
    assert_eq!(swap.parent_chain, deserialized.parent_chain, "{}: parent_chain mismatch", test_name);
    assert_eq!(swap.l1_txid, deserialized.l1_txid, "{}: l1_txid mismatch", test_name);
    assert_eq!(swap.required_confirmations, deserialized.required_confirmations, "{}: required_confirmations mismatch", test_name);
    assert_eq!(swap.state, deserialized.state, "{}: state mismatch", test_name);
    assert_eq!(swap.l2_recipient, deserialized.l2_recipient, "{}: l2_recipient mismatch", test_name);
    assert_eq!(swap.l2_amount, deserialized.l2_amount, "{}: l2_amount mismatch", test_name);
    assert_eq!(swap.l1_recipient_address, deserialized.l1_recipient_address, "{}: l1_recipient_address mismatch", test_name);
    assert_eq!(swap.l1_amount, deserialized.l1_amount, "{}: l1_amount mismatch", test_name);
    assert_eq!(swap.l1_claimer_address, deserialized.l1_claimer_address, "{}: l1_claimer_address mismatch", test_name);
    assert_eq!(swap.created_at_height, deserialized.created_at_height, "{}: created_at_height mismatch", test_name);
    assert_eq!(swap.expires_at_height, deserialized.expires_at_height, "{}: expires_at_height mismatch", test_name);
    
    println!("  {}: ✓ Deserialization successful", test_name);
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt::init();
    test_enum_serialization()?;
    println!("\n");
    test_swap_serialization()
}
