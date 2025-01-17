use crate::bitcoind::{BitcoindClient, BitcoindUtxoLookup, Synchronised};
use crate::wallet::{Wallet, WalletInterface};

use crate::database::{LdkDatabase, WalletDatabase};
use anyhow::{anyhow, bail, Context, Result};
use api::FeeRate;
use async_trait::async_trait;
use bitcoin::secp256k1::PublicKey;
use bitcoin::{BlockHash, Network, Transaction};
use lightning::chain::channelmonitor::ChannelMonitor;
use lightning::chain::keysinterface::{InMemorySigner, KeysManager};
use lightning::chain::BestBlock;
use lightning::chain::{self, ChannelMonitorUpdateStatus};
use lightning::chain::{chainmonitor, Watch};
use lightning::ln::channelmanager::{self, ChannelDetails};
use lightning::ln::channelmanager::{ChainParameters, ChannelManagerReadArgs};
use lightning::ln::msgs::NetAddress;
use lightning::ln::peer_handler::{IgnoringMessageHandler, MessageHandler};
use lightning::routing::gossip::{ChannelInfo, NodeId, NodeInfo, P2PGossipSync};
use lightning::routing::router::DefaultRouter;
use lightning::routing::scoring::{ProbabilisticScorer, ProbabilisticScoringParameters};
use lightning::util::config::UserConfig;

use crate::logger::KldLogger;
use lightning::util::indexed_map::IndexedMap;
use lightning_background_processor::{BackgroundProcessor, GossipSync};
use lightning_block_sync::SpvClient;
use lightning_block_sync::UnboundedCache;
use lightning_block_sync::{init, BlockSourceResult};
use lightning_block_sync::{poll, BlockSource};
use log::{error, info, warn};
use rand::random;
use settings::Settings;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tokio::runtime::Handle;
use tokio::sync::oneshot::{self, Receiver, Sender};
use tokio::sync::RwLock;

use super::event_handler::EventHandler;
use super::net_utils::PeerAddress;
use super::payment_info::PaymentInfoStorage;
use super::peer_manager::PeerManager;
use super::{
    ldk_error, ChainMonitor, ChannelManager, LdkPeerManager, LightningInterface, NetworkGraph,
    OnionMessenger, OpenChannelResult, Peer, PeerStatus,
};

#[async_trait]
impl LightningInterface for Controller {
    fn identity_pubkey(&self) -> PublicKey {
        self.channel_manager.get_our_node_id()
    }

    async fn synced(&self) -> Result<bool> {
        Ok(self.bitcoind_client.is_synchronised().await?
            && self.channel_manager.current_best_block().block_hash()
                == self
                    .bitcoind_client
                    .get_best_block()
                    .await
                    .map_err(|e| anyhow!(e.into_inner()))?
                    .0)
    }

    fn graph_num_nodes(&self) -> usize {
        self.network_graph.read_only().nodes().len()
    }

    fn graph_num_channels(&self) -> usize {
        self.network_graph.read_only().channels().len()
    }

    fn num_peers(&self) -> usize {
        self.peer_manager.get_connected_peers().len()
    }

    fn wallet_balance(&self) -> u64 {
        match self.wallet.balance() {
            Ok(balance) => balance.confirmed,
            Err(e) => {
                error!("Unable to get wallet balance for metrics: {}", e);
                0
            }
        }
    }

    fn alias(&self) -> String {
        self.settings.node_name.clone()
    }

    async fn block_height(&self) -> Result<u64> {
        self.bitcoind_client
            .get_blockchain_info()
            .await
            .map(|i| i.blocks)
    }

    fn network(&self) -> bitcoin::Network {
        self.settings.bitcoin_network.into()
    }

    fn num_active_channels(&self) -> usize {
        self.channel_manager
            .list_channels()
            .iter()
            .filter(|c| c.is_usable)
            .count()
    }

    fn num_inactive_channels(&self) -> usize {
        self.channel_manager
            .list_channels()
            .iter()
            .filter(|c| c.is_channel_ready && !c.is_usable)
            .count()
    }

    fn num_pending_channels(&self) -> usize {
        self.channel_manager
            .list_channels()
            .iter()
            .filter(|c| !c.is_channel_ready)
            .count()
    }

    fn list_channels(&self) -> Vec<ChannelDetails> {
        self.channel_manager.list_channels()
    }

