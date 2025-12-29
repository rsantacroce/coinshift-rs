#!/bin/bash
#
# Litecoin Regtest Setup Script (Parent chain for swap testing)
#
# Ports used (defaults, override via env):
#   RPC: 19443
#   P2P: 39333
#
# Data directory (default, override via env):
#   $PROJECT_ROOT/coinshift-litecoin-data
#

# Exit on error
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/_env.sh"
source "$SCRIPT_DIR/_litecoin_env.sh"

check_port() {
  local port=$1
  local name=$2
  if command -v lsof >/dev/null 2>&1; then
    if lsof -Pi :$port -sTCP:LISTEN -t >/dev/null 2>&1; then
      echo "ERROR: $name port $port is still in use!"
      echo "Please manually kill the process using: lsof -ti :$port | xargs kill -9"
      return 1
    fi
  elif command -v ss >/dev/null 2>&1; then
    if ss -lptn "sport = :$port" 2>/dev/null | grep -q LISTEN; then
      echo "ERROR: $name port $port is still in use!"
      echo "Please manually kill the process using: ss -lptn 'sport = :$port'"
      return 1
    fi
  fi
  return 0
}

echo "=========================================="
echo "Starting Litecoin Regtest (parent chain)"
echo "=========================================="

ensure_litecoin_bins

# Clean up data directory
echo "Cleaning up data directory..."
rm -rf "$LITECOIN_DATADIR"
mkdir -p "$LITECOIN_DATADIR"

# Create litecoin.conf to disable MWEB
# This prevents "bad-txns-vin-empty" errors during mining by pushing MWEB
# activation to a very high block height that we'll never reach.
cat > "$LITECOIN_DATADIR/litecoin.conf" << 'EOF'
# Disable MWEB by setting activation parameters to never activate
# MWEB causes "bad-txns-vin-empty" errors when mining regtest blocks after ~block 431
# because it creates HogEx transactions with empty inputs
[regtest]
# Push MWEB activation to far future using version bits params
# vbparams format: name:start:end - setting start very high means never activates
vbparams=mweb:999999999:999999999
EOF
echo "Created litecoin.conf to disable MWEB"
sleep 1

if ! check_port "$LITECOIN_P2P_PORT" "Litecoin P2P"; then
  exit 1
fi
if ! check_port "$LITECOIN_RPC_PORT" "Litecoin RPC"; then
  exit 1
fi

echo "Starting litecoind..."
echo "  RPC Port: $LITECOIN_RPC_PORT"
echo "  P2P Port: $LITECOIN_P2P_PORT"
echo "  Data Dir: $LITECOIN_DATADIR"

# Note: MWEB is disabled via litecoin.conf (vbparams=mweb:999999999:999999999)
# to avoid "bad-txns-vin-empty" errors during mining.
if ! "$LITECOIND" -regtest \
  -fallbackfee=0.0002 \
  -rpcuser="$RPC_USER" \
  -rpcpassword="$RPC_PASSWORD" \
  -rpcport="$LITECOIN_RPC_PORT" \
  -server -txindex \
  -bind=0.0.0.0:"$LITECOIN_P2P_PORT" \
  -port="$LITECOIN_P2P_PORT" \
  -datadir="$LITECOIN_DATADIR" \
  -daemon; then
  echo "ERROR: Failed to start litecoind"
  echo "Check the logs in $LITECOIN_DATADIR/debug.log"
  exit 1
fi

sleep 2

echo "Verifying litecoind is using the correct ports..."
if command -v lsof >/dev/null 2>&1; then
  echo "  Checking port $LITECOIN_P2P_PORT (P2P):"
  lsof -Pi :$LITECOIN_P2P_PORT -sTCP:LISTEN 2>/dev/null | head -3 || echo "    Port $LITECOIN_P2P_PORT not in use (unexpected)"
  echo "  Checking port $LITECOIN_RPC_PORT (RPC):"
  lsof -Pi :$LITECOIN_RPC_PORT -sTCP:LISTEN 2>/dev/null | head -3 || echo "    Port $LITECOIN_RPC_PORT not in use (unexpected)"
fi

echo "Litecoin node started, waiting for it to initialize..."
sleep 2

echo "Waiting for Litecoin RPC to be ready..."
max_attempts=60
attempt=0
while [ $attempt -lt $max_attempts ]; do
  if "$LITECOIN_CLI" -regtest -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$LITECOIN_RPC_PORT" -datadir="$LITECOIN_DATADIR" getblockchaininfo >/dev/null 2>&1; then
    echo "Litecoin RPC is ready!"
    break
  fi
  attempt=$((attempt + 1))
  if [ $((attempt % 5)) -eq 0 ]; then
    echo "  Still waiting... (attempt $attempt/$max_attempts)"
    if ! pgrep -f "litecoind.*$LITECOIN_RPC_PORT" >/dev/null 2>&1; then
      echo "ERROR: litecoind process died!"
      echo "Check the logs in $LITECOIN_DATADIR/debug.log"
      exit 1
    fi
  fi
  sleep 1
done

if [ $attempt -eq $max_attempts ]; then
  echo "ERROR: Litecoin RPC failed to start after $max_attempts attempts"
  echo "Check the logs in $LITECOIN_DATADIR/debug.log"
  echo "Trying to get error info..."
  "$LITECOIN_CLI" -regtest -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$LITECOIN_RPC_PORT" -datadir="$LITECOIN_DATADIR" getblockchaininfo 2>&1 || true
  exit 1
fi

echo "Creating Litecoin wallet..."
"$LITECOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$LITECOIN_RPC_PORT" \
  -datadir="$LITECOIN_DATADIR" \
  createwallet "$LITECOIN_WALLET" >/dev/null 2>&1 || true

ADDR=$("$LITECOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$LITECOIN_RPC_PORT" \
  -datadir="$LITECOIN_DATADIR" \
  -rpcwallet="$LITECOIN_WALLET" \
  getnewaddress)

echo "Mining 101 blocks on Litecoin regtest..."
if ! "$LITECOIN_CLI" -regtest -rpcwait \
  -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$LITECOIN_RPC_PORT" \
  -datadir="$LITECOIN_DATADIR" \
  generatetoaddress 101 "$ADDR" >/dev/null 2>&1; then
  echo "WARNING: generatetoaddress failed, trying 'generate'..."
  "$LITECOIN_CLI" -regtest -rpcwait \
    -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASSWORD" -rpcport="$LITECOIN_RPC_PORT" \
    -datadir="$LITECOIN_DATADIR" \
    generate 101 >/dev/null
fi

echo ""
echo "=========================================="
echo "Litecoin regtest is ready!"
echo "=========================================="
echo "RPC Port: $LITECOIN_RPC_PORT"
echo "P2P Port: $LITECOIN_P2P_PORT"
echo "Data Dir: $LITECOIN_DATADIR"
echo "Wallet:   $LITECOIN_WALLET"
echo ""
echo "Next helpers:"
echo "  ./5a_litecoin_generate_address.sh"
echo "  ./5b_litecoin_check_balance.sh"
echo "  ./5c_mine_litecoin.sh 10"
echo ""


