#!/bin/bash

# Check if server-url is provided
if [ -z "$1" ]; then
  echo "Usage: $0 --server-url <server-url> [--username <username>]"
  exit 1
fi

# Parse arguments
SERVER_URL=""
USERNAME=""

while [[ $# -gt 0 ]]; do
  key="$1"

  case $key in
    --server-url)
      SERVER_URL="$2"
      shift # past argument
      shift # past value
      ;;
    --username)
      USERNAME="$2"#!/bin/bash

# Check if server-url is provided
if [ -z "$1" ]; then
  echo "Usage: $0 --server-url <server-url> [--username <username>]"
  exit 1
fi

# Parse arguments
SERVER_URL=""
USERNAME=""

while [[ $# -gt 0 ]]; do
  key="$1"

  case $key in
    --server-url)
      SERVER_URL="$2"
      shift # past argument
      shift # past value
      ;;
    --username)
      USERNAME="$2"
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

# Loop to run the prover 16 times with CIRCUIT_IDS from (1,0) to (15,0) and then (255,0)
for i in {1..15} 255; do
  CIRCUIT_IDS="($i,0)"
  
  for j in {1..3}; do
    if [ -z "$USERNAME" ]; then
      echo "Running prover with server-url: $SERVER_URL and circuit-ids: $CIRCUIT_IDS (Attempt $j)"
      zk f cargo run --release --bin client -- --server-url "$SERVER_URL" --circuit-ids-rounds "$CIRCUIT_IDS"
    else
      echo "Running prover with server-url: $SERVER_URL, username: $USERNAME, and circuit-ids: $CIRCUIT_IDS (Attempt $j)"
      zk f cargo run --release --bin client -- --server-url "$SERVER_URL" --username "$USERNAME" --circuit-ids-rounds "$CIRCUIT_IDS"
    fi

    # Check if the command succeeded
    if [ $? -ne 0 ]; then
      echo "Prover failed on attempt $j for circuit-ids: $CIRCUIT_IDS. Retrying ..."
      sleep 5
    fi
  done
done

echo "Prover has completed 48 runs (3 times for each of the 16 circuit-ids)."
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

# Loop to run the prover 16 times with CIRCUIT_IDS from (1,0) to (15,0) and then (255,0)
for i in {1..15} 255; do
  CIRCUIT_IDS="($i,0)"
  if [ -z "$USERNAME" ]; then
    echo "Running prover with server-url: $SERVER_URL and circuit-ids: $CIRCUIT_IDS"
    zk f cargo run --release --bin client -- --server-url "$SERVER_URL" --circuit-ids-rounds "$CIRCUIT_IDS"
  else
    echo "Running prover with server-url: $SERVER_URL, username: $USERNAME, and circuit-ids: $CIRCUIT_IDS"
    zk f cargo run --release --bin client -- --server-url "$SERVER_URL" --username "$USERNAME" --circuit-ids-rounds "$CIRCUIT_IDS"
  fi

  # Check if the command succeeded
  if [ $? -ne 0 ]; then
    echo "Prover failed. Retrying ..."
    sleep 5
  fi
done

echo "Prover has completed 16 runs."