    async fn open_channel(
        &self,
        their_network_key: PublicKey,
        channel_value_satoshis: u64,
        push_msat: Option<u64>,
        fee_rate: Option<FeeRate>,
        override_config: Option<UserConfig>,
    ) -> Result<OpenChannelResult> {
        if !self.bitcoind_client.is_synchronised().await? {
            bail!("Bitcoind is syncronising blockchain")
        }
        if !self.peer_manager.is_connected(&their_network_key) {
            return Err(anyhow!("Peer not connected"));
        }
        let user_channel_id: u128 = random();
        let channel_id = self
            .channel_manager
            .create_channel(
                their_network_key,
                channel_value_satoshis,
                push_msat.unwrap_or_default(),
                user_channel_id,
                override_config,
            )
            .map_err(ldk_error)?;
        let receiver = self
            .async_api_requests
            .funding_transactions
            .insert(user_channel_id, fee_rate.unwrap_or_default())
            .await;
        let transaction = receiver.await??;
        let txid = transaction.txid();
        Ok(OpenChannelResult {
            transaction,
            txid,
            channel_id,
        })
    }

    async fn close_channel(
        &self,
        channel_id: &[u8; 32],
        counterparty_node_id: &PublicKey,
    ) -> Result<()> {
        if !self.bitcoind_client.is_synchronised().await? {
            bail!("Bitcoind is syncronising blockchain")
        }
        self.channel_manager
            .close_channel(channel_id, counterparty_node_id)
            .map_err(ldk_error)
    }

    fn set_channel_fee(
        &self,
        counterparty_node_id: &PublicKey,
        channel_ids: &[[u8; 32]],
        forwarding_fee_proportional_millionths: Option<u32>,
        forwarding_fee_base_msat: Option<u32>,
    ) -> Result<(u32, u32)> {
        let mut channel_config = self.user_config().channel_config;
        if let Some(fee) = forwarding_fee_proportional_millionths {
            channel_config.forwarding_fee_proportional_millionths = fee;
        }
        if let Some(fee) = forwarding_fee_base_msat {
            channel_config.forwarding_fee_base_msat = fee;
        }
        self.channel_manager
            .update_channel_config(counterparty_node_id, channel_ids, &channel_config)
            .map_err(ldk_error)?;
        Ok((
            channel_config.forwarding_fee_base_msat,
            channel_config.forwarding_fee_proportional_millionths,
        ))
    }

    fn alias_of(&self, public_key: &PublicKey) -> Option<String> {
        self.network_graph
            .read_only()
            .node(&NodeId::from_pubkey(public_key))
            .and_then(|n| n.announcement_info.as_ref().map(|a| a.alias.to_string()))
    }

    /// List all the peers that we have channels with along with their connection status.
    async fn list_peers(&self) -> Result<Vec<Peer>> {
        let connected_peers = self.peer_manager.get_connected_peers();
        let channel_peers: Vec<PublicKey> = self
            .channel_manager
            .list_channels()
            .iter()
            .map(|c| c.counterparty.node_id)
            .collect();
        let persistent_peers = self.database.fetch_peers().await?;

        let mut response = vec![];

        let mut all_pub_keys: HashSet<PublicKey> = HashSet::from_iter(
            connected_peers
                .iter()
                .map(|p| p.0)
                .collect::<Vec<PublicKey>>(),
        );
        all_pub_keys.extend(channel_peers);
        all_pub_keys.extend(persistent_peers.keys());

        for public_key in all_pub_keys {
            let net_address = connected_peers
                .iter()
                .find(|p| p.0 == public_key)
                .and_then(|p| p.1.clone());
            let status = if net_address.is_some() {
                PeerStatus::Connected
            } else {
                PeerStatus::Disconnected
            };
            response.push(Peer {
                public_key,
                net_address,
                status,
                alias: self.alias_of(&public_key).unwrap_or_default(),
            });
        }
        Ok(response)
    }

    async fn connect_peer(
        &self,
        public_key: PublicKey,
        peer_address: Option<PeerAddress>,
    ) -> Result<()> {
        if let Some(net_address) = peer_address {
            self.peer_manager
                .connect_peer(public_key, net_address)
                .await
        } else {
            let addresses: Vec<PeerAddress> = self
                .network_graph
                .read_only()
                .get_addresses(&public_key)
                .context("No addresses found for node")?
                .into_iter()
                .filter(|a| matches!(a, NetAddress::IPv4 { addr: _, port: _ }))
                .map(PeerAddress)
                .collect();
            for address in addresses {
                if let Err(e) = self
                    .peer_manager
                    .connect_peer(public_key, address.clone())
                    .await
                {
                    info!("Could not connect to {public_key}@{address}. {}", e);
                } else {
                    return Ok(());
                }
            }
            Err(anyhow!("Could not connect to any peer addresses."))
        }
    }

