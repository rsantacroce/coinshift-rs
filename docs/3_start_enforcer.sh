#!/bin/bash
#
# Start Enforcer Script
# 
# Usage:
#   ./3_start_enforcer.sh [--skip-proposal]
#   
# Options:
#   --skip-proposal    Skip creating sidechain proposal (useful if it already exists)
#

# Exit on error
set -e

# Parse command line arguments
SKIP_PROPOSAL=0
for arg in "$@"; do
    case $arg in
        --skip-proposal)
            SKIP_PROPOSAL=1
            shift
            ;;
        *)
            # Unknown option
            ;;
    esac
done

export BITCOIN_DIR="/home/parallels/Projects/bitcoin-patched/build/bin"
export BITCOIN_CLI="$BITCOIN_DIR/bitcoin-cli"
export ENFORCER="/home/parallels/Projects/bip300301_enforcer/target/debug/bip300301_enforcer"
export RPC_USER="user"
export RPC_PASSWORD="passwordDC"

# Mainchain regtest (for sidechain activation)
export MAINCHAIN_RPC_PORT="18443"
export MAINCHAIN_DATADIR="/home/parallels/Projects/coinshift-mainchain-data"
export MAINCHAIN_WALLET="mainchainwallet"

# Enforcer
export ENFORCER_GRPC_PORT="50051"
export ENFORCER_GRPC_ADDR="127.0.0.1:$ENFORCER_GRPC_PORT"
export ENFORCER_GRPC_URL="http://$ENFORCER_GRPC_ADDR"

# ZMQ (for mainchain)
export ZMQ_SEQUENCE="tcp://127.0.0.1:29000"

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "=========================================="
echo "Starting Enforcer"
echo "=========================================="

# Check if mainchain is running
echo "Checking if mainchain is running..."
if ! "$BITCOIN_CLI" -regtest -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$MAINCHAIN_RPC_PORT" -datadir="$MAINCHAIN_DATADIR" getblockchaininfo >/dev/null 2>&1; then
    echo "ERROR: Mainchain is not running!"
    echo "Please run ./1_start_mainchain.sh first"
    exit 1
fi
echo "Mainchain is running ✓"

# Kill existing enforcer
echo "Stopping any existing enforcer..."
pkill -f bip300301_enforcer || true
sleep 2

# Start enforcer
echo "Starting enforcer..."
"$ENFORCER" \
  --node-rpc-addr=127.0.0.1:"$MAINCHAIN_RPC_PORT" \
  --node-rpc-user="$RPC_USER" \
  --node-rpc-pass="$RPC_PASSWORD" \
  --node-zmq-addr-sequence="$ZMQ_SEQUENCE" \
  --serve-grpc-addr "$ENFORCER_GRPC_ADDR" \
  --enable-wallet \
  --wallet-sync-source=disabled &

echo "$ENFORCER" \
  --node-rpc-addr=127.0.0.1:"$MAINCHAIN_RPC_PORT" \
  --node-rpc-user="$RPC_USER" \
  --node-rpc-pass="$RPC_PASSWORD" \
  --node-zmq-addr-sequence="$ZMQ_SEQUENCE" \
  --serve-grpc-addr "$ENFORCER_GRPC_ADDR" \
  --enable-wallet \
  --wallet-sync-source=disabled &

ENFORCER_PID=$!
echo "Enforcer started with PID: $ENFORCER_PID"

# Wait for enforcer gRPC to be ready
echo "Waiting for enforcer gRPC to be ready..."
max_attempts=30
attempt=0
while [ $attempt -lt $max_attempts ]; do
    if grpcurl -plaintext "$ENFORCER_GRPC_ADDR" list >/dev/null 2>&1; then
        echo "Enforcer gRPC is ready!"
        break
    fi
    attempt=$((attempt + 1))
    sleep 1
done

if [ $attempt -eq $max_attempts ]; then
    echo "WARNING: Enforcer gRPC may not be ready yet"
    echo "You can check manually with: grpcurl -plaintext $ENFORCER_GRPC_ADDR list"
fi

# Check if sidechain proposal already exists (unless skipped)
if [ "$SKIP_PROPOSAL" -eq 1 ]; then
    echo "Skipping sidechain proposal creation (--skip-proposal flag set)"
