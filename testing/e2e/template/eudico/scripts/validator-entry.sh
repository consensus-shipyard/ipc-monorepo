#!/usr/bin/env bash

set -e

# After parameters are fetched the daemon will write an API token
# to LOTUS_PATH, which we need to use to contact the server.
while [ ! -f $LOTUS_PATH/token ]; do
  echo "Waiting for the API token to appear...";
  sleep 5
done

API_TOKEN=$(cat $LOTUS_PATH/token)

# Set the env var that Lotus is looking for.
export FULLNODE_API_INFO=${API_TOKEN}:/dns/${DAEMON_HOSTNAME}/tcp/1234/http

echo "Running as subnet ${IPC_SUBNET_ID}"
exec /scripts/subnet-validator.sh
