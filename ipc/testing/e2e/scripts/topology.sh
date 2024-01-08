#!/usr/bin/env bash

set -e

if [ $# -ne 1 ]
then
    echo "usage: ./topology.sh <topology-yaml-path>"
    exit 1
fi

TOPO_YAML=$1
TOPO_JSON=$(dirname $TOPO_YAML)/$(basename $TOPO_YAML .yaml).json
TOPO_SH=$(dirname $TOPO_JSON)/$(basename $TOPO_JSON .json).sh

echo "[*] Compiling $TOPO_YAML to $TOPO_SH"

yq -Poj $TOPO_YAML > $TOPO_JSON

echo "#!/usr/bin/env bash" > $TOPO_SH
echo "# Generated from $TOPO_YAML" >> $TOPO_SH
echo "set -e" >> $TOPO_SH

echo "# Create the agent(s)" >> $TOPO_SH
cat $TOPO_JSON | jq -r '
  .agents[]
  | "make --no-print-directory agent/up IPC_AGENT_NR=" + (.nr | tostring)
' >> $TOPO_SH

echo "# Create the root node(s)" >> $TOPO_SH
cat $TOPO_JSON | jq -r '
  . as $top
  |
    [
      $top.agents[]
      | . as $agent
      | .connections[]
      | {
          key:   .node | tostring,
          value: $agent.nr | tostring
        }
    ]
    | from_entries as $node_agent_map
  | $top.nodes[]
  | select((.parent_node == .nr) or (. | has("parent_node") | not))
  | ("make --no-print-directory node/up"
        + " IPC_NODE_NR=" + (.nr | tostring)
        + " IPC_AGENT_NR=" + ($node_agent_map[.nr | tostring])
        + " IPC_SUBNET_NAME=" + (.subnet.name | tostring)
    )
' >> $TOPO_SH

echo "# Alternate connecting agents and creating subnets and nodes to run them" >> $TOPO_SH
cat $TOPO_JSON | jq -r '
  . as $top
  |
      [
        $top.agents[]
        | . as $agent
        | .connections[]
        | {
            sort_key: ((.node | tostring) + "/a"),
            node: .node,
            agent: $agent.nr,
            cmd: ("make --no-print-directory connect"
                    + " IPC_NODE_NR="  + (.node | tostring)
                    + " IPC_AGENT_NR=" + ($agent.nr | tostring)
                 )
          }
      ] as $connections
    | $connections
    | map(. | { key: .node|tostring, value: .agent|tostring })
    | from_entries as $node_agent_map
    | [
        $top.nodes[]
        | select(has("parent_node") and (.parent_node != .nr))
        | {
            sort_key: ((.parent_node | tostring) + "/b"),
            cmd: ("make --no-print-directory node/up"
                    + " IPC_NODE_NR="         + (.nr | tostring)
                    + " IPC_AGENT_NR="        + ($node_agent_map[.nr | tostring])
                    + " IPC_PARENT_NODE_NR="  + (.parent_node | tostring)
                    + " IPC_PARENT_AGENT_NR=" + ($node_agent_map[.parent_node | tostring])
                    + " IPC_WALLET_NR="   + (.wallet | tostring)
                    + " IPC_SUBNET_NAME=" + (.subnet.name)
                    + " IPC_WALLET_FUNDS="          + (.wallet_funds // 0 | tostring)
                    + " IPC_SUBNET_FUNDS="          + (.subnet_funds // 0 | tostring)
                    + " IPC_COLLATERAL="            + (.collateral   // 0 | tostring)
                    + " IPC_MIN_VALIDATOR_STAKE="   + (.subnet.min_validator_stake   // 1  | tostring)
                    + " IPC_MIN_VALIDATORS="        + (.subnet.min_validators        // 0  | tostring)
                    + " IPC_BOTTOMUP_CHECK_PERIOD=" + (.subnet.bottomup_check_period // 10 | tostring)
                    + " IPC_TOPDOWN_CHECK_PERIOD="  + (.subnet.topdown_check_period  // 10 | tostring)
                  )
          }
      ] as $subnets
    | [
        $connections | map({sort_key: .sort_key, cmd: .cmd}),
        $subnets
      ]
    | flatten(1)
    | sort_by(.sort_key)
    | .[]
    | .cmd
' >> $TOPO_SH

rm $TOPO_JSON
