// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT

use anyhow::{anyhow, bail, Context};
use fendermint_abci::ApplicationService;
use fendermint_app::{App, AppConfig, AppParentFinalityQuery, AppStore, BitswapBlockstore};
use fendermint_app_settings::AccountKind;
use fendermint_crypto::SecretKey;
use fendermint_rocksdb::{blockstore::NamespaceBlockstore, namespaces, RocksDb, RocksDbConfig};
use fendermint_vm_actor_interface::eam::EthAddress;
use fendermint_vm_interpreter::{
    bytes::{BytesMessageInterpreter, ProposalPrepareMode},
    chain::{ChainMessageInterpreter, CheckpointPool},
    fvm::{Broadcaster, FvmMessageInterpreter, ValidatorContext},
    signed::SignedMessageInterpreter,
};
use fendermint_vm_resolver::ipld::IpldResolver;
use fendermint_vm_snapshot::{SnapshotManager, SnapshotParams};
use fendermint_vm_topdown::proxy::IPCProviderProxy;
use fendermint_vm_topdown::sync::launch_polling_syncer;
use fendermint_vm_topdown::{CachedFinalityProvider, Toggle};
use fvm_shared::address::Address;
use ipc_provider::config::subnet::{EVMSubnet, SubnetConfig};
use ipc_provider::IpcProvider;
use libp2p::identity::secp256k1;
use libp2p::identity::Keypair;
use std::sync::Arc;
use tracing::info;

use crate::cmd::key::read_secret_key;
use crate::{cmd, options::run::RunArgs, settings::Settings};

fn create_ipc_provider_proxy(settings: &Settings) -> anyhow::Result<IPCProviderProxy> {
    let topdown_config = settings.ipc.topdown_config()?;
    let subnet = ipc_provider::config::Subnet {
        id: settings
            .ipc
            .subnet_id
            .parent()
            .ok_or_else(|| anyhow!("subnet has no parent"))?,
        config: SubnetConfig::Fevm(EVMSubnet {
            provider_http: topdown_config
                .parent_http_endpoint
                .to_string()
                .parse()
                .unwrap(),
            auth_token: None,
            registry_addr: topdown_config.parent_registry,
            gateway_addr: topdown_config.parent_gateway,
        }),
    };
    info!("init ipc provider with subnet: {}", subnet.id);

    let ipc_provider = IpcProvider::new_with_subnet(None, subnet)?;
    IPCProviderProxy::new(ipc_provider, settings.ipc.subnet_id.clone())
}

cmd! {
  RunArgs(self, settings) {
    run(settings).await
  }
}

