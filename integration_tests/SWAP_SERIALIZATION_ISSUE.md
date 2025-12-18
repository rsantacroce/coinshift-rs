# Swap Serialization Issue Analysis

## Problem
Swaps are failing to deserialize after being saved to the database with error:
```
InvalidTagEncoding(64) or InvalidTagEncoding(80) or InvalidTagEncoding(251)
```

## Root Cause
The issue appears to be a mismatch between how `heed::SerdeBincode` serializes/deserializes and how the `Swap` struct is structured. The test shows that:

1. Individual enums (`SwapState`, `SwapDirection`, `ParentChainType`, `SwapTxId`) serialize/deserialize correctly
2. The full `Swap` struct fails to deserialize after serialization
3. The error occurs at different byte positions depending on the bincode configuration used

## Test Results
- Using `bincode::serialize()` (legacy, fixed-length): Fails with `InvalidTagEncoding(80)`
- Using `DefaultOptions::new()` (variable-length): Fails with `InvalidTagEncoding(251)`

## Possible Solutions
1. Ensure `Swap` struct fields are in a consistent order
2. Check if custom serde attributes (`#[serde(with = "...")]`) are causing issues
3. Verify that `Option<Address>` serializes correctly
4. Check if `heed::SerdeBincode` uses a specific bincode configuration that we need to match

## Next Steps
- Check the actual `heed-types` source code to see what bincode configuration it uses
- Test with a minimal `Swap` struct to isolate the problematic field
- Consider using explicit enum discriminants if needed
