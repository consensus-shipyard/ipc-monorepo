# syntax=docker/dockerfile:1

# The builder and runner are in separate Dockerfile so that we can use different caching strategies
# in the builder depending on whether we are building on CI or locally, but they are concatenated
# just before the build.

FROM debian:bookworm-slim

RUN apt-get update && \
  apt-get install -y libssl3 ca-certificates && \
  rm -rf /var/lib/apt/lists/*

ENV FM_HOME_DIR=/fendermint
ENV HOME=$FM_HOME_DIR
WORKDIR $FM_HOME_DIR

EXPOSE 26658

ENTRYPOINT ["fendermint"]
CMD ["run"]

STOPSIGNAL SIGTERM

ENV FM_ABCI__LISTEN__HOST=0.0.0.0
ENV FM_ETH__LISTEN__HOST=0.0.0.0

COPY fendermint/docker/.artifacts/bundle.car $FM_HOME_DIR/bundle.car
COPY fendermint/docker/.artifacts/contracts $FM_HOME_DIR/contracts
COPY --from=builder /app/fendermint/app/config $FM_HOME_DIR/config
COPY --from=builder /app/output/bin/fendermint /usr/local/bin/fendermint
