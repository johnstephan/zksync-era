#!/bin/bash

# Check if server-url is provided
if [ -z "$1" ]; then
  echo "Usage: $0 --server-url <server-url> [--circuit-ids <circuit-ids>]"
  exit 1
fi

# Parse arguments
SERVER_URL=""
CIRCUIT_IDS=""

while [[ $# -gt 0 ]]; do
  key="$1"

  case $key in
    --server-url)
      SERVER_URL="$2"
      shift # past argument
      shift # past value
      ;;
    --circuit-ids)
      CIRCUIT_IDS="$2"
      shift # past argument
      shift # past value
      ;;
    *)    # unknown option
      echo "Unknown option $1"
      exit 1
      ;;
  esac
done

# Check if server-url is set
if [ -z "$SERVER_URL" ]; then
  echo "Error: --server-url is required"
  exit 1
fi

# Loop to run the prover
while true; do
  if [ -z "$CIRCUIT_IDS" ]; then
    echo "Running prover with server-url: $SERVER_URL"
    zk f cargo run --release --bin client -- --server-url "$SERVER_URL"
  else
    echo "Running prover with server-url: $SERVER_URL and circuit-ids: $CIRCUIT_IDS"
    zk f cargo run --release --bin client -- --server-url "$SERVER_URL" --circuit-ids-rounds "$CIRCUIT_IDS"
  fi

  # Check if the command succeeded
  if [ $? -ne 0 ]; then
    echo "Prover failed. Retrying ..."
  fi

  # Sleep for a short duration before the next attempt
  sleep 5

done