    async fn disconnect_peer(&self, public_key: PublicKey) -> Result<()> {
        self.peer_manager.disconnect_by_node_id(public_key).await
    }

    fn public_addresses(&self) -> Vec<String> {
        self.settings.public_addresses.clone()
    }

    fn get_node(&self, node_id: &NodeId) -> Option<NodeInfo> {
        self.network_graph.read_only().node(node_id).cloned()
    }

    fn nodes(&self) -> IndexedMap<NodeId, NodeInfo> {
        self.network_graph.read_only().nodes().clone()
    }

    fn get_channel(&self, channel_id: u64) -> Option<ChannelInfo> {
        self.network_graph.read_only().channel(channel_id).cloned()
    }

    fn channels(&self) -> IndexedMap<u64, ChannelInfo> {
        self.network_graph.read_only().channels().clone()
    }

    // Use this to override the default/startup config.
    fn user_config(&self) -> UserConfig {
        *self.channel_manager.get_current_default_configuration()
    }
}

pub(crate) struct AsyncAPIRequests {
    pub funding_transactions: AsyncSenders<u128, FeeRate, Result<Transaction>>,
}

impl AsyncAPIRequests {
    fn new() -> AsyncAPIRequests {
        AsyncAPIRequests {
            funding_transactions: AsyncSenders::new(),
        }
    }
}

pub(crate) struct AsyncSenders<K, V, RV> {
    senders: RwLock<HashMap<K, (V, Sender<RV>)>>,
}

impl<K: Eq + Hash, V: Clone, RV> AsyncSenders<K, V, RV> {
    fn new() -> AsyncSenders<K, V, RV> {
        AsyncSenders {
            senders: RwLock::new(HashMap::new()),
        }
    }

    async fn insert(&self, k: K, v: V) -> Receiver<RV> {
        let (tx, rx) = oneshot::channel::<RV>();
        self.senders.write().await.insert(k, (v, tx));
        rx
    }

    pub async fn get(&self, k: &K) -> Option<(V, impl FnOnce(RV))> {
        if let Some((v, tx)) = self.senders.write().await.remove(k) {
            let respond = |rv: RV| {
                if tx.send(rv).is_err() {
                    warn!("Receiver dropped");
                }
            };
            return Some((v, respond));
        }
        None
    }

    pub async fn respond(&self, k: &K, rv: RV) {
        if let Some((_, tx)) = self.senders.write().await.remove(k) {
            if tx.send(rv).is_err() {
                warn!("Receiver dropped");
            }
        }
    }
}

pub struct Controller {
    settings: Arc<Settings>,
    database: Arc<LdkDatabase>,
    bitcoind_client: Arc<BitcoindClient>,
    channel_manager: Arc<ChannelManager>,
    peer_manager: Arc<PeerManager>,
    network_graph: Arc<NetworkGraph>,
    wallet: Arc<Wallet<WalletDatabase, BitcoindClient>>,
    async_api_requests: Arc<AsyncAPIRequests>,
    background_processor: Arc<Mutex<Option<BackgroundProcessor>>>,
}

impl Controller {
    pub fn stop(&self) {
        // Disconnect our peers and stop accepting new connections. This ensures we don't continue
        // updating our channel data after we've stopped the background processor.
        self.peer_manager.disconnect_all_peers();
        if let Some(bgp) = self.background_processor.lock().unwrap().take() {
            bgp.stop()
                .expect("Background processor did not stop cleanly");
        }
    }

