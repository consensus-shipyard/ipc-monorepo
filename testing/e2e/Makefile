# Functions to help set up the infrastructure needed to run end-to-end tests.

# Parameters that can be set when calling make to act like named arguments, or by defining env vars.
# e.g. `make <target> IPC_AGENT_NR=1`
IPC_AGENT_NR        ?= 0
IPC_WALLET_NR       ?= 0
IPC_NODE_NR         ?= 0
IPC_NODE_TYPE       ?= eudico
IPC_PARENT_NODE_NR  ?= 0
IPC_PARENT_AGENT_NR ?= 0
IPC_SUBNET_NAME     ?= ""

PROJECT_ROOT     := ../..
IPC_INFRA_DIR    := $(PROJECT_ROOT)/bin/ipc-infra
IPC_SRC          := $(shell find $(PROJECT_ROOT) -type f \( -name "*.rs" \) | grep -v target | grep -v testing)
IPC_AGENT        := $(PROJECT_ROOT)/bin/ipc-agent

IPC_NETWORK      := ipc
IPC_VERSION      := latest

# Location of artifacts derived from input parameters.
IPC_AGENT_DIR    := .ipc/agents/agent-$(IPC_AGENT_NR)
IPC_AGENT_CONFIG := $(IPC_AGENT_DIR)/config.toml
IPC_AGENT_SUBNETS:= $(shell find . -type f -wholename "./$(IPC_AGENT_DIR)/subnets/node-*" | sort)
IPC_AGENT_URL    := http://127.0.0.1:303$(IPC_AGENT_NR)/json_rpc
IPC_NODE_DIR     := .ipc/nodes/node-$(IPC_NODE_NR)

# Find created agents and nodes
IPC_AGENTS       := $(shell find . -type d -wholename "./.ipc/agents/agent-*" | awk -F '/' '{print $$4}')
IPC_NODES        := $(shell find . -type d -wholename "./.ipc/nodes/node-*" | awk -F '/' '{print $$4}')

# Don't remove artifacts created as dependencies.
.SECONDARY:

.PHONY: clean
clean: down
	docker network rm $(IPC_NETWORK) || exit 0
	rm -rf .ipc .make


# Stop and remove all nodes and agents.
down: \
		$(patsubst %, %/down, $(IPC_AGENTS)) \
		$(patsubst %, %/down, $(IPC_NODES))


# Create composition directory agent-$(IPC_AGENT_NR)
agent: $(IPC_AGENT_DIR) $(IPC_AGENT_CONFIG)


# Start an agent daemon.
agent/up: agent  $(IPC_AGENT_DIR)/steps/up


# Stop an agent daemon and remove its configuration, as if it never existed.
agent/down:
	if [ -d $(IPC_AGENT_DIR) ]; then \
		cd $(IPC_AGENT_DIR) && docker-compose down && cd - && rm -rf $(IPC_AGENT_DIR); \
	fi

# Start an agent.
agent-%/down:
	$(MAKE) --no-print-directory agent/up IPC_AGENT_NR=$*

# Stop and remove specific agent.
agent-%/down:
	$(MAKE) --no-print-directory agent/down IPC_AGENT_NR=$*


# Reload the configuration of an agent.
agent/reload: agent/up $(IPC_AGENT)
	$(IPC_AGENT) config reload --ipc-agent-url $(IPC_AGENT_URL)


# Create composition directory node-$(IPC_NODE_NR)
node: $(IPC_NODE_DIR)


# Start a node.
node/up: \
		node \
		$(IPC_NODE_DIR)/steps/pre-up \
		$(IPC_NODE_DIR)/steps/up \
		$(IPC_NODE_DIR)/steps/post-up


# Stop a node and remove its configuration, as if it never existed.
node/down:
	if [ -d $(IPC_NODE_DIR) ]; then \
		cd $(IPC_NODE_DIR) && docker-compose down -v && cd - && rm -rf $(IPC_NODE_DIR); \
	fi

# Start a node.
node-%/up:
	$(MAKE) --no-print-directory node/up IPC_NODE_NR=$*

# Stop and remove a specific node.
node-%/down:
	$(MAKE) --no-print-directory node/down IPC_NODE_NR=$*


# Connect an agent to a node. Assumes the node is running, but won't start it on its own.
# A node will always be participating in a specific subnet, so this means exactly one entry in the agent's config.
# Example: `make connect IPC_AGENT_NR=0 IPC_NODE_NR=0`
connect: agent/up
	./scripts/connect.sh $(IPC_AGENT_DIR) $(IPC_NODE_DIR)
	$(MAKE) --no-print-directory agent/reload


# Create a subnet on the active node.
subnet: $(IPC_NODE_DIR)/subnets/$(IPC_SUBNET_NAME) $(IPC_AGENT)