/// Run the Fendermint ABCI Application.
///
/// This method acts as our composition root.
async fn run(settings: Settings) -> anyhow::Result<()> {
    let tendermint_rpc_url = settings.tendermint_rpc_url()?;
    tracing::info!("Connecting to Tendermint at {tendermint_rpc_url}");

    let tendermint_client: tendermint_rpc::HttpClient =
        tendermint_rpc::HttpClient::new(tendermint_rpc_url)
            .context("failed to create Tendermint client")?;

    let validator = match settings.validator_key {
        Some(ref key) => {
            let sk = key.path(settings.home_dir());
            if sk.exists() && sk.is_file() {
                let sk = read_secret_key(&sk).context("failed to read validator key")?;
                let addr = to_address(&sk, &key.kind)?;
                tracing::info!("validator key address: {addr} detected");
                Some((sk, addr))
            } else {
                bail!("validator key does not exist: {}", sk.to_string_lossy());
            }
        }
        None => {
            tracing::debug!("validator key not configured");
            None
        }
    };

    let validator_ctx = validator.map(|(sk, addr)| {
        // For now we are using the validator key for submitting transactions.
        // This allows us to identify transactions coming from bonded validators, to give priority to protocol related transactions.
        let broadcaster = Broadcaster::new(
            tendermint_client.clone(),
            addr,
            sk.clone(),
            settings.fvm.gas_fee_cap.clone(),
            settings.fvm.gas_premium.clone(),
            settings.fvm.gas_overestimation_rate,
        )
        .with_max_retries(settings.broadcast.max_retries)
        .with_retry_delay(settings.broadcast.retry_delay);

        ValidatorContext::new(sk, broadcaster)
    });

    let interpreter = FvmMessageInterpreter::<NamespaceBlockstore, _>::new(
        tendermint_client.clone(),
        validator_ctx,
        settings.contracts_dir(),
        settings.fvm.gas_overestimation_rate,
        settings.fvm.gas_search_step,
        settings.fvm.exec_in_check,
    );
    let interpreter = SignedMessageInterpreter::new(interpreter);
    let interpreter = ChainMessageInterpreter::<_, NamespaceBlockstore>::new(interpreter);
    let interpreter =
        BytesMessageInterpreter::new(interpreter, ProposalPrepareMode::AppendOnly, false);

    let ns = Namespaces::default();
    let db = open_db(&settings, &ns).context("error opening DB")?;

    // Blockstore for actors.
    let state_store =
        NamespaceBlockstore::new(db.clone(), ns.state_store).context("error creating state DB")?;

    let resolve_pool = CheckpointPool::new();

    // If enabled, start a resolver that communicates with the application through the resolve pool.
    if settings.resolver.enabled() {
        let service =
            make_resolver_service(&settings, db.clone(), state_store.clone(), ns.bit_store)?;

        let client = service.client();

        let own_subnet_id = settings.resolver.subnet_id.clone();

        client
            .add_provided_subnet(own_subnet_id.clone())
            .context("error adding own provided subnet.")?;

        let resolver = IpldResolver::new(
            client,
            resolve_pool.queue(),
            settings.resolver.retry_delay,
            own_subnet_id,
        );

        tracing::info!("starting the IPLD Resolver Service...");
        tokio::spawn(async move {
            if let Err(e) = service.run().await {
                tracing::error!("IPLD Resolver Service failed: {e:#}")
            }
        });

        tracing::info!("starting the IPLD Resolver...");
        tokio::spawn(async move { resolver.run().await });
    } else {
        tracing::info!("IPLD Resolver disabled.")
    }

    let (parent_finality_provider, ipc_tuple) = if settings.ipc.is_topdown_enabled() {
        info!("topdown finality enabled");
        let topdown_config = settings.ipc.topdown_config()?;
        let config = fendermint_vm_topdown::Config::new(
            topdown_config.chain_head_delay,
            topdown_config.polling_interval,
            topdown_config.exponential_back_off,
            topdown_config.exponential_retry_limit,
        )
        .with_proposal_delay(topdown_config.proposal_delay)
        .with_max_proposal_range(topdown_config.max_proposal_range);
        let ipc_provider = Arc::new(create_ipc_provider_proxy(&settings)?);
        let finality_provider =
            CachedFinalityProvider::uninitialized(config.clone(), ipc_provider.clone()).await?;
        let p = Arc::new(Toggle::enabled(finality_provider));
        (p, Some((ipc_provider, config)))
    } else {
        info!("topdown finality disabled");
        (Arc::new(Toggle::disabled()), None)
    };

    // Start a snapshot manager in the background.
    let snapshots = if settings.snapshots.enabled {
        let (manager, client) = SnapshotManager::new(
            state_store.clone(),
            SnapshotParams {
                snapshots_dir: settings.snapshots_dir(),
                download_dir: settings.snapshots.download_dir(),
                block_interval: settings.snapshots.block_interval,
                chunk_size: settings.snapshots.chunk_size_bytes,
                hist_size: settings.snapshots.hist_size,
                last_access_hold: settings.snapshots.last_access_hold,
                sync_poll_interval: settings.snapshots.sync_poll_interval,
            },
        )
        .context("failed to create snapshot manager")?;

        tracing::info!("starting the SnapshotManager...");
        let tendermint_client = tendermint_client.clone();
        tokio::spawn(async move { manager.run(tendermint_client).await });

        Some(client)
    } else {
        info!("snapshots disabled");
        None
    };

    let app: App<_, _, AppStore, _> = App::new(
        AppConfig {
            app_namespace: ns.app,
            state_hist_namespace: ns.state_hist,
            state_hist_size: settings.db.state_hist_size,
            builtin_actors_bundle: settings.builtin_actors_bundle(),
        },
        db,
        state_store,
        interpreter,
        resolve_pool,
        parent_finality_provider.clone(),
        snapshots,
    )?;

    if let Some((agent_proxy, config)) = ipc_tuple {
        let app_parent_finality_query = AppParentFinalityQuery::new(app.clone());
        tokio::spawn(async move {
            match launch_polling_syncer(
                app_parent_finality_query,
                config,
                parent_finality_provider,
                agent_proxy,
                tendermint_client,
            )
            .await
            {
                Ok(_) => {}
                Err(e) => tracing::error!("cannot launch polling syncer: {e}"),
            }
        });
    }

    let service = ApplicationService(app);

    // Split it into components.
    let (consensus, mempool, snapshot, info) =
        tower_abci::split::service(service, settings.abci.bound);

    // Hand those components to the ABCI server. This is where tower layers could be added.
    let server = tower_abci::v037::Server::builder()
        .consensus(consensus)
        .snapshot(snapshot)
        .mempool(mempool)
        .info(info)
        .finish()
        .context("error creating ABCI server")?;

    // Run the ABCI server.
    server
        .listen(settings.abci.listen.to_string())
        .await
        .map_err(|e| anyhow!("error listening: {e}"))?;

    Ok(())
}

