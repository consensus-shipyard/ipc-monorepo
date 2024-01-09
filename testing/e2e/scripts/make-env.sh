#!/usr/bin/env bash

# Set default env values. Create overrides with `export IPC_<name>=<value>` before running `make <target>` commands,
# or override them for individual nodes e.g. `IPC_<name>=<value> make <target>`
# Captures all IPC_ env vars into the .env file so later they don't revert if we run things in another console.
# Take default values from e.g. template/agent/.env file, unless override exists in the actual env vars.

if [ $# -ne 3 ]
then
    echo "usage: ./make-env.sh <env-in> <env-out> <env-prefix>"
    exit 1
fi

ENV_IN=$1
ENV_OUT=$2
ENV_PREFIX=$3

# Read key=value pairs from the template .env file
while read kv; do
  k=$(echo ${kv} | cut -d'=' -f1)
  v=$(echo ${kv} | cut -d'=' -f2)
  # Create env var unless it already is set.
  if [ -z "${!k}" ]; then
    export ${k}=${v}
  fi
done < ${ENV_IN}

# Clean out any pre-existing content.
rm -f ${ENV_OUT}

# Write key=value pairs for all env vars starting with the prefix.
for k in $(compgen -v ${ENV_PREFIX}); do
  echo ${k}=${!k} >> ${ENV_OUT}
done
