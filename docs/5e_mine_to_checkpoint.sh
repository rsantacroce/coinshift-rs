#!/bin/bash
#
# Mine Litecoin regtest blocks to reach the lowest checkpoint (721,000)
#
# Usage:
#   ./5e_mine_to_checkpoint.sh [target_height] [wallet_name]
#   Default: target_height = 721000 (lowest checkpoint), wallet = $LITECOIN_WALLET
#
# Example:
#   ./5e_mine_to_checkpoint.sh              # Mine to 721,000
#   ./5e_mine_to_checkpoint.sh 721000        # Explicitly mine to 721,000
#   ./5e_mine_to_checkpoint.sh 721000 mywallet
#
# MWEB Handling:
#   MWEB (MimbleWimble Extension Blocks) activates at block ~431 in Litecoin regtest.
#   After activation, mining requires MWEB transactions or you'll get "bad-txns-vin-empty"
#   errors. This script automatically:
#   1. Creates an MWEB address before block 431
#   2. Sends MWEB transactions periodically to satisfy the protocol
#   3. Handles errors by creating MWEB transactions on-demand
#
# Troubleshooting:
#   If you see "bad-txns-vin-empty" errors, the script will try to auto-recover.
#   If it persists, restart Litecoin with fresh data: ./5_start_litecoin.sh
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/_env.sh"
source "$SCRIPT_DIR/_litecoin_env.sh"

TARGET_HEIGHT="${1:-721000}"
WALLET="${2:-$LITECOIN_WALLET}"

# Check if we should use a fresh mining wallet to avoid problematic transactions
USE_FRESH_WALLET="${USE_FRESH_MINING_WALLET:-false}"
MINING_WALLET="${WALLET}_mining"

if [ "$USE_FRESH_WALLET" != "true" ]; then
    # Use the regular wallet
    echo "Ensuring wallet exists and is loaded..."
    if ! ltc_cli createwallet "$WALLET" >/dev/null 2>&1; then
        # Wallet might already exist, try to load it
        ltc_cli loadwallet "$WALLET" >/dev/null 2>&1 || true
    fi
else
    # Use a fresh mining wallet to avoid problematic transactions
    echo "Using fresh mining wallet to avoid problematic transactions..."
    echo "Creating/loading mining wallet: $MINING_WALLET"
    if ! ltc_cli createwallet "$MINING_WALLET" >/dev/null 2>&1; then
        ltc_cli loadwallet "$MINING_WALLET" >/dev/null 2>&1 || true
    fi
    WALLET="$MINING_WALLET"
    echo "✓ Using fresh mining wallet: $WALLET"
fi

# Verify wallet is loaded
if ! ltc_cli -rpcwallet="$WALLET" getwalletinfo >/dev/null 2>&1; then
    echo "ERROR: Wallet '$WALLET' is not accessible"
    echo "Trying to load wallet..."
    ltc_cli loadwallet "$WALLET" 2>&1 || {
        echo "Failed to load wallet. Available wallets:"
        ltc_cli listwallets 2>&1 || true
        exit 1
    }
fi

# Get current height
CURRENT_HEIGHT=$(ltc_cli getblockcount 2>/dev/null || echo "0")
echo "Current Litecoin regtest height: $CURRENT_HEIGHT"
echo "Target height: $TARGET_HEIGHT"

if [ "$CURRENT_HEIGHT" -ge "$TARGET_HEIGHT" ]; then
    echo "✓ Already at or above target height!"
    exit 0
fi

BLOCKS_NEEDED=$((TARGET_HEIGHT - CURRENT_HEIGHT))
echo "Blocks needed: $BLOCKS_NEEDED"
echo ""

if [ "$BLOCKS_NEEDED" -gt 10000 ]; then
    echo "⚠️  WARNING: This will mine $BLOCKS_NEEDED blocks, which may take a while."
    echo "   In regtest, blocks are mined quickly, but this is still a large number."
    echo ""
    read -p "Continue? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Cancelled."
        exit 0
    fi
fi

ADDR=$(ltc_cli -rpcwallet="$WALLET" getnewaddress)
echo "Mining address: $ADDR"
echo ""

# First, verify the node is responsive
echo "Checking node responsiveness..."
if ! ltc_cli getblockchaininfo >/dev/null 2>&1; then
    echo "ERROR: Litecoin node is not responding!"
    echo "Please check that litecoind is running."
    exit 1
fi
echo "✓ Node is responsive"

