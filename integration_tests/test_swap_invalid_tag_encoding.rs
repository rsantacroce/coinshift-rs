//! Test to reproduce InvalidTagEncoding(64) error from fill_swap_test
//! 
//! This test creates a swap similar to what's created in fill_swap_test and tests
//! bincode serialization/deserialization to reproduce the InvalidTagEncoding(64) error.

use coinshift::types::{Address, Swap, SwapDirection, SwapId, SwapState, SwapTxId, ParentChainType};
use bitcoin::Amount;
use bincode;
use std::error::Error;
use hex;

/// Test that reproduces the InvalidTagEncoding(64) error
/// 
/// The error occurs when trying to deserialize a swap that was serialized with bincode.
/// The swap ID from the error log is: 0edcc53e3ea55d58ac526c2a87ad2ee1d4e26dedb3f8f973995b132bd12a9f41
fn test_invalid_tag_encoding_64() -> Result<(), Box<dyn Error>> {
    println!("=== Testing InvalidTagEncoding(64) reproduction ===");
    
    // Swap ID from the error log
    let swap_id_bytes = hex::decode("0edcc53e3ea55d58ac526c2a87ad2ee1d4e26dedb3f8f973995b132bd12a9f41")?;
    let mut swap_id_array = [0u8; 32];
    swap_id_array.copy_from_slice(&swap_id_bytes);
    let swap_id = SwapId(swap_id_array);
    
    println!("Swap ID: {}", swap_id);
    
    // Create a swap similar to what's created in fill_swap_test
    // - L2 → L1 swap
    // - Signet parent chain
    // - 5,000,000 sats L2 amount
    // - 5,000,000 sats L1 amount
    // - Has l2_recipient (not an open swap)
    // - Pending state (initial state)
    
    // Create a dummy L2 recipient address (20 bytes)
    let l2_recipient = Address([1u8; 20]);
    
    // Create a swap with values similar to fill_swap_test
    let swap = Swap::new(
        swap_id,
        SwapDirection::L2ToL1,
        ParentChainType::Signet,
        SwapTxId::Hash32([0u8; 32]), // Dummy TXID
        None, // Use default confirmations (3 for Signet)
        Some(l2_recipient),
        Amount::from_sat(5_000_000), // L2 amount: 5,000,000 sats
        Some("tb1qvu95383kuvcwxv2z0lextheej4sdwz4vvxvkxg".to_string()), // L1 recipient address
        Some(Amount::from_sat(5_000_000)), // L1 amount: 5,000,000 sats
        100, // created_at_height
        Some(200), // expires_at_height
    );
    
    println!("\nSwap details:");
    println!("  ID: {}", swap.id);
    println!("  Direction: {:?}", swap.direction);
    println!("  Parent Chain: {:?}", swap.parent_chain);
    println!("  State: {:?}", swap.state);
    println!("  L2 Amount: {} sats", swap.l2_amount.to_sat());
    println!("  L1 Amount: {:?} sats", swap.l1_amount.map(|a| a.to_sat()));
    println!("  L2 Recipient: {:?}", swap.l2_recipient);
    println!("  L1 Recipient Address: {:?}", swap.l1_recipient_address);
    println!("  Required Confirmations: {}", swap.required_confirmations);
    println!("  Created At Height: {}", swap.created_at_height);
    println!("  Expires At Height: {:?}", swap.expires_at_height);
    
    // Serialize using bincode (same as heed::SerdeBincode uses)
    println!("\n=== Serialization ===");
    let serialized = match bincode::serialize(&swap) {
        Ok(bytes) => {
            println!("✓ Serialization successful: {} bytes", bytes.len());
            bytes
        }
        Err(e) => {
            println!("✗ Serialization failed: {}", e);
            return Err(e.into());
        }
    };
    
    // Print hex dump of serialized data
    println!("\nSerialized data (hex dump):");
    for (i, chunk) in serialized.chunks(16).enumerate() {
        let hex_str: Vec<String> = chunk.iter().map(|b| format!("{:02x}", b)).collect();
        let offset = i * 16;
        let ascii: String = chunk.iter()
            .map(|&b| if b.is_ascii_graphic() { b as char } else { '.' })
            .collect();
        println!("  {:04x}: {}  {}", offset, hex_str.join(" "), ascii);
    }
    
    // Analyze the byte stream to find where InvalidTagEncoding(64) might occur
    println!("\n=== Byte Analysis ===");
    println!("Looking for byte value 64 (0x40) which causes InvalidTagEncoding(64)...");
    for (i, &byte) in serialized.iter().enumerate() {
        if byte == 64 {
            println!("  Found byte 64 (0x40) at offset {} (0x{:x})", i, i);
            // Show context around this byte
            let start = i.saturating_sub(8);
            let end = (i + 9).min(serialized.len());
            let context = &serialized[start..end];
            let hex_str: Vec<String> = context.iter().map(|b| format!("{:02x}", b)).collect();
            println!("    Context: {}", hex_str.join(" "));
            println!("    Position in context: {}", i - start);
        }
    }
    
    // Try to deserialize field by field to find where it fails
    println!("\n=== Field-by-field Deserialization Test ===");
    
    // Test deserializing individual fields
    let mut offset = 0;
    
    // 1. SwapId (32 bytes)
    if serialized.len() >= offset + 32 {
        let id_bytes = &serialized[offset..offset + 32];
        match bincode::deserialize::<SwapId>(id_bytes) {
            Ok(id) => println!("  ✓ SwapId at offset {}: {:?}", offset, id),
            Err(e) => println!("  ✗ SwapId at offset {}: {}", offset, e),
        }
        offset += 32;
    }
    
    // 2. SwapDirection (enum, typically 1 byte for tag + 0 bytes for unit variant)
    if serialized.len() > offset {
        println!("  Testing SwapDirection at offset {} (byte value: 0x{:02x})", offset, serialized[offset]);
        match bincode::deserialize::<SwapDirection>(&serialized[offset..]) {
            Ok(dir) => {
                println!("  ✓ SwapDirection: {:?}", dir);
                // Direction is a unit enum, so it's just 1 byte
                offset += 1;
            }
            Err(e) => println!("  ✗ SwapDirection: {}", e),
        }
    }
    
    // 3. ParentChainType (enum, typically 1 byte for tag)
    if serialized.len() > offset {
        println!("  Testing ParentChainType at offset {} (byte value: 0x{:02x})", offset, serialized[offset]);
        match bincode::deserialize::<ParentChainType>(&serialized[offset..]) {
            Ok(pc) => {
                println!("  ✓ ParentChainType: {:?}", pc);
                offset += 1;
            }
            Err(e) => println!("  ✗ ParentChainType: {}", e),
        }
    }
    
    // 4. SwapTxId (enum with Hash32 variant: 1 byte tag + 32 bytes)
    if serialized.len() > offset {
        println!("  Testing SwapTxId at offset {} (byte value: 0x{:02x})", offset, serialized[offset]);
        if serialized.len() >= offset + 33 {
            match bincode::deserialize::<SwapTxId>(&serialized[offset..offset + 33]) {
                Ok(txid) => {
                    println!("  ✓ SwapTxId: {:?}", txid);
                    offset += 33;
                }
                Err(e) => println!("  ✗ SwapTxId: {}", e),
            }
        }
    }
    
    // 5. required_confirmations (u32, 4 bytes)
    if serialized.len() >= offset + 4 {
        let rc_bytes = &serialized[offset..offset + 4];
        let rc = u32::from_le_bytes(rc_bytes.try_into().unwrap());
        println!("  ✓ required_confirmations at offset {}: {} (bytes: {:?})", offset, rc, rc_bytes);
        offset += 4;
    }
    
    // 6. SwapState (enum)
    if serialized.len() > offset {
        println!("  Testing SwapState at offset {} (byte value: 0x{:02x})", offset, serialized[offset]);
        match bincode::deserialize::<SwapState>(&serialized[offset..]) {
            Ok(state) => {
                println!("  ✓ SwapState: {:?}", state);
                // Pending is a unit variant, so 1 byte
                offset += 1;
            }
            Err(e) => println!("  ✗ SwapState: {}", e),
        }
    }
    
    // 7. Option<Address> (1 byte for Some/None + 20 bytes if Some)
    if serialized.len() > offset {
        println!("  Testing Option<Address> at offset {} (byte value: 0x{:02x})", offset, serialized[offset]);
        if serialized[offset] == 1 && serialized.len() >= offset + 21 {
            let addr_bytes = &serialized[offset + 1..offset + 21];
            println!("    Address bytes: {:?}", addr_bytes);
            offset += 21;
        } else if serialized[offset] == 0 {
            println!("    None variant");
            offset += 1;
        }
    }
    
    // 8. l2_amount (Amount serialized as u64 via serde, 8 bytes)
    if serialized.len() >= offset + 8 {
        let amount_bytes = &serialized[offset..offset + 8];
        let amount_sats = u64::from_le_bytes(amount_bytes.try_into().unwrap());
        println!("  ✓ l2_amount at offset {}: {} sats (bytes: {:?})", offset, amount_sats, amount_bytes);
        offset += 8;
    }
    
    // 9. Option<String> for l1_recipient_address
    if serialized.len() > offset {
        println!("  Testing Option<String> at offset {} (byte value: 0x{:02x})", offset, serialized[offset]);
        // Option<String> is serialized as: 1 byte (0/1) + if Some: length (varint) + string bytes
        if serialized[offset] == 1 {
            offset += 1;
            // Try to deserialize the string
            match bincode::deserialize::<Option<String>>(&serialized[offset..]) {
                Ok(Some(s)) => {
                    println!("    ✓ String: {} (length: {})", s, s.len());
                    // String is: length (varint) + bytes
                    // For bincode legacy, length is u64 (8 bytes) + string bytes
                    offset += 8 + s.len();
                }
                Ok(None) => {
                    println!("    None");
                    offset += 1;
                }
                Err(e) => {
                    println!("    ✗ Failed to deserialize string: {}", e);
                    // Try to find where it fails
                    if serialized.len() > offset + 8 {
                        let len_bytes = &serialized[offset..offset + 8];
                        let len = u64::from_le_bytes(len_bytes.try_into().unwrap());
                        println!("      Length bytes: {:?} -> {}", len_bytes, len);
                        if serialized.len() >= offset + 8 + len as usize {
                            let str_bytes = &serialized[offset + 8..offset + 8 + len as usize];
                            println!("      String bytes: {:?}", str_bytes);
                        }
                    }
                }
            }
        } else {
            println!("    None variant");
            offset += 1;
        }
    }
    
    // 10. Option<Amount> for l1_amount
    if serialized.len() > offset {
        println!("  Testing Option<Amount> at offset {} (byte value: 0x{:02x})", offset, serialized[offset]);
        if serialized[offset] == 1 && serialized.len() >= offset + 9 {
            let amount_bytes = &serialized[offset + 1..offset + 9];
            let amount_sats = u64::from_le_bytes(amount_bytes.try_into().unwrap());
            println!("    ✓ Some(Amount): {} sats", amount_sats);
            offset += 9;
        } else if serialized[offset] == 0 {
            println!("    None variant");
            offset += 1;
        }
    }
    
    // 11. Option<String> for l1_claimer_address
    if serialized.len() > offset {
        println!("  Testing Option<String> (l1_claimer_address) at offset {} (byte value: 0x{:02x})", offset, serialized[offset]);
        if serialized[offset] == 0 {
            println!("    None variant");
            offset += 1;
        }
    }
    
    // 12. created_at_height (u32, 4 bytes)
    if serialized.len() >= offset + 4 {
        let height_bytes = &serialized[offset..offset + 4];
        let height = u32::from_le_bytes(height_bytes.try_into().unwrap());
        println!("  ✓ created_at_height at offset {}: {} (bytes: {:?})", offset, height, height_bytes);
        offset += 4;
    }
    
    // 13. Option<u32> for expires_at_height
    if serialized.len() > offset {
        println!("  Testing Option<u32> (expires_at_height) at offset {} (byte value: 0x{:02x})", offset, serialized[offset]);
        if serialized[offset] == 1 && serialized.len() >= offset + 5 {
            let height_bytes = &serialized[offset + 1..offset + 5];
            let height = u32::from_le_bytes(height_bytes.try_into().unwrap());
            println!("    ✓ Some(u32): {}", height);
            // offset would be offset + 5 here
        } else if serialized[offset] == 0 {
            println!("    None variant");
            // offset would be offset + 1 here
        }
    }
    
    println!("\n=== Full Deserialization Test ===");
    // Now try to deserialize the full swap
    match bincode::deserialize::<Swap>(&serialized) {
        Ok(deserialized) => {
            println!("✓ Full deserialization successful!");
            println!("  Deserialized swap ID: {}", deserialized.id);
            assert_eq!(swap.id, deserialized.id);
            assert_eq!(swap.direction, deserialized.direction);
            assert_eq!(swap.parent_chain, deserialized.parent_chain);
            assert_eq!(swap.l2_amount, deserialized.l2_amount);
            assert_eq!(swap.l1_amount, deserialized.l1_amount);
            println!("  All fields match!");
        }
        Err(e) => {
            println!("✗ Full deserialization failed: {}", e);
            println!("  Error details: {:?}", e);
            
            // Check if it's the InvalidTagEncoding error
            let err_str = format!("{}", e);
            if err_str.contains("InvalidTagEncoding") {
                println!("\n  ⚠ REPRODUCED InvalidTagEncoding ERROR!");
                println!("  This matches the error from the database read failure.");
            }
            
            return Err(e.into());
        }
    }
    
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt::init();
    test_invalid_tag_encoding_64()
}
