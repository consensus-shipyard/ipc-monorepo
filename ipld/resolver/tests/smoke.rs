// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT
//! Test that a cluster of IPLD resolver can be started in memory,
//! that they bootstrap from  each other and are able to resolve CIDs.
//!
//! Run the tests as follows:
//! ```ignore
//! RUST_LOG=debug cargo test -p ipc_ipld_resolver --test smoke resolve
//! ```

// For inspiration on testing libp2p look at:
// * https://github.com/libp2p/rust-libp2p/blob/v0.50.0/misc/multistream-select/tests/transport.rs
// * https://github.com/libp2p/rust-libp2p/blob/v0.50.0/protocols/ping/tests/ping.rs
// * https://github.com/libp2p/rust-libp2p/blob/v0.50.0/protocols/gossipsub/tests/smoke.rs
// They all use a different combination of `MemoryTransport` and executors.
// These tests attempt to use `MemoryTransport` so it's quicker, with `Swarm::with_tokio_executor`
// so we can leave the polling to the `Service` running in a `Task`, rather than do it from the test
// (although these might be orthogonal).

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use anyhow::anyhow;
use fvm_ipld_encoding::IPLD_RAW;
use fvm_ipld_hamt::Hamt;
use fvm_shared::{address::Address, ActorID};
use ipc_ipld_resolver::{
    Client, Config, ConnectionConfig, ContentConfig, DiscoveryConfig, Event, MembershipConfig,
    NetworkConfig, Service, VoteRecord,
};
use ipc_api::subnet_id::SubnetID;
use libipld::{
    multihash::{Code, MultihashDigest},
    Cid,
};
use libp2p::{
    core::{
        muxing::StreamMuxerBox,
        transport::{Boxed, MemoryTransport},
    },
    identity::Keypair,
    mplex,
    multiaddr::Protocol,
    plaintext::PlainText2Config,
    yamux, Multiaddr, PeerId, Transport,
};
use rand::{rngs::StdRng, Rng, SeedableRng};

mod store;
use store::*;
use tokio::{sync::broadcast, time::timeout};

struct Agent {
    config: Config,
    client: Client,
    events: broadcast::Receiver<Event>,
    store: TestBlockstore,
}

struct Cluster {
    agents: Vec<Agent>,
}

impl Cluster {
    pub fn size(&self) -> usize {
        self.agents.len()
    }
}

struct ClusterBuilder {
    size: u32,
    rng: StdRng,
    services: Vec<Service<TestStoreParams>>,
    agents: Vec<Agent>,
}

impl ClusterBuilder {
    fn new(size: u32) -> Self {
        // Each port has to be unique, so each test must use a different seed.
        // This is shared between all instances.
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seed = COUNTER.fetch_add(1, Ordering::Relaxed);
        Self::new_with_seed(size, seed)
    }

    fn new_with_seed(size: u32, seed: u64) -> Self {
        Self {
            size,
            rng: rand::rngs::StdRng::seed_from_u64(seed),
            services: Default::default(),
            agents: Default::default(),
        }
    }

    /// Add a node with randomized address, optionally bootstrapping from an existing node.
    fn add_node(&mut self, bootstrap: Option<usize>) {
        let bootstrap_addr = bootstrap.map(|i| {
            let config = &self.agents[i].config;
            let peer_id = config.network.local_peer_id();
            let mut addr = config.connection.listen_addr.clone();
            addr.push(Protocol::P2p(peer_id.into()));
            addr
        });
        let config = make_config(&mut self.rng, self.size, bootstrap_addr);
        let (service, store) = make_service(config.clone());
        let client = service.client();
        let events = service.subscribe();
        self.services.push(service);
        self.agents.push(Agent {
            config,
            client,
            events,
            store,
        });
    }

    /// Start running all services
    fn run(self) -> Cluster {
        for service in self.services {
            tokio::task::spawn(async move { service.run().await.expect("error running service") });
        }
        Cluster {
            agents: self.agents,
        }
    }
}