# Create a wallet on the active node.
wallet: $(IPC_NODE_DIR)/wallets/wallet-$(IPC_WALLET_NR) $(IPC_AGENT)


# Fund a wallet on the active node.
wallet/fund: $(IPC_NODE_DIR)/wallets/wallet-$(IPC_WALLET_NR)
	./scripts/fund-wallet.sh $(IPC_AGENT_DIR) $(IPC_NODE_DIR) $< $(IPC_AGENT) $(IPC_AGENT_URL)


# Join the subnet of the active node, on the parent node.
subnet/join: $(IPC_NODE_DIR)/steps/pre-up $(IPC_NODE_DIR)/steps/up agent/up $(IPC_AGENT)
	./scripts/join-subnet.sh $(IPC_AGENT_DIR) $(IPC_NODE_DIR) $(IPC_AGENT) $(IPC_AGENT_URL)


# Fund the subnet of the active node, on the parent node.
# This seems to require that the subnet is active, so some validators need to join first.
subnet/fund: $(IPC_NODE_DIR)/steps/pre-up $(IPC_NODE_DIR)/steps/up agent/up $(IPC_AGENT)
	./scripts/fund-subnet.sh $(IPC_AGENT_DIR) $(IPC_NODE_DIR) $(IPC_AGENT) $(IPC_AGENT_URL)


# Turn a topology YAML file into a script
topologies/%.sh: topologies/%.yaml scripts/topology.sh | jq yq
	./scripts/topology.sh $<


# Compile and execute a topology.
setup/%: topologies/%.sh
	bash $<


# Build the IPC agent binary.
$(IPC_AGENT): $(IPC_SRC)
	$(MAKE) -C $(PROJECT_ROOT) build


# Make sure the eudico docker image is built.
$(IPC_INFRA_DIR):
	$(MAKE) -C $(PROJECT_ROOT) install-infra


# Initialize the agent directory.
$(IPC_AGENT_DIR):
	mkdir -p $@
	cp template/agent/compose.yaml $@/compose.yaml

	@# The config.toml will be recreated from config.toml.orig plus the active subnets.
	cp template/agent/config.toml  $@/config.toml.orig

	@# Create .env file with overrides taken from the environment.
	export IPC_VERSION=$(IPC_VERSION); \
	export IPC_NETWORK=$(IPC_NETWORK); \
	export IPC_AGENT_NR=$(IPC_AGENT_NR); \
	./scripts/make-env.sh template/agent/.env $@/.env IPC_


# Initialize the agent config.
$(IPC_AGENT_CONFIG): $(IPC_AGENT_DIR) $(IPC_AGENT_SUBNETS)
	@echo "# DO NOT EDIT! Any changes will be overwritten as subnets are added. Add changes to config.toml.orig instead." > $@
	cat $(IPC_AGENT_DIR)/config.toml.orig >> $@
	for subnet in $(IPC_AGENT_SUBNETS); do cat $$subnet >> $@; done


# Start the IPC agent docker container.
$(IPC_AGENT_DIR)/steps/up: .make/docker/network .make/docker/images/ipc-agent
	cd $(IPC_AGENT_DIR) && docker-compose up -d
	mkdir -p $(dir $@) && touch $@


# Initialize the node directory based on the active node type.
# If the node has a parent, make a wallet and a subnet on the parent node.
# The agent used must be one connected to the parent node.
# Assumes that the root node can use some pre-funded wallet.key file in the template genesis directory.
$(IPC_NODE_DIR): | .make/$(IPC_NODE_TYPE) template/$(IPC_NODE_TYPE)/genesis/wallet.key
	mkdir -p $@
	cp template/$(IPC_NODE_TYPE)/compose.yaml $@/

	@# Create .env file with overrides taken from the environment.
	@# The IPC_SUBNET will have to be evaluated later when pre-up step has been executed.
	if [ "$(IPC_PARENT_NODE_NR)" = "$(IPC_NODE_NR)" ]; then \
		export IPC_WALLET_KEY=${PWD}/template/$(IPC_NODE_TYPE)/genesis/wallet.key && \
		export IPC_SUBNET="\"echo /r31415926\""; \
	else \
		export IPC_WALLET_KEY=${PWD}/.ipc/nodes/node-$(IPC_PARENT_NODE_NR)/wallets/wallet-$(IPC_WALLET_NR)/wallet.key && \
		export IPC_SUBNET="\"cat .ipc/nodes/node-$(IPC_PARENT_NODE_NR)/subnets/$(IPC_SUBNET_NAME)/id\"" ; \
	fi && \
	export IPC_SUBNET_NAME=$(IPC_SUBNET_NAME) ; \
	export IPC_PROJECT_ROOT=${PWD}/$(PROJECT_ROOT) && \
	export IPC_NETWORK=$(IPC_NETWORK) && \
	export IPC_PARENT_NODE_NR=$(IPC_PARENT_NODE_NR) && \
	export IPC_WALLET_NR=$(IPC_WALLET_NR) && \
	export IPC_AGENT_NR=$(IPC_AGENT_NR) && \
	export IPC_NODE_NR=$(IPC_NODE_NR) && \
	export IPC_NODE_TYPE=$(IPC_NODE_TYPE) && \
	./scripts/make-env.sh template/$(IPC_NODE_TYPE)/.env $@/.env IPC_