    pub async fn start_ldk(
        settings: Arc<Settings>,
        database: Arc<LdkDatabase>,
        bitcoind_client: Arc<BitcoindClient>,
        wallet: Arc<Wallet<WalletDatabase, BitcoindClient>>,
        seed: &[u8; 32],
    ) -> Result<Controller> {
        // BitcoindClient implements the FeeEstimator trait, so it'll act as our fee estimator.
        let fee_estimator = bitcoind_client.clone();

        // BitcoindClient implements the BroadcasterInterface trait, so it'll act as our transaction broadcaster.
        let broadcaster = bitcoind_client.clone();

        let network = settings.bitcoin_network.into();

        // Initialize the ChainMonitor
        let chain_monitor: Arc<ChainMonitor> = Arc::new(chainmonitor::ChainMonitor::new(
            None,
            broadcaster.clone(),
            KldLogger::global(),
            fee_estimator.clone(),
            database.clone(),
        ));

        let is_first_start = database
            .is_first_start()
            .await
            .context("could not check if database has been initialized")?;
        // Initialize the KeysManager
        // The key seed that we use to derive the node privkey (that corresponds to the node pubkey) and
        // other secret key material.
        let cur = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap();
        let keys_manager = Arc::new(KeysManager::new(seed, cur.as_secs(), cur.subsec_nanos()));

        let network_graph = Arc::new(
            database
                .fetch_graph()
                .await
                .context("Could not query network graph from database")?
                .unwrap_or_else(|| NetworkGraph::new(network, KldLogger::global())),
        );
        let scorer = Arc::new(Mutex::new(
            database
                .fetch_scorer(
                    ProbabilisticScoringParameters::default(),
                    network_graph.clone(),
                )
                .await?
                .unwrap_or_else(|| {
                    ProbabilisticScorer::new(
                        ProbabilisticScoringParameters::default(),
                        network_graph.clone(),
                        KldLogger::global(),
                    )
                }),
        ));
        let random_seed_bytes: [u8; 32] = random();
        let router = Arc::new(DefaultRouter::new(
            network_graph.clone(),
            KldLogger::global(),
            random_seed_bytes,
            scorer.clone(),
        ));

        // Initialize the ChannelManager
        let mut channelmonitors = database
            .fetch_channel_monitors(keys_manager.as_ref(), keys_manager.as_ref())
            .await?;
        let mut user_config = UserConfig::default();
        user_config
            .channel_handshake_limits
            .force_announced_channel_preference = false;
        user_config.channel_handshake_config.announced_channel = true;

        let (channel_manager_blockhash, channel_manager) = {
            if is_first_start {
                let getinfo_resp = bitcoind_client.get_blockchain_info().await?;
                let chain_params = ChainParameters {
                    network,
                    best_block: BestBlock::new(
                        getinfo_resp.best_block_hash,
                        getinfo_resp.blocks as u32,
                    ),
                };
                let new_channel_manager = channelmanager::ChannelManager::new(
                    fee_estimator.clone(),
                    chain_monitor.clone(),
                    broadcaster.clone(),
                    router,
                    KldLogger::global(),
                    keys_manager.clone(),
                    keys_manager.clone(),
                    keys_manager.clone(),
                    user_config,
                    chain_params,
                );
                (getinfo_resp.best_block_hash, new_channel_manager)
            } else {
                let channel_monitor_mut_refs =
                    channelmonitors.iter_mut().map(|(_, cm)| cm).collect();
                let read_args = ChannelManagerReadArgs::new(
                    keys_manager.clone(),
                    keys_manager.clone(),
                    keys_manager.clone(),
                    fee_estimator.clone(),
                    chain_monitor.clone(),
                    broadcaster.clone(),
                    router,
                    KldLogger::global(),
                    user_config,
                    channel_monitor_mut_refs,
                );
                database
                    .fetch_channel_manager(read_args)
                    .await
                    .context("failed to query channel manager from database")?
            }
        };
        let channel_manager: Arc<ChannelManager> = Arc::new(channel_manager);

        let gossip_sync = Arc::new_cyclic(|u| {
            let utxo_lookup = Arc::new(BitcoindUtxoLookup::new(
                &settings,
                bitcoind_client.clone(),
                network_graph.clone(),
                u.clone(),
            ));
            P2PGossipSync::new(
                network_graph.clone(),
                Some(utxo_lookup),
                KldLogger::global(),
            )
        });

        let onion_messenger: Arc<OnionMessenger> = Arc::new(OnionMessenger::new(
            keys_manager.clone(),
            keys_manager.clone(),
            KldLogger::global(),
            IgnoringMessageHandler {},
        ));
        let ephemeral_bytes: [u8; 32] = random();
        let current_time = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let lightning_msg_handler = MessageHandler {
            chan_handler: channel_manager.clone(),
            route_handler: gossip_sync.clone(),
            onion_message_handler: onion_messenger,
        };
        let ldk_peer_manager = Arc::new(LdkPeerManager::new(
            lightning_msg_handler,
            current_time.try_into().unwrap(),
            &ephemeral_bytes,
            KldLogger::global(),
            IgnoringMessageHandler {},
            keys_manager.clone(),
        ));
        let peer_manager = Arc::new(PeerManager::new(
            ldk_peer_manager.clone(),
            channel_manager.clone(),
            database.clone(),
            settings.clone(),
        )?);

        let async_api_requests = Arc::new(AsyncAPIRequests::new());
        // Handle LDK Events
        // TODO: persist payment info to disk
        let inbound_payments: PaymentInfoStorage = Arc::new(Mutex::new(HashMap::new()));
        let outbound_payments: PaymentInfoStorage = Arc::new(Mutex::new(HashMap::new()));
        let event_handler = EventHandler::new(
            channel_manager.clone(),
            bitcoind_client.clone(),
            keys_manager.clone(),
            inbound_payments,
            outbound_payments,
            network_graph.clone(),
            wallet.clone(),
            async_api_requests.clone(),
            Handle::current(),
        );

        // Background Processing
        let background_processor = BackgroundProcessor::start(
            database.clone(),
            event_handler,
            chain_monitor.clone(),
            channel_manager.clone(),
            GossipSync::p2p(gossip_sync),
            ldk_peer_manager.clone(),
            KldLogger::global(),
            Some(scorer),
        );

        let bitcoind_client_clone = bitcoind_client.clone();
        let channel_manager_clone = channel_manager.clone();
        let peer_manager_clone = peer_manager.clone();
        tokio::spawn(async move {
            bitcoind_client_clone
                .wait_for_blockchain_synchronisation()
                .await;
            Controller::sync_to_chain_tip(
                network,
                bitcoind_client_clone,
                chain_monitor,
                channel_manager_blockhash,
                channel_manager_clone,
                channelmonitors,
            )
            .await
            .unwrap();

            peer_manager_clone.listen().await;
            peer_manager_clone.keep_channel_peers_connected();
            peer_manager_clone.regularly_broadcast_node_announcement();
        });

        Ok(Controller {
            settings,
            database,
            bitcoind_client,
            channel_manager,
            peer_manager,
            network_graph,
            wallet,
            async_api_requests,
            background_processor: Arc::new(Mutex::new(Some(background_processor))),
        })
    }