/// Start a cluster of agents from a single bootstrap node,
/// make available some content on one agent and resolve it from another.
#[tokio::test]
async fn single_bootstrap_single_provider_resolve_one() {
    let _ = env_logger::builder().is_test(true).try_init();
    //env_logger::init();

    // Choose agents.
    let cluster_size = 3;
    let bootstrap_idx = 0;
    let provider_idx = 1;
    let resolver_idx = 2;

    // TODO: Get the seed from QuickCheck
    let mut builder = ClusterBuilder::new(cluster_size);

    // Build a cluster of nodes.
    for i in 0..builder.size {
        builder.add_node(if i == 0 { None } else { Some(bootstrap_idx) });
    }

    // Start the swarms.
    let mut cluster = builder.run();

    // Insert a CID of a complex recursive data structure.
    let cid = insert_test_data(&mut cluster.agents[provider_idx]).expect("failed to insert data");

    // Sanity check that we can read the data back.
    check_test_data(&mut cluster.agents[provider_idx], &cid).expect("failed to read back the data");

    // Wait a little for the cluster to connect.
    // TODO: Wait on some condition instead of sleep.
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Announce the support of some subnet.
    let subnet_id = make_subnet_id(1001);

    cluster.agents[provider_idx]
        .client
        .add_provided_subnet(subnet_id.clone())
        .expect("failed to add provided subnet");

    // Wait a little for the gossip to spread and peer lookups to happen, then another round of gossip.
    // TODO: Wait on some condition instead of sleep.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Ask for the CID to be resolved from by another peer.
    cluster.agents[resolver_idx]
        .client
        .resolve(cid, subnet_id.clone())
        .await
        .expect("failed to send request")
        .expect("failed to resolve content");

    // Check that the CID is deposited into the store of the requestor.
    check_test_data(&mut cluster.agents[resolver_idx], &cid).expect("failed to resolve from store");
}

/// Start two agents, subscribe to the same subnet, publish and receive a vote.
#[tokio::test]
async fn single_bootstrap_publish_receive_vote() {
    let _ = env_logger::builder().is_test(true).try_init();
    //env_logger::init();

    // TODO: Get the seed from QuickCheck
    let mut builder = ClusterBuilder::new(2);

    // Build a cluster of nodes.
    for i in 0..builder.size {
        builder.add_node(if i == 0 { None } else { Some(0) });
    }

    // Start the swarms.
    let mut cluster = builder.run();

    // Wait a little for the cluster to connect.
    // TODO: Wait on some condition instead of sleep.
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Announce the support of some subnet.
    let subnet_id = make_subnet_id(1001);

    for i in 0..cluster.size() {
        cluster.agents[i]
            .client
            .add_provided_subnet(subnet_id.clone())
            .expect("failed to add provided subnet");
    }

    // Wait a little for the gossip to spread and peer lookups to happen, then another round of gossip.
    // TODO: Wait on some condition instead of sleep.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Vote on some random CID.
    let validator_key = Keypair::generate_secp256k1();
    let cid = Cid::new_v1(IPLD_RAW, Code::Sha2_256.digest(b"foo"));
    let vote = VoteRecord::signed(&validator_key, subnet_id, cid, "finalized".into())
        .expect("failed to sign vote");

    // Pubilish vote
    cluster.agents[0]
        .client
        .publish_vote(vote.clone())
        .expect("failed to send vote");

    // Receive vote.
    let event = timeout(Duration::from_secs(2), cluster.agents[1].events.recv())
        .await
        .expect("timeout receiving vote")
        .expect("error receiving vote");

    if let Event::ReceivedVote(v) = event {
        assert_eq!(&*v, vote.record());
    } else {
        panic!("unexpected {event:?}")
    }
}

/// Start two agents, pin a subnet, publish preemptively and receive.
#[tokio::test]
async fn single_bootstrap_publish_receive_preemptive() {
    let _ = env_logger::builder().is_test(true).try_init();

    // TODO: Get the seed from QuickCheck
    let mut builder = ClusterBuilder::new(2);

    // Build a cluster of nodes.
    for i in 0..builder.size {
        builder.add_node(if i == 0 { None } else { Some(0) });
    }

    // Start the swarms.
    let mut cluster = builder.run();

    // Wait a little for the cluster to connect.
    // TODO: Wait on some condition instead of sleep.
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Pin a subnet on the bootstrap node.
    let subnet_id = make_subnet_id(1001);

    cluster.agents[0]
        .client
        .pin_subnet(subnet_id.clone())
        .expect("failed to pin subnet");

    // TODO: Wait on some condition instead of sleep.
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Publish some content from the other agent.
    let data = vec![1, 2, 3];
    cluster.agents[1]
        .client
        .publish_preemptive(subnet_id.clone(), data.clone())
        .expect("failed to send vote");

    // Receive pre-emptive data..
    let event = timeout(Duration::from_secs(2), cluster.agents[0].events.recv())
        .await
        .expect("timeout receiving data")
        .expect("error receiving data");

    if let Event::ReceivedPreemptive(s, d) = event {
        assert_eq!(s, subnet_id);
        assert_eq!(d, data);
    } else {
        panic!("unexpected {event:?}")
    }
}

