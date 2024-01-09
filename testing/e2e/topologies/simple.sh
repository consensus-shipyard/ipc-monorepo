#!/usr/bin/env bash
# Generated from topologies/simple.yaml
set -e
# Create the agent(s)
make --no-print-directory agent/up IPC_AGENT_NR=0
# Create the root node(s)
make --no-print-directory node/up IPC_NODE_NR=0 IPC_AGENT_NR=0 IPC_SUBNET_NAME=head
# Alternate connecting agents and creating subnets and nodes to run them
make --no-print-directory connect IPC_NODE_NR=0 IPC_AGENT_NR=0
make --no-print-directory node/up IPC_NODE_NR=1 IPC_AGENT_NR=0 IPC_PARENT_NODE_NR=0 IPC_PARENT_AGENT_NR=0 IPC_WALLET_NR=0 IPC_SUBNET_NAME=thorax IPC_WALLET_FUNDS=10 IPC_SUBNET_FUNDS=5 IPC_COLLATERAL=1 IPC_MIN_VALIDATOR_STAKE=1 IPC_MIN_VALIDATORS=0 IPC_BOTTOMUP_CHECK_PERIOD=10 IPC_TOPDOWN_CHECK_PERIOD=10
make --no-print-directory connect IPC_NODE_NR=1 IPC_AGENT_NR=0
make --no-print-directory node/up IPC_NODE_NR=2 IPC_AGENT_NR=0 IPC_PARENT_NODE_NR=1 IPC_PARENT_AGENT_NR=0 IPC_WALLET_NR=0 IPC_SUBNET_NAME=abdomen IPC_WALLET_FUNDS=4 IPC_SUBNET_FUNDS=2 IPC_COLLATERAL=1 IPC_MIN_VALIDATOR_STAKE=1 IPC_MIN_VALIDATORS=0 IPC_BOTTOMUP_CHECK_PERIOD=10 IPC_TOPDOWN_CHECK_PERIOD=10
make --no-print-directory connect IPC_NODE_NR=2 IPC_AGENT_NR=0