# Dependency check to ensure we are only asking agents to perform actions on nodes they have been connected to.
$(IPC_AGENT_DIR)/subnets/node-$(IPC_NODE_NR):
	@echo ERROR: agent-$(IPC_AGENT_NR) has not been connected to node-$(IPC_NODE_NR) yet; use `make connect` first
	@exit 1


# Tell an agent to create a wallet on a node and save it to a file.
# The agent has to be connected to the node to issue the commands.
$(IPC_NODE_DIR)/wallets/wallet-$(IPC_WALLET_NR): | $(IPC_AGENT_DIR)/subnets/node-$(IPC_NODE_NR) node/up agent/up
	./scripts/new-wallet.sh $(IPC_AGENT_DIR) $(IPC_NODE_DIR) $@ $(IPC_AGENT) $(IPC_AGENT_URL)


# Create a new subnet on a node.
# The agent has to be connected to the node to issue the commands.
$(IPC_NODE_DIR)/subnets/$(IPC_SUBNET_NAME): | $(IPC_AGENT_DIR)/subnets/node-$(IPC_NODE_NR) node/up agent/up
	./scripts/new-subnet.sh $(IPC_AGENT_DIR) $(IPC_NODE_DIR) $@ $(IPC_AGENT) $(IPC_AGENT_URL)


# Set up dependencies in the parent node, e.g. create the wallet.
$(IPC_NODE_DIR)/steps/pre-up: | $(IPC_NODE_DIR)
	bash -c ' \
	set -a && \
	source $(IPC_NODE_DIR)/.env && \
	set +a && \
	if [ "$${IPC_PARENT_NODE_NR}" != "$(IPC_NODE_NR)" ]; then \
		$(MAKE) --no-print-directory subnet wallet wallet/fund IPC_NODE_NR=$${IPC_PARENT_NODE_NR} IPC_AGENT_NR=$${IPC_PARENT_AGENT_NR} ; \
	fi && \
	echo IPC_SUBNET_ID=$$($${IPC_SUBNET}) >> $(IPC_NODE_DIR)/.env ; \
	'
	mkdir -p $(dir $@) && touch $@


# Start the node docker containers.
$(IPC_NODE_DIR)/steps/up: | $(IPC_NODE_DIR) .make/docker/network
	cd $(IPC_NODE_DIR) && docker-compose up -d
	mkdir -p $(dir $@) && touch $@


# Run steps that require the node to be running, e.g. join the subnet with the validator address.
# We do an extra docker-compose up in case the validator container crashed because it needs to join first.
$(IPC_NODE_DIR)/steps/post-up: | $(IPC_NODE_DIR)
	bash -c ' \
	set -a && \
	source $(IPC_NODE_DIR)/.env && \
	set +a && \
	if [ "$${IPC_PARENT_NODE_NR}" != "$(IPC_NODE_NR)" ]; then \
		$(MAKE) --no-print-directory subnet/join subnet/fund IPC_AGENT_NR=$${IPC_PARENT_AGENT_NR} ; \
		cd $(IPC_NODE_DIR) && docker-compose up -d ; \
	fi \
	'
	mkdir -p $(dir $@) && touch $@


# Create the docker network every container connects to.
.make/docker/network:
	docker network create $(IPC_NETWORK)
	mkdir -p $(dir $@) && touch $@


# Build the IPC agent docker image.
.make/docker/images/ipc-agent: $(IPC_SRC)
	cd $(PROJECT_ROOT) && docker build -t ipc-agent .
	mkdir -p $(dir $@) && touch $@


# Build eudico specific artifacts
.make/eudico: $(IPC_INFRA_DIR)


# Make sure `jq` is installed. Use this with the `| jq` prerequisite so it doesn't cause re-run of tasks.
.PHONY: jq
jq:
	@if [ -z "$(shell which jq)" ]; then \
		echo "Please install jq. See https://stedolan.github.io/jq/"; \
		exit 1; \
	fi


# Make sure `yq` is installed. Use this with the `| yq` prerequisite so it doesn't cause re-run of tasks.
.PHONY: yq
yq:
	@if [ -z "$(shell which yq)" ]; then \
		echo "Please install yq. See https://github.com/mikefarah/yq"; \
		exit 1; \
	fi