    async fn sync_to_chain_tip(
        network: Network,
        bitcoind_client: Arc<BitcoindClient>,
        chain_monitor: Arc<ChainMonitor>,
        channel_manager_blockhash: BlockHash,
        channel_manager: Arc<ChannelManager>,
        channelmonitors: Vec<(BlockHash, ChannelMonitor<InMemorySigner>)>,
    ) -> BlockSourceResult<()> {
        // Sync ChannelMonitors and ChannelManager to chain tip
        let mut chain_listener_channel_monitors = Vec::new();
        let mut cache = UnboundedCache::new();

        let mut chain_listeners = vec![(
            channel_manager_blockhash,
            channel_manager.as_ref() as &(dyn chain::Listen + Send + Sync),
        )];

        for (blockhash, channel_monitor) in channelmonitors {
            let outpoint = channel_monitor.get_funding_txo().0;
            chain_listener_channel_monitors.push((
                blockhash,
                (
                    channel_monitor,
                    bitcoind_client.clone(),
                    bitcoind_client.clone(),
                    KldLogger::global(),
                ),
                outpoint,
            ));
        }

        for monitor_listener_info in chain_listener_channel_monitors.iter_mut() {
            chain_listeners.push((
                monitor_listener_info.0,
                &monitor_listener_info.1 as &(dyn chain::Listen + Send + Sync),
            ));
        }
        let chain_tip = init::synchronize_listeners(
            bitcoind_client.clone(),
            network,
            &mut cache,
            chain_listeners,
        )
        .await?;

        // Give ChannelMonitors to ChainMonitor
        for (_, (channel_monitor, _, _, _), funding_outpoint) in chain_listener_channel_monitors {
            assert_eq!(
                chain_monitor.watch_channel(funding_outpoint, channel_monitor),
                ChannelMonitorUpdateStatus::Completed
            );
        }

        // Connect and Disconnect Blocks
        tokio::spawn(async move {
            let chain_poller = poll::ChainPoller::new(bitcoind_client, network);
            let chain_listener = (chain_monitor, channel_manager);
            let mut spv_client =
                SpvClient::new(chain_tip, chain_poller, &mut cache, &chain_listener);
            loop {
                if let Err(e) = spv_client.poll_best_tip().await {
                    error!("{}", e.into_inner())
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });

        Ok(())
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.stop()
    }
}
