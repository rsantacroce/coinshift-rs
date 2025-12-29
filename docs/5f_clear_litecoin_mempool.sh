#!/bin/bash
#
# Clear Litecoin regtest mempool to fix bad-txns-vin-empty errors
#
# Usage:
#   ./5f_clear_litecoin_mempool.sh
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/_env.sh"
source "$SCRIPT_DIR/_litecoin_env.sh"

echo "Clearing Litecoin regtest mempool..."

# Check if node is running
if ! ltc_cli getblockchaininfo >/dev/null 2>&1; then
    echo "ERROR: Litecoin node is not running!"
    exit 1
fi

# Try to clear mempool
if ltc_cli clearmempool 2>&1; then
    echo "✓ Mempool cleared successfully"
    
    # Show mempool info
    echo ""
    echo "Mempool info:"
    ltc_cli getmempoolinfo 2>&1 || echo "  (getmempoolinfo not available)"
else
    echo "WARNING: clearmempool command failed or not available"
    echo ""
    echo "Alternative: Restart the Litecoin node to clear the mempool:"
    echo "  1. Stop: ltc_cli stop"
    echo "  2. Start: ./5_start_litecoin.sh"
fi

