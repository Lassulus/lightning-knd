use std::process::{Command, Output};

use anyhow::{bail, Result};
use api::{
    Channel, FundChannelResponse, GetInfo, NetworkChannel, NetworkNode, NewAddressResponse, Peer,
    SetChannelFeeResponse, WalletBalance, WalletTransferResponse,
};
use bitcoin::secp256k1::PublicKey;

use serde::de;

use test_utils::{TEST_ADDRESS, TEST_PUBLIC_KEY, TEST_SHORT_CHANNEL_ID};

use super::api::create_api_server;

#[tokio::test]
async fn test_cli_get_info() -> Result<()> {
    let output = run_cli("get-info", &[]).await?;
    let _: GetInfo = deserialize(&output.stdout)?;
    Ok(())
}

#[tokio::test]
async fn test_cli_get_balance() -> Result<()> {
    let output = run_cli("get-balance", &[]).await?;
    let _: WalletBalance = deserialize(&output.stdout)?;
    Ok(())
}

#[tokio::test]
async fn test_cli_new_address() -> Result<()> {
    let output = run_cli("new-address", &[]).await?;
    let _: NewAddressResponse = deserialize(&output.stdout)?;
    Ok(())
}

#[tokio::test]
async fn test_cli_withdraw() -> Result<()> {
    let output = run_cli(
        "withdraw",
        &[
            "--address",
            TEST_ADDRESS,
            "--satoshis",
            "1000",
            "--fee-rate",
            "3000perkw",
        ],
    )
    .await?;
    let _: WalletTransferResponse = deserialize(&output.stdout)?;
    Ok(())
}

#[tokio::test]
async fn test_cli_list_channels() -> Result<()> {
    let output = run_cli("list-channels", &[]).await?;
    let _: Vec<Channel> = deserialize(&output.stdout)?;
    Ok(())
}

#[tokio::test]
async fn test_cli_list_peers() -> Result<()> {
    let output = run_cli("list-peers", &[]).await?;
    let _: Vec<Peer> = deserialize(&output.stdout)?;
    Ok(())
}

#[tokio::test]
async fn test_cli_connect_peer() -> Result<()> {
    let output = run_cli("connect-peer", &["--public-key", TEST_PUBLIC_KEY]).await?;
    let _: PublicKey = deserialize(&output.stdout)?;
    Ok(())
}

#[tokio::test]
async fn test_cli_connect_peer_malformed_id() -> Result<()> {
    let output = run_cli("connect-peer", &["--public-key", "abc@1.2"]).await?;
    let _: api::Error = deserialize(&output.stdout)?;
    Ok(())
}

#[tokio::test]
async fn test_cli_disconnect_peer() -> Result<()> {
    let output = run_cli("disconnect-peer", &["--public-key", TEST_PUBLIC_KEY]).await?;

    assert!(&output.stdout.is_empty());
    Ok(())
}

#[tokio::test]
async fn test_cli_open_channel() -> Result<()> {
    let output = run_cli(
        "open-channel",
        &[
            "--public-key",
            TEST_PUBLIC_KEY,
            "--sats",
            "1000",
            "--announce",
            "false",
            "--fee-rate",
            "urgent",
        ],
    )
    .await?;
    let _: FundChannelResponse = deserialize(&output.stdout)?;
    Ok(())
}

#[tokio::test]
async fn test_cli_set_channel_fee() -> Result<()> {
    let output = run_cli(
        "set-channel-fee",
        &[
            "--id",
            &TEST_SHORT_CHANNEL_ID.to_string(),
            "--base-fee",
            "1000",
            "--ppm-fee",
            "200",
        ],
    )
    .await?;
    let _: SetChannelFeeResponse = deserialize(&output.stdout)?;
    Ok(())
}

#[tokio::test]
async fn test_cli_set_channel_fee_all() -> Result<()> {
    let output = run_cli(
        "set-channel-fee",
        &["--id", "all", "--base-fee", "1000", "--ppm-fee", "200"],
    )
    .await?;
    let _: SetChannelFeeResponse = deserialize(&output.stdout)?;
    Ok(())
}

#[tokio::test]
async fn test_cli_close_channel() -> Result<()> {
    let output = run_cli(
        "close-channel",
        &["--id", &TEST_SHORT_CHANNEL_ID.to_string()],
    )
    .await?;
    assert!(output.stdout.is_empty());
    Ok(())
}

#[tokio::test]
async fn test_cli_get_network_node() -> Result<()> {
    let output = run_cli("network-nodes", &["--id", TEST_PUBLIC_KEY]).await?;
    let nodes: Vec<NetworkNode> = deserialize(&output.stdout)?;
    assert!(!nodes.is_empty());
    Ok(())
}

#[tokio::test]
async fn test_cli_list_network_nodes() -> Result<()> {
    let output = run_cli("network-nodes", &[]).await?;
    let nodes: Vec<NetworkNode> = deserialize(&output.stdout)?;
    assert!(!nodes.is_empty());
    Ok(())
}

#[tokio::test]
async fn test_cli_get_network_channel() -> Result<()> {
    let output = run_cli("network-channels", &["--id", "1234"]).await?;
    let result = String::from_utf8(output.stdout)?;
    assert!(result.contains("404"));
    Ok(())
}

#[tokio::test]
async fn test_cli_list_network_channels() -> Result<()> {
    let output = run_cli("network-channels", &[]).await?;
    let channels: Vec<NetworkChannel> = deserialize(&output.stdout)?;
    assert!(channels.is_empty());
    Ok(())
}

fn deserialize<'a, T>(bytes: &'a [u8]) -> Result<T>
where
    T: de::Deserialize<'a>,
{
    match serde_json::from_slice::<T>(bytes) {
        Ok(t) => Ok(t),
        Err(_) => {
            let s = String::from_utf8_lossy(bytes);
            bail!("Expected json output, but got: {}", s)
        }
    }
}

async fn run_cli(command: &str, extra_args: &[&str]) -> Result<Output> {
    let context = create_api_server().await?;

    let output = Command::new(env!("CARGO_BIN_EXE_kld-cli"))
        .args([
            "--target",
            &context.settings.rest_api_address,
            "--cert-path",
            &format!("{}/kld.crt", context.settings.certs_dir),
            "--macaroon-path",
            &format!("{}/macaroons/admin.macaroon", context.settings.data_dir),
            command,
        ])
        .args(extra_args)
        .output()
        .unwrap();

    if !output.status.success() {
        panic!("{}", String::from_utf8(output.stderr).unwrap());
    }
    Ok(output)
}