fn make_service(config: Config) -> (Service<TestStoreParams>, TestBlockstore) {
    let store = TestBlockstore::default();
    let svc = Service::new_with_transport(config, store.clone(), build_transport).unwrap();
    (svc, store)
}

fn make_config(rng: &mut StdRng, cluster_size: u32, bootstrap_addr: Option<Multiaddr>) -> Config {
    let config = Config {
        connection: ConnectionConfig {
            listen_addr: Multiaddr::from(Protocol::Memory(rng.gen::<u64>())),
            expected_peer_count: cluster_size,
            max_incoming: cluster_size,
            max_peers_per_query: cluster_size,
            event_buffer_capacity: cluster_size,
        },
        network: NetworkConfig {
            local_key: Keypair::generate_secp256k1(),
            network_name: "smoke-test".to_owned(),
        },
        discovery: DiscoveryConfig {
            static_addresses: bootstrap_addr.iter().cloned().collect(),
            target_connections: cluster_size.try_into().unwrap(),
            enable_kademlia: true,
        },
        membership: MembershipConfig {
            static_subnets: vec![],
            max_subnets: 10,
            publish_interval: Duration::from_secs(5),
            min_time_between_publish: Duration::from_secs(1),
            max_provider_age: Duration::from_secs(60),
        },
        content: ContentConfig {
            rate_limit_bytes: 1 << 20,
            rate_limit_period: Duration::from_secs(60),
        },
    };

    config
}

/// Builds an in-memory transport for libp2p to communicate over.
fn build_transport(local_key: Keypair) -> Boxed<(PeerId, StreamMuxerBox)> {
    let auth_config = PlainText2Config {
        local_public_key: local_key.public(),
    };

    let mplex_config = {
        let mut mplex_config = mplex::MplexConfig::new();
        mplex_config.set_max_buffer_size(usize::MAX);

        let mut yamux_config = yamux::YamuxConfig::default();
        yamux_config.set_max_buffer_size(16 * 1024 * 1024);
        yamux_config.set_receive_window_size(16 * 1024 * 1024);
        // yamux_config.set_window_update_mode(WindowUpdateMode::OnRead);
        libp2p::core::upgrade::SelectUpgrade::new(yamux_config, mplex_config)
    };

    MemoryTransport::default()
        .upgrade(libp2p::core::upgrade::Version::V1)
        .authenticate(auth_config)
        .multiplex(mplex_config)
        .boxed()
}

/// Make a subnet under a rootnet.
fn make_subnet_id(actor_id: ActorID) -> SubnetID {
    let act = Address::new_id(actor_id);
    let root = SubnetID::new_root(0);
    SubnetID::new_from_parent(&root, act)
}

/// Insert a HAMT into the block store of an agent.
fn insert_test_data(agent: &mut Agent) -> anyhow::Result<Cid> {
    let mut hamt: Hamt<_, String, u32> = Hamt::new(&agent.store);

    // Insert enough data into the HAMT to make sure it grows from a single `Node`.
    for i in 0..1000 {
        hamt.set(i, format!("value {i}"))?;
    }
    let cid = hamt.flush()?;

    Ok(cid)
}

fn check_test_data(agent: &mut Agent, cid: &Cid) -> anyhow::Result<()> {
    let hamt: Hamt<_, String, u32> = Hamt::load(cid, &agent.store)?;

    // Check all the data inserted by `insert_test_data`.
    for i in 0..1000 {
        match hamt.get(&i)? {
            None => return Err(anyhow!("key {i} is missing")),
            Some(v) if *v != format!("value {i}") => return Err(anyhow!("unexpected value: {v}")),
            _ => {}
        }
    }

    Ok(())
}
