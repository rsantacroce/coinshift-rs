#!/bin/bash
#
# Send Litecoin to a specific address
#
# Usage:
#   ./5d_send_litecoin.sh <address> [amount] [wallet_name]
#   
# Arguments:
#   address    Litecoin address to send to (required)
#   amount     Amount in LTC to send (default: 0.01)
#   wallet_name Wallet name to send from (default: $LITECOIN_WALLET)
#
# Example:
#   ./5d_send_litecoin.sh ltc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh 0.1
#   ./5d_send_litecoin.sh ltc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh 0.1 mywallet
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/_env.sh"
source "$SCRIPT_DIR/_litecoin_env.sh"

# Check if address is provided
if [ -z "$1" ]; then
    echo "ERROR: Address is required"
    echo ""
    echo "Usage: $0 <address> [amount] [wallet_name]"
    echo "  address    Litecoin address to send to (required)"
    echo "  amount     Amount in LTC to send (default: 0.01)"
    echo "  wallet_name Wallet name to send from (default: $LITECOIN_WALLET)"
    echo ""
    echo "Example:"
    echo "  $0 ltc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh 0.1"
    exit 1
fi

TARGET_ADDRESS="$1"
AMOUNT="${2:-0.01}"
WALLET="${3:-$LITECOIN_WALLET}"

# Ensure wallet exists (ignore if it already does)
ltc_cli createwallet "$WALLET" >/dev/null 2>&1 || true

# Check if Litecoin node is running
echo "Checking if Litecoin node is running..."
if ! ltc_cli getblockchaininfo >/dev/null 2>&1; then
    echo "ERROR: Litecoin node is not running!"
    echo "Please run ./5_start_litecoin.sh first"
    exit 1
fi
echo "Litecoin node is running ✓"

# Check wallet balance
echo "Checking wallet balance..."
BALANCE=$(ltc_cli -rpcwallet="$WALLET" getbalance 2>/dev/null || echo "0")
echo "Wallet balance: $BALANCE LTC"

# Check if we have enough funds
if (( $(echo "$BALANCE < $AMOUNT" | bc -l) )); then
    echo "ERROR: Insufficient funds in wallet"
    echo "  Required: $AMOUNT LTC"
    echo "  Available: $BALANCE LTC"
    echo ""
    echo "Mine some blocks first: ./5c_mine_litecoin.sh 101"
    exit 1
fi

# Send funds
echo ""
echo "Sending $AMOUNT LTC to address: $TARGET_ADDRESS"
TXID=$(ltc_cli -rpcwallet="$WALLET" sendtoaddress "$TARGET_ADDRESS" "$AMOUNT" 2>&1)

if [ $? -eq 0 ]; then
    echo "✓ Transaction sent successfully!"
    echo "  Transaction ID: $TXID"
    echo ""
    echo "Waiting for confirmation..."
    ADDR=$(ltc_cli -rpcwallet="$WALLET" getnewaddress)
    if ! ltc_cli -rpcwallet="$WALLET" generatetoaddress 1 "$ADDR" >/dev/null 2>&1; then
        echo "ERROR: Failed to generate confirmation block"
        echo "Error details:"
        ltc_cli -rpcwallet="$WALLET" generatetoaddress 1 "$ADDR" 2>&1 || true
        echo "Transaction was sent but confirmation block generation failed."
    fi
    
    echo "✓ Transaction confirmed!"
    echo ""
    echo "New wallet balance:"
    ltc_cli -rpcwallet="$WALLET" getbalance
else
    echo "✗ Failed to send transaction:"
    echo "$TXID"
    exit 1
fi