# Check wallet
echo "Checking wallet..."
if ! ltc_cli -rpcwallet="$WALLET" getwalletinfo >/dev/null 2>&1; then
    echo "ERROR: Cannot access wallet '$WALLET'"
    echo "Available wallets:"
    ltc_cli listwallets 2>&1 || true
    exit 1
fi
echo "✓ Wallet is accessible"

ensure_litecoin_bins

# ============================================================================
# MWEB HANDLING
# ============================================================================
# MWEB (MimbleWimble Extension Blocks) activates around block 431 in regtest.
# After activation, mining requires at least one MWEB transaction in the block,
# otherwise we get "bad-txns-vin-empty" errors because the HogEx transaction
# has empty inputs.
#
# Solution: Create an MWEB address and send transactions to it periodically.
# This provides the required MWEB transaction for the block template.
# ============================================================================

MWEB_ACTIVATION_HEIGHT=431
MWEB_ADDR=""
MWEB_ENABLED=false

# Function to create MWEB transaction before mining
create_mweb_tx() {
    if [ "$MWEB_ENABLED" != "true" ]; then
        return 0
    fi
    
    # Check if we have enough balance
    local balance
    balance=$(ltc_cli -rpcwallet="$WALLET" getbalance 2>/dev/null || echo "0")
    
    # Need at least 1 LTC to create MWEB transaction
    if [ "$(echo "$balance > 1" | bc 2>/dev/null || echo "0")" = "1" ]; then
        # Send a small amount to MWEB address
        if ltc_cli -rpcwallet="$WALLET" sendtoaddress "$MWEB_ADDR" 0.001 >/dev/null 2>&1; then
            return 0
        fi
    fi
    return 1
}

# Function to setup MWEB when approaching activation height
setup_mweb() {
    local current_height=$1
    
    # Only setup MWEB once, when we're approaching activation
    if [ "$MWEB_ENABLED" = "true" ]; then
        return 0
    fi
    
    if [ "$current_height" -ge $((MWEB_ACTIVATION_HEIGHT - 10)) ]; then
        echo ""
        echo "Approaching MWEB activation height ($MWEB_ACTIVATION_HEIGHT)..."
        echo "Setting up MWEB address for mining compatibility..."
        
        # Create MWEB address
        MWEB_ADDR=$(ltc_cli -rpcwallet="$WALLET" getnewaddress "" mweb 2>/dev/null || echo "")
        
        if [ -n "$MWEB_ADDR" ]; then
            echo "  ✓ Created MWEB address: $MWEB_ADDR"
            MWEB_ENABLED=true
            
            # Send initial MWEB transaction if we have balance
            local balance
            balance=$(ltc_cli -rpcwallet="$WALLET" getbalance 2>/dev/null || echo "0")
            echo "  Current balance: $balance LTC"
            
            if [ "$(echo "$balance > 1" | bc 2>/dev/null || echo "0")" = "1" ]; then
                echo "  Creating initial MWEB transaction..."
                if ltc_cli -rpcwallet="$WALLET" sendtoaddress "$MWEB_ADDR" 0.1 >/dev/null 2>&1; then
                    echo "  ✓ Created MWEB transaction"
                else
                    echo "  Warning: Could not create MWEB transaction"
                fi
            else
                echo "  Warning: Not enough balance for MWEB transaction yet"
            fi
        else
            echo "  Warning: Could not create MWEB address (mweb type not supported?)"
        fi
        echo ""
    fi
}

# Test mining with a single block
echo "Testing mining with 1 block to verify setup..."
echo "Current height before test: $(ltc_cli getblockcount 2>/dev/null || echo 'unknown')"