namespaces! {
    Namespaces {
        app,
        state_hist,
        state_store,
        bit_store
    }
}

/// Open database with all
fn open_db(settings: &Settings, ns: &Namespaces) -> anyhow::Result<RocksDb> {
    let path = settings.data_dir().join("rocksdb");
    info!(
        path = path.to_string_lossy().into_owned(),
        "opening database"
    );
    let db = RocksDb::open_cf(path, &RocksDbConfig::default(), ns.values().iter())?;
    Ok(db)
}

fn make_resolver_service(
    settings: &Settings,
    db: RocksDb,
    state_store: NamespaceBlockstore,
    bit_store_ns: String,
) -> anyhow::Result<ipc_ipld_resolver::Service<libipld::DefaultParams>> {
    // Blockstore for Bitswap.
    let bit_store = NamespaceBlockstore::new(db, bit_store_ns).context("error creating bit DB")?;

    // Blockstore for Bitswap with a fallback on the actor store for reads.
    let bitswap_store = BitswapBlockstore::new(state_store, bit_store);

    let config = to_resolver_config(settings).context("error creating resolver config")?;

    let service = ipc_ipld_resolver::Service::new(config, bitswap_store)
        .context("error creating IPLD Resolver Service")?;

    Ok(service)
}

fn to_resolver_config(settings: &Settings) -> anyhow::Result<ipc_ipld_resolver::Config> {
    use ipc_ipld_resolver::{
        Config, ConnectionConfig, ContentConfig, DiscoveryConfig, MembershipConfig, NetworkConfig,
    };

    let r = &settings.resolver;

    let local_key = {
        let path = r.network.local_key(settings.home_dir());
        let sk = read_secret_key(&path)?;
        let sk = secp256k1::SecretKey::from_bytes(sk.serialize())?;
        Keypair::Secp256k1(secp256k1::Keypair::from(sk))
    };

    let network_name = format!(
        "ipld-resolver-{}-{}",
        r.subnet_id.root_id(),
        r.network.network_name
    );

    let config = Config {
        connection: ConnectionConfig {
            listen_addr: r.connection.listen_addr.clone(),
            expected_peer_count: r.connection.expected_peer_count,
            max_incoming: r.connection.max_incoming,
            max_peers_per_query: r.connection.max_peers_per_query,
            event_buffer_capacity: r.connection.event_buffer_capacity,
        },
        network: NetworkConfig {
            local_key,
            network_name,
        },
        discovery: DiscoveryConfig {
            static_addresses: r.discovery.static_addresses.clone(),
            target_connections: r.discovery.target_connections,
            enable_kademlia: r.discovery.enable_kademlia,
        },
        membership: MembershipConfig {
            static_subnets: r.membership.static_subnets.clone(),
            max_subnets: r.membership.max_subnets,
            publish_interval: r.membership.publish_interval,
            min_time_between_publish: r.membership.min_time_between_publish,
            max_provider_age: r.membership.max_provider_age,
        },
        content: ContentConfig {
            rate_limit_bytes: r.content.rate_limit_bytes,
            rate_limit_period: r.content.rate_limit_period,
        },
    };

    Ok(config)
}

fn to_address(sk: &SecretKey, kind: &AccountKind) -> anyhow::Result<Address> {
    let pk = sk.public_key().serialize();
    match kind {
        AccountKind::Regular => Ok(Address::new_secp256k1(&pk)?),
        AccountKind::Ethereum => Ok(Address::from(EthAddress::new_secp256k1(&pk)?)),
    }
}
