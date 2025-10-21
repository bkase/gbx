#!/usr/bin/env bash
# Shared helper for running browser tests with a local server
set -euo pipefail

# Usage: run-browser-test.sh <test_dir> <port> <test_script>
TEST_DIR="${1:?Missing test directory}"
PORT="${2:?Missing port}"
TEST_SCRIPT="${3:?Missing test script path}"

echo "Starting server for ${TEST_DIR} on port ${PORT}..."
node server.js "${TEST_DIR}" "${PORT}" &
SERVER_PID=$!
trap "kill ${SERVER_PID} 2>/dev/null || true" EXIT

sleep 3

echo "Running browser test: ${TEST_SCRIPT}..."
export WASM_TEST_PORT="${PORT}"
export DEMO_TEST_PORT="${PORT}"
node "${TEST_SCRIPT}"