TEST_OUTPUT=$("$LITECOIN_CLI" -regtest \
    -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" \
    -rpcport="$LITECOIN_RPC_PORT" -datadir="$LITECOIN_DATADIR" \
    -rpcwallet="$WALLET" \
    generatetoaddress 1 "$ADDR" 2>&1)
TEST_EXIT=$?

echo "Command completed with exit code: $TEST_EXIT"

if [ $TEST_EXIT -ne 0 ]; then
    echo "ERROR: Failed to mine even 1 block!"
    echo "Error: $TEST_OUTPUT"
    echo ""
    echo "Trying to get more diagnostic info..."
    echo "Blockchain info:"
    ltc_cli getblockchaininfo 2>&1 | head -5 || echo "  Failed to get blockchain info"
    exit 1
fi

echo "✓ Test mining successful!"
echo ""

echo "Mining $BLOCKS_NEEDED blocks to address: $ADDR"
echo "This may take a while..."
echo ""
echo "Note: MWEB activates at block ~$MWEB_ACTIVATION_HEIGHT. The script will"
echo "      automatically create MWEB transactions to enable mining past this point."
echo ""

# Mine in batches to show progress
# Use smaller batches for large numbers to avoid timeouts and mempool issues
# Smaller batches also help prevent transaction accumulation
if [ "$BLOCKS_NEEDED" -gt 10000 ]; then
    BATCH_SIZE=100
elif [ "$BLOCKS_NEEDED" -gt 1000 ]; then
    BATCH_SIZE=500
else
    BATCH_SIZE=100
fi

REMAINING=$BLOCKS_NEEDED
MINED=0
LAST_REPORTED=0
LAST_MWEB_TX=0

echo "Using batch size: $BATCH_SIZE blocks per batch"
echo ""

while [ $REMAINING -gt 0 ]; do
    # Calculate batch size
    BATCH=$((REMAINING > BATCH_SIZE ? BATCH_SIZE : REMAINING))
    
    # Get current blockchain height
    set +e
    CHAIN_HEIGHT=$(ltc_cli getblockcount 2>/dev/null || echo "0")
    set -e
    
    # Setup MWEB when approaching activation height
    setup_mweb "$CHAIN_HEIGHT"
    
    # Create MWEB transaction periodically (every 100 blocks) after MWEB is enabled
    # This ensures there's always an MWEB transaction available for mining
    if [ "$MWEB_ENABLED" = "true" ] && [ $((CHAIN_HEIGHT - LAST_MWEB_TX)) -ge 100 ]; then
        if create_mweb_tx; then
            LAST_MWEB_TX=$CHAIN_HEIGHT
        fi
    fi
    
    # Show what we're about to do (more frequent updates for visibility)
    if [ $((MINED % (BATCH_SIZE * 5))) -eq 0 ] || [ $MINED -eq 0 ] || [ $MINED -lt 1000 ]; then
        echo "[$(date +%H:%M:%S)] Mining batch: $BATCH blocks (Total mined: $MINED / $BLOCKS_NEEDED, Remaining: $REMAINING, Height: $CHAIN_HEIGHT)"
    fi
    
    # Try generatetoaddress - use direct call to avoid -rpcwait hanging
    # Show that we're starting to mine (for large batches, this can take time)
    if [ "$BATCH" -ge 50 ]; then
        echo "  Mining $BATCH blocks (this may take a moment)..."
    fi
    
    set +e  # Temporarily disable exit on error to capture the actual exit code
    ERROR_OUTPUT=$("$LITECOIN_CLI" -regtest \
        -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" \
        -rpcport="$LITECOIN_RPC_PORT" -datadir="$LITECOIN_DATADIR" \
        -rpcwallet="$WALLET" \
        generatetoaddress "$BATCH" "$ADDR" 2>&1)
    GENERATE_EXIT=$?
    set -e  # Re-enable exit on error
    
    # Small sleep after successful mining to let the node process
    if [ $GENERATE_EXIT -eq 0 ]; then
        sleep 0.2
        # Show brief success for large batches
        if [ "$BATCH" -ge 50 ]; then
            echo "  ✓ Mined $BATCH blocks successfully"
        fi
    fi
    
    if [ $GENERATE_EXIT -ne 0 ]; then
        echo ""
        echo "ERROR: generatetoaddress failed for batch of $BATCH blocks"
        echo "Error message: $ERROR_OUTPUT"
        echo ""
        
        # Check if this is the MWEB "bad-txns-vin-empty" error
        if echo "$ERROR_OUTPUT" | grep -q "bad-txns-vin-empty"; then
            echo "Detected MWEB-related error. Creating MWEB transaction..."
            
            # Force setup MWEB if not already done
            if [ "$MWEB_ENABLED" != "true" ]; then
                echo "  Setting up MWEB address..."
                MWEB_ADDR=$(ltc_cli -rpcwallet="$WALLET" getnewaddress "" mweb 2>/dev/null || echo "")
                if [ -n "$MWEB_ADDR" ]; then
                    MWEB_ENABLED=true
                    echo "  ✓ Created MWEB address: $MWEB_ADDR"
                fi
            fi
            
            # Create MWEB transaction
            if [ "$MWEB_ENABLED" = "true" ]; then
                echo "  Sending MWEB transaction..."
                if ltc_cli -rpcwallet="$WALLET" sendtoaddress "$MWEB_ADDR" 0.01 >/dev/null 2>&1; then
                    echo "  ✓ Created MWEB transaction"
                    LAST_MWEB_TX=$CHAIN_HEIGHT
                    sleep 0.5
                else
                    echo "  Warning: Could not create MWEB transaction (insufficient funds?)"
                    # Check balance
                    balance=$(ltc_cli -rpcwallet="$WALLET" getbalance 2>/dev/null || echo "0")
                    echo "  Current balance: $balance LTC"
                fi
            fi
        fi
        
        # Retry mining after MWEB fix
        echo "Retrying mining..."
        set +e
        ERROR_OUTPUT_RETRY=$("$LITECOIN_CLI" -regtest \
            -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" \
            -rpcport="$LITECOIN_RPC_PORT" -datadir="$LITECOIN_DATADIR" \
            -rpcwallet="$WALLET" \
            generatetoaddress "$BATCH" "$ADDR" 2>&1)
        RETRY_EXIT=$?
        set -e
        
        if [ $RETRY_EXIT -ne 0 ]; then
            # Try with single block to isolate the issue
            echo "Retry failed. Trying with single block..."
            set +e
            SINGLE_OUTPUT=$("$LITECOIN_CLI" -regtest \
                -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" \
                -rpcport="$LITECOIN_RPC_PORT" -datadir="$LITECOIN_DATADIR" \
                -rpcwallet="$WALLET" \
                generatetoaddress 1 "$ADDR" 2>&1)
            SINGLE_EXIT=$?
            set -e
            
            if [ $SINGLE_EXIT -ne 0 ]; then
                echo "ERROR: Failed even with single block"
                echo "Error message: $SINGLE_OUTPUT"
                echo ""
                echo "This is likely an MWEB issue. Try these steps:"
                echo "  1. Check your Litecoin version supports MWEB"
                echo "  2. Ensure you have spendable balance for MWEB transactions"
                echo "  3. Restart Litecoin node with fresh data:"
                echo "     ltc_cli stop"
                echo "     ./5_start_litecoin.sh"
                exit 1
            else
                # Single block worked, reduce batch size drastically
                BATCH=1
                BATCH_SIZE=1
                echo "Success with single block. Reducing batch size to 1."
            fi
        else
            echo "✓ Success after creating MWEB transaction"
        fi
    fi
    
    # Update counters (use set +e around arithmetic to prevent exit on errors)
    set +e
    MINED=$((MINED + BATCH))
    REMAINING=$((REMAINING - BATCH))
    set -e
    
    # Safety check - ensure we're making progress
    if [ "$REMAINING" -ge "$BLOCKS_NEEDED" ]; then
        echo "ERROR: REMAINING ($REMAINING) >= BLOCKS_NEEDED ($BLOCKS_NEEDED) - loop error!"
        exit 1
    fi
    
    # Report progress more frequently for better feedback
    # Report every 1000 blocks or every 10 batches (whichever is smaller), but at least every 500 blocks
    REPORT_INTERVAL=$((BATCH_SIZE * 10))
    if [ $REPORT_INTERVAL -gt 500 ]; then
        REPORT_INTERVAL=500
    fi
    if [ $MINED -ge $((LAST_REPORTED + REPORT_INTERVAL)) ] || [ $REMAINING -eq 0 ]; then
        set +e  # Temporarily disable exit on error for status check
        CURRENT=$(ltc_cli getblockcount 2>/dev/null || echo "0")
        set -e  # Re-enable exit on error
        set +e  # Temporarily disable for arithmetic
        PERCENT=$((MINED * 100 / BLOCKS_NEEDED))
        set -e
        echo "[$(date +%H:%M:%S)] Progress: $MINED/$BLOCKS_NEEDED blocks ($PERCENT%) - Current height: $CURRENT"
        LAST_REPORTED=$MINED
        
        # Periodically create MWEB transactions to keep mining going
        if [ "$MWEB_ENABLED" = "true" ] && [ $((MINED % 1000)) -eq 0 ] && [ $MINED -gt 0 ]; then
            echo "  Creating periodic MWEB transaction..."
            create_mweb_tx || true
        fi
        
        # Add a small delay every 1000 blocks to let the node catch up
        if [ $((MINED % 1000)) -eq 0 ] && [ $MINED -gt 0 ]; then
            sleep 0.3
        fi
    fi
done

FINAL_HEIGHT=$(ltc_cli getblockcount 2>/dev/null || echo "0")
echo ""
echo "✓ Mining complete!"
echo "  Final height: $FINAL_HEIGHT"
echo "  Target was: $TARGET_HEIGHT"

if [ "$FINAL_HEIGHT" -ge "$TARGET_HEIGHT" ]; then
    echo "  ✓ Successfully reached checkpoint height!"
else
    echo "  ⚠️  Warning: Final height is below target (this shouldn't happen)"
fi