else
    echo "Checking for existing sidechain proposals..."
    PROPOSALS_OUTPUT=$(grpcurl -plaintext "$ENFORCER_GRPC_ADDR" \
      cusf.mainchain.v1.ValidatorService/GetSidechainProposals \
      2>&1)

    PROPOSAL_EXISTS=0
    if echo "$PROPOSALS_OUTPUT" | grep -q "sidechain_proposals"; then
        # Check if there are any proposals in the response
        if echo "$PROPOSALS_OUTPUT" | grep -q "sidechain_number"; then
            PROPOSAL_EXISTS=1
            echo "Sidechain proposal(s) already exist ✓"
            echo "Proposal details:"
            echo "$PROPOSALS_OUTPUT" | head -20
        fi
    fi

    if [ "$PROPOSAL_EXISTS" -eq 0 ]; then
        echo "No existing sidechain proposals found"
        # Create sidechain proposal
        echo "Creating sidechain proposal..."
        if [ -f "$SCRIPT_DIR/create_sidechain_proposal.json" ]; then
            CREATE_OUTPUT=$(grpcurl -plaintext -d @ "$ENFORCER_GRPC_ADDR" \
              cusf.mainchain.v1.WalletService/CreateSidechainProposal \
              < "$SCRIPT_DIR/create_sidechain_proposal.json" 2>&1)
            if [ $? -eq 0 ]; then
                echo "Sidechain proposal created ✓"
            else
                echo "WARNING: Failed to create proposal: $CREATE_OUTPUT"
            fi
        else
            echo "WARNING: create_sidechain_proposal.json not found at $SCRIPT_DIR/create_sidechain_proposal.json"
        fi
    fi
fi

# Mine additional blocks for sidechain activation
echo "Mining additional blocks for sidechain activation..."
ADDR=$("$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$MAINCHAIN_RPC_PORT" \
  -datadir="$MAINCHAIN_DATADIR" \
  -rpcwallet="$MAINCHAIN_WALLET" \
  getnewaddress)

"$BITCOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$MAINCHAIN_RPC_PORT" \
  -datadir="$MAINCHAIN_DATADIR" \
  generatetoaddress 6 "$ADDR"

# Find enforcer data directory
echo ""
echo "Finding enforcer data directory..."
# Try common locations
ENFORCER_DATADIR=""
if [ -d "$HOME/.local/share/bip300301_enforcer" ]; then
    ENFORCER_DATADIR="$HOME/.local/share/bip300301_enforcer"
elif [ -d "$HOME/Library/Application Support/bip300301_enforcer" ]; then
    ENFORCER_DATADIR="$HOME/Library/Application Support/bip300301_enforcer"
elif [ -d "/var/lib/bip300301_enforcer" ]; then
    ENFORCER_DATADIR="/var/lib/bip300301_enforcer"
else
    # Try to find it from the process
    ENFORCER_CMD=$(ps -p $ENFORCER_PID -o args= 2>/dev/null || echo "")
    if [ -n "$ENFORCER_CMD" ]; then
        # Look for --datadir or similar in the command
        if echo "$ENFORCER_CMD" | grep -q "datadir"; then
            ENFORCER_DATADIR=$(echo "$ENFORCER_CMD" | grep -oP 'datadir=\K[^\s]+' || echo "")
        fi
    fi
    # If still not found, check default location
    if [ -z "$ENFORCER_DATADIR" ] && [ -d "$HOME/.bip300301_enforcer" ]; then
        ENFORCER_DATADIR="$HOME/.bip300301_enforcer"
    fi
fi

if [ -z "$ENFORCER_DATADIR" ]; then
    ENFORCER_DATADIR="(default location - check enforcer logs or process)"
fi

echo ""
echo "=========================================="
echo "Enforcer is ready!"
echo "=========================================="
echo "gRPC Address: $ENFORCER_GRPC_ADDR"
echo "Enforcer PID: $ENFORCER_PID"
echo "Enforcer Data Directory: $ENFORCER_DATADIR"
echo ""
echo "All services are now running!"
echo ""
echo "To stop all services:"
echo "  pkill -f bitcoind"
echo "  pkill -f bip300301_enforcer"
echo ""

