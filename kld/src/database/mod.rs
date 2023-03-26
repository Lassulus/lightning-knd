mod ldk_database;
pub mod peer;
mod wallet_database;

pub use ldk_database::LdkDatabase;
pub use wallet_database::WalletDatabase;

use anyhow::{Context, Result};
use log::{error, info};
use openssl::ssl::{SslConnector, SslFiletype, SslMethod};
use postgres_openssl::MakeTlsConnector;
use tokio_postgres::Client;

use settings::Settings;

#[macro_export]
macro_rules! to_i64 {
    ($int: expr) => {
        i64::try_from($int).unwrap()
    };
}

#[macro_export]
macro_rules! from_i64 {
    ($row: expr, $name: expr) => {
        $row.get::<&str, i64>(&$name).try_into().unwrap()
    };
}

#[macro_export]
macro_rules! from_maybe_i64 {
    ($row: expr, $name: expr) => {
        $row.get::<&str, Option<i64>>(&$name)
            .map(|x| x.try_into().unwrap())
    };
}

pub async fn connection(settings: &Settings) -> Result<Client> {
    let log_safe_params = format!(
        "host={} port={} user={} dbname={}",
        settings.database_host,
        settings.database_port,
        settings.database_user,
        settings.database_name
    );
    let mut builder = SslConnector::builder(SslMethod::tls())?;
    builder.set_ca_file(&settings.database_ca_cert_path)?;
    builder.set_certificate_file(&settings.database_client_cert_path, SslFiletype::PEM)?;
    builder.set_private_key_file(&settings.database_client_key_path, SslFiletype::PEM)?;
    let connector = MakeTlsConnector::new(builder.build());
    let (client, connection) = tokio_postgres::connect(&log_safe_params, connector)
        .await
        .with_context(|| format!("could not connect to database ({log_safe_params})"))?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            error!("Database connection error: {}", e);
        }
    });
    Ok(client)
}

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("src/database/sql");
}

pub async fn migrate_database(settings: &Settings) -> Result<()> {
    let mut client = connection(settings)
        .await
        .with_context(|| format!("cannot connect to database '{}'", settings.database_name))?;
    info!("Running database migrations");
    embedded::migrations::runner()
        .run_async(&mut client)
        .await?;
    Ok(())
}