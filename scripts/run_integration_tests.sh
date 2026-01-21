#!/bin/bash
#
# Run Coinshift Integration Tests
#
# This script runs the swap creation integration tests.
# It assumes all required binaries are built and available.
#
# Prerequisites:
#   - All binaries built (coinshift_app, bip300301_enforcer, bitcoind, etc.)
#   - Environment variables set (via example.env or COINSHIFT_INTEGRATION_TEST_ENV)
#
# Usage:
#   ./run_integration_tests.sh [test_name] [test_name2] ...
#
# Examples:
#   ./run_integration_tests.sh                    # Run all tests
#   ./run_integration_tests.sh swap_creation_fixed # Run specific test
#   ./run_integration_tests.sh swap_creation_fixed swap_creation_open  # Run multiple tests
#

set -e

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
INTEGRATION_TESTS_DIR="$PROJECT_ROOT/integration_tests"

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

echo_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

echo_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

echo_step() {
    echo -e "${BLUE}[STEP]${NC} $1"
}

# Check if we're in the right directory
if [ ! -d "$INTEGRATION_TESTS_DIR" ]; then
    echo_error "Integration tests directory not found: $INTEGRATION_TESTS_DIR"
    exit 1
fi

# Check if example.env exists
ENV_FILE="$INTEGRATION_TESTS_DIR/example.env"
if [ ! -f "$ENV_FILE" ]; then
    echo_warn "example.env not found at $ENV_FILE"
    echo_info "You may need to create it or set COINSHIFT_INTEGRATION_TEST_ENV"
else
    echo_info "Using environment file: $ENV_FILE"
    export COINSHIFT_INTEGRATION_TEST_ENV="$ENV_FILE"
fi

# Check if binaries are built
echo_step "Checking if binaries are built..."

check_binary() {
    local var_name=$1
    local path=$(eval echo \$$var_name)
    if [ -z "$path" ]; then
        if [ -f "$ENV_FILE" ]; then
            path=$(grep "^$var_name=" "$ENV_FILE" | cut -d'=' -f2 | tr -d "'\"")
        fi
    fi
    
    if [ -z "$path" ] || [ ! -f "$path" ]; then
        echo_error "$var_name binary not found or not set"
        echo_info "Expected path: $path"
        echo_info "Please build the binary or set $var_name environment variable"
        return 1
    fi
    echo_info "  ✓ $var_name: $path"
    return 0
}

# Source env file if it exists
if [ -f "$ENV_FILE" ]; then
    set -a
    source "$ENV_FILE"
    set +a
fi

# Check required binaries
MISSING_BINARIES=0

if ! check_binary "COINSHIFT_APP"; then
    MISSING_BINARIES=1
fi

if ! check_binary "BIP300301_ENFORCER"; then
    MISSING_BINARIES=1
fi

if ! check_binary "BITCOIND"; then
    MISSING_BINARIES=1
fi

if ! check_binary "BITCOIN_CLI"; then
    MISSING_BINARIES=1
fi

if [ $MISSING_BINARIES -eq 1 ]; then
    echo_error "Some required binaries are missing. Please build them first."
    echo_info "Build commands:"
    echo_info "  cd $PROJECT_ROOT && cargo build --bin coinshift_app"
    echo_info "  cd /path/to/bip300301_enforcer && cargo build --bin bip300301_enforcer"
    echo_info "  cd /path/to/bitcoin-patched && ./autogen.sh && ./configure && make"
    exit 1
fi

# Build integration tests if needed
echo_step "Building integration tests..."
cd "$PROJECT_ROOT"
if ! cargo build --example integration_tests --manifest-path integration_tests/Cargo.toml 2>&1 | tail -5; then
    echo_error "Failed to build integration tests"
    exit 1
fi

# Run tests
echo_step "Running integration tests..."
cd "$INTEGRATION_TESTS_DIR"

# Get test binary path
TEST_BINARY="$PROJECT_ROOT/target/debug/examples/integration_tests"

if [ ! -f "$TEST_BINARY" ]; then
    echo_error "Test binary not found: $TEST_BINARY"
    echo_info "Please build it first: cargo build --example integration_tests --manifest-path integration_tests/Cargo.toml"
    exit 1
fi

# Run tests with optional filter
if [ $# -eq 0 ]; then
    echo_info "Running all integration tests..."
    "$TEST_BINARY"
else
    echo_info "Running selected tests: $@"
    "$TEST_BINARY" --tests "$(IFS=','; echo "$*")"
fi

EXIT_CODE=$?

if [ $EXIT_CODE -eq 0 ]; then
    echo_info "All tests passed! ✓"
else
    echo_error "Some tests failed (exit code: $EXIT_CODE)"
fi

exit $EXIT_CODE

