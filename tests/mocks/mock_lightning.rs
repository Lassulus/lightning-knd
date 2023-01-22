use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
};

use anyhow::Result;
use async_trait::async_trait;
use bitcoin::{consensus::deserialize, hashes::Hash, secp256k1::PublicKey, Network, Txid};
use hex::FromHex;
use lightning::{
    chain::transaction::OutPoint,
    ln::{
        channelmanager::{ChannelCounterparty, ChannelDetails},
        features::InitFeatures,
    },
    util::config::UserConfig,
};
use lightning_knd::api::{LightningInterface, OpenChannelResult, Peer, PeerStatus};
use test_utils::random_public_key;

use super::{TEST_PUBLIC_KEY, TEST_TX};

pub struct MockLightning {
    pub num_peers: usize,
    pub num_nodes: usize,
    pub num_channels: usize,
    pub wallet_balance: u64,
    pub channels: Vec<ChannelDetails>,
}

impl Default for MockLightning {
    fn default() -> Self {
        let channel = ChannelDetails {
            channel_id: [1u8; 32],
            counterparty: ChannelCounterparty {
                node_id: PublicKey::from_str(TEST_PUBLIC_KEY).unwrap(),
                features: InitFeatures::empty(),
                unspendable_punishment_reserve: 5000,
                forwarding_info: None,
                outbound_htlc_minimum_msat: Some(1000),
                outbound_htlc_maximum_msat: Some(100),
            },
            funding_txo: Some(OutPoint {
                txid: Txid::all_zeros(),
                index: 2,
            }),
            channel_type: None,
            short_channel_id: Some(34234124),
            outbound_scid_alias: None,
            inbound_scid_alias: None,
            channel_value_satoshis: 1000000,
            unspendable_punishment_reserve: Some(10000),
            user_channel_id: 3434232,
            balance_msat: 10001,
            outbound_capacity_msat: 100000,
            next_outbound_htlc_limit_msat: 500,
            inbound_capacity_msat: 200000,
            confirmations_required: Some(3),
            confirmations: Some(10),
            force_close_spend_delay: Some(6),
            is_outbound: true,
            is_channel_ready: true,
            is_usable: true,
            is_public: true,
            inbound_htlc_minimum_msat: Some(300),
            inbound_htlc_maximum_msat: Some(300000),
            config: None,
        };
        Self {
            num_peers: 5,
            num_nodes: 6,
            num_channels: 7,
            wallet_balance: 8,
            channels: vec![channel],
        }
    }
}

#[async_trait]
impl LightningInterface for MockLightning {
    fn alias(&self) -> String {
        "test".to_string()
    }
    fn identity_pubkey(&self) -> PublicKey {
        random_public_key()
    }

    fn graph_num_nodes(&self) -> usize {
        self.num_nodes
    }

    fn graph_num_channels(&self) -> usize {
        self.num_channels
    }

    fn block_height(&self) -> usize {
        50000
    }

    fn network(&self) -> bitcoin::Network {
        Network::Bitcoin
    }
    fn num_active_channels(&self) -> usize {
        0
    }

    fn num_inactive_channels(&self) -> usize {
        0
    }

    fn num_pending_channels(&self) -> usize {
        0
    }
    fn num_peers(&self) -> usize {
        self.num_peers
    }

    fn wallet_balance(&self) -> u64 {
        self.wallet_balance
    }

    fn version(&self) -> String {
        "v0.1".to_string()
    }

    fn list_channels(&self) -> Vec<ChannelDetails> {
        self.channels.clone()
    }

    fn alias_of(&self, _node_id: PublicKey) -> Option<String> {
        Some("test_node".to_string())
    }

    fn addresses(&self) -> Vec<String> {
        vec![
            "127.0.0.1:2324".to_string(),
            "194.454.23.2:2020".to_string(),
        ]
    }

    async fn open_channel(
        &self,
        _their_network_key: PublicKey,
        _channel_value_satoshis: u64,
        _push_msat: Option<u64>,
        _override_config: Option<UserConfig>,
    ) -> Result<OpenChannelResult> {
        let transaction =
            deserialize::<bitcoin::Transaction>(&Vec::<u8>::from_hex(TEST_TX).unwrap()).unwrap();
        let txid = transaction.txid();
        Ok(OpenChannelResult {
            transaction,
            txid,
            channel_id: [1u8; 32],
        })
    }

    async fn list_peers(&self) -> Result<Vec<Peer>> {
        Ok(vec![Peer {
            public_key: PublicKey::from_str(TEST_PUBLIC_KEY).unwrap(),
            socked_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080),
            status: PeerStatus::Connected,
            alias: "test".to_string(),
        }])
    }

    async fn connect_peer(
        &self,
        _public_key: PublicKey,
        _socket_addr: Option<SocketAddr>,
    ) -> Result<()> {
        Ok(())
    }

    fn disconnect_peer(&self, _public_key: PublicKey) {}
}