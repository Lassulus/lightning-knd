use anyhow::{anyhow, bail, Context, Result};

use log::warn;
use regex::Regex;
use serde::Serialize;
use serde_derive::Deserialize;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use super::secrets::Secrets;
use super::NixosFlake;

/// IpV6String allows prefix only address format and normal ipv6 address
///
/// Some providers include the subnet in their address shown in the webinterface i.e. 2607:5300:203:5cdf::/64
/// This format is rejected by IpAddr in Rust and we store subnets in a different configuration option.
/// This struct detects such cashes in the kuutamo.toml file and converting it to 2607:5300:203:5cdf:: with a warning message, providing a more user-friendly experience.
type IpV6String = String;

trait AsIpAddr {
    /// Handle ipv6 subnet identifier and normalize to a valide ip address and a mask.
    fn normalize(&self) -> Result<(IpAddr, Option<u8>)>;
}

impl AsIpAddr for IpV6String {
    fn normalize(&self) -> Result<(IpAddr, Option<u8>)> {
        if let Some(idx) = self.find('/') {
            let mask = self
                .get(idx + 1..self.len())
                .map(|i| i.parse::<u8>())
                .with_context(|| {
                    format!("ipv6_address contains invalid subnet identifier: {self}")
                })?
                .ok();

            match self.get(0..idx) {
                Some(addr_str) if mask.is_some() => {
                    if let Ok(addr) = addr_str.parse::<IpAddr>() {
                        warn!("{self:} contains a ipv6 subnet identifier... will use {addr:} for ipv6_address and {:} for ipv6_cidr", mask.unwrap_or_default());
                        Ok((addr, mask))
                    } else {
                        Err(anyhow!("ipv6_address is not invalid"))
                    }
                }
                _ => Err(anyhow!("ipv6_address is not invalid")),
            }
        } else {
            Ok((self.parse::<IpAddr>()?, None))
        }
    }
}

#[derive(Debug, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    global: Global,

    #[serde(default)]
    host_defaults: HostConfig,
    #[serde(default)]
    hosts: HashMap<String, HostConfig>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct NearKeyFile {
    pub account_id: String,
    pub public_key: String,
    // Credential files generated which near cli works with have private_key
    // rather than secret_key field.  To make it possible to read those from
    // neard add private_key as an alias to this field so either will work.
    #[serde(alias = "private_key")]
    pub secret_key: String,
}

fn default_secret_directory() -> PathBuf {
    PathBuf::from("secrets")
}

fn default_flake() -> String {
    "github:kuutamolabs/lightning-knd".to_string()
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct CockroachPeer {
    pub name: String,
    pub ipv4_address: Option<IpAddr>,
    pub ipv6_address: Option<IpAddr>,
}

#[derive(Debug, Default, Deserialize)]
struct HostConfig {
    #[serde(default)]
    ipv4_address: Option<IpAddr>,
    #[serde(default)]
    ipv4_gateway: Option<IpAddr>,
    #[serde(default)]
    ipv4_cidr: Option<u8>,
    #[serde(default)]
    nixos_module: Option<String>,
    #[serde(default)]
    extra_nixos_modules: Vec<String>,

    #[serde(default)]
    pub mac_address: Option<String>,
    #[serde(default)]
    ipv6_address: Option<IpV6String>,
    #[serde(default)]
    ipv6_gateway: Option<IpAddr>,
    #[serde(default)]
    ipv6_cidr: Option<u8>,

    #[serde(default)]
    public_ssh_keys: Vec<String>,

    #[serde(default)]
    install_ssh_user: Option<String>,

    #[serde(default)]
    ssh_hostname: Option<String>,

    #[serde(default)]
    pub disks: Option<Vec<PathBuf>>,

    #[serde(default)]
    pub bitcoind_disks: Option<Vec<PathBuf>>,
}

/// NixOS host configuration
#[derive(Debug, PartialEq, Eq, Clone, Serialize)]
pub struct Host {
    /// Name identifying the host
    pub name: String,

    /// NixOS module to use as a base for the host from the flake
    pub nixos_module: String,

    /// Extra NixOS modules to include in the system
    pub extra_nixos_modules: Vec<String>,

    /// Mac address of the public interface to use
    pub mac_address: Option<String>,

    /// Public ipv4 address of the host
    pub ipv4_address: Option<IpAddr>,
    /// Cidr of the public ipv4 address
    pub ipv4_cidr: Option<u8>,
    /// Public ipv4 gateway ip address
    pub ipv4_gateway: Option<IpAddr>,

    /// Public ipv6 address of the host
    pub ipv6_address: Option<IpAddr>,
    /// Cidr of the public ipv6 address
    pub ipv6_cidr: Option<u8>,
    /// Public ipv6 gateway address of the host
    pub ipv6_gateway: Option<IpAddr>,

    /// SSH Username used when connecting during installation
    pub install_ssh_user: String,

    /// SSH hostname used for connecting
    pub ssh_hostname: String,

    /// Public ssh keys that will be added to the nixos configuration
    pub public_ssh_keys: Vec<String>,

    /// Block device paths to use for installing
    pub disks: Vec<PathBuf>,

    /// Block device paths to use for bitcoind's blockchain state
    pub bitcoind_disks: Vec<PathBuf>,

    /// CockroachDB nodes to connect to
    pub cockroach_peers: Vec<CockroachPeer>,
}

impl Host {
    /// Returns prepared secrets directory for host
    pub fn secrets(&self, secrets_dir: &Path) -> Result<Secrets> {
        let lightning = secrets_dir.join("lightning");
        let cockroachdb = secrets_dir.join("cockroachdb");

        let secret_files = vec![
            // for kld
            (
                PathBuf::from("/var/lib/secrets/kld/ca.pem"),
                fs::read_to_string(lightning.join("ca.pem")).context("failed to read ca.pem")?,
            ),
            (
                PathBuf::from("/var/lib/secrets/kld/kld.pem"),
                fs::read_to_string(lightning.join(format!("{}.pem", self.name)))
                    .context("failed to read kld.pem")?,
            ),
            (
                PathBuf::from("/var/lib/secrets/kld/kld.key"),
                fs::read_to_string(lightning.join(format!("{}.key", self.name)))
                    .context("failed to read kld.key")?,
            ),
            (
                PathBuf::from("/var/lib/secrets/kld/client.kld.crt"),
                fs::read_to_string(cockroachdb.join("client.kld.crt"))
                    .context("failed to read client.kld.crt")?,
            ),
            (
                PathBuf::from("/var/lib/secrets/kld/client.kld.key"),
                fs::read_to_string(cockroachdb.join("client.kld.key"))
                    .context("failed to read client.kld.key")?,
            ),
            // for cockroachdb
            (
                PathBuf::from("/var/lib/secrets/cockroachdb/ca.crt"),
                fs::read_to_string(cockroachdb.join("ca.crt")).context("failed to read ca.crt")?,
            ),
            (
                PathBuf::from("/var/lib/secrets/cockroachdb/client.root.crt"),
                fs::read_to_string(cockroachdb.join("client.root.crt"))
                    .context("failed to read client.root.crt")?,
            ),
            (
                PathBuf::from("/var/lib/secrets/cockroachdb/client.root.key"),
                fs::read_to_string(cockroachdb.join("client.root.key"))
                    .context("failed to read client.root.key")?,
            ),
            (
                PathBuf::from("/var/lib/secrets/cockroachdb/node.crt"),
                fs::read_to_string(cockroachdb.join(format!("{}.node.crt", self.name)))
                    .context("failed to read node.crt")?,
            ),
            (
                PathBuf::from("/var/lib/secrets/cockroachdb/node.key"),
                fs::read_to_string(cockroachdb.join(format!("{}.node.key", self.name)))
                    .context("failed to read node.key")?,
            ),
        ];

        Secrets::new(secret_files.iter()).context("failed to prepare uploading secrets")
    }
    /// The hostname to which we will deploy
    pub fn deploy_ssh_target(&self) -> String {
        format!("root@{}", self.ssh_hostname)
    }
    /// The hostname to which we will deploy
    pub fn flake_uri(&self, flake: &NixosFlake) -> String {
        format!("{}#{}", flake.path().display(), self.name)
    }
}

/// Global configuration affecting all hosts
#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Default)]
pub struct Global {
    /// Flake url where the nixos configuration is
    #[serde(default = "default_flake")]
    pub flake: String,

    /// Directory where the secrets are stored i.e. certificates
    #[serde(default = "default_secret_directory")]
    pub secret_directory: PathBuf,
}

fn validate_global(global: &Global, working_directory: &Path) -> Result<Global> {
    let mut global = global.clone();
    if global.secret_directory.is_relative() {
        global.secret_directory = working_directory.join(global.secret_directory);
    };
    Ok(global)
}

fn validate_host(name: &str, host: &HostConfig, default: &HostConfig) -> Result<Host> {
    let name = name.to_string();

    if name.is_empty() || name.len() > 63 {
        bail!(
            "a host's name must be between 1 and 63 characters long, got: '{}'",
            name
        );
    }
    let hostname_regex = Regex::new(r"^[a-z0-9][a-z0-9\-]{0,62}$").unwrap();
    if !hostname_regex.is_match(&name) {
        bail!("a host's name must only contain letters from a to z, the digits from 0 to 9, and the hyphen (-). But not starting with a hyphen. got: '{}'", name);
    }
    let mac_address = if let Some(ref a) = &host.mac_address {
        let mac_address_regex = Regex::new(r"^([0-9A-Fa-f]{2}[:-]){5}([0-9A-Fa-f]{2})$").unwrap();
        if !mac_address_regex.is_match(a) {
            bail!("mac address does match a valid format: {} (valid example value: 02:42:34:d1:18:7a)", a);
        }
        Some(a.clone())
    } else {
        None
    };

    let ipv4_address = if let Some(address) = host.ipv4_address {
        if !address.is_ipv4() {
            bail!("ipv4_address provided for hosts.{name} is not an ipv4 address: {address}");
        }
        // FIXME: this is currently an unstable feature
        //if address.is_global() {
        //    warn!("ipv4_address provided for hosts.{} is not a public ipv4 address: {}. This might not work with near mainnet", name, address);
        //}
        Some(address)
    } else {
        None
    };

    let ipv4_cidr = if let Some(cidr) = host.ipv4_cidr.or(default.ipv4_cidr) {
        if !(0..32_u8).contains(&cidr) {
            bail!("ipv4_cidr for hosts.{name} is not between 0 and 32: {cidr}")
        }
        Some(cidr)
    } else {
        None
    };

    let nixos_module = host
        .nixos_module
        .as_deref()
        .with_context(|| format!("no nixos_module provided for hosts.{name}"))?
        .to_string();

    let mut extra_nixos_modules = vec![];
    extra_nixos_modules.extend_from_slice(&host.extra_nixos_modules);
    extra_nixos_modules.extend_from_slice(&default.extra_nixos_modules);

    let ipv4_gateway = host.ipv4_gateway.or(default.ipv4_gateway);

    let ipv6_cidr = host.ipv6_cidr.or(default.ipv6_cidr);

    let ipv6_gateway = host.ipv6_gateway.or(default.ipv6_gateway);

    let (ipv6_address, mask) = if let Some(ipv6_address) = host.ipv6_address.as_ref() {
        let (ipv6_address, mask) = ipv6_address
            .normalize()
            .with_context(|| format!("ipv6_address provided for host.{name:} is not valid"))?;
        if !ipv6_address.is_ipv6() {
            bail!("value provided in ipv6_address for hosts.{name} is not an ipv6 address: {ipv6_address}");
        }

        if let Some(ipv6_cidr) = ipv6_cidr {
            if !(0..128_u8).contains(&ipv6_cidr) {
                bail!("ipv6_cidr for hosts.{name} is not between 0 and 128: {ipv6_cidr}")
            }
        } else if mask.is_none() {
            bail!("no ipv6_cidr provided for hosts.{name}");
        }

        if ipv6_gateway.is_none() {
            bail!("no ipv6_gateway provided for hosts.{name}")
        }

        // FIXME: this is currently an unstable feature
        //if ipv6_address.is_global() {
        //    warn!("ipv6_address provided for hosts.{} is not a public ipv6 address: {}. This might not work with near mainnet", name, ipv6_address);
        //}

        (Some(ipv6_address), mask)
    } else {
        (None, None)
    };

    let address = ipv4_address
        .or(ipv6_address)
        .with_context(|| format!("no ipv4_address or ipv6_address provided for hosts.{name}"))?;

    if ipv4_gateway.is_none() && ipv6_gateway.is_none() {
        bail!("no ipv4_gateway or ipv6_gateway provided for hosts.{name}");
    }

    let ssh_hostname = host
        .ssh_hostname
        .as_ref()
        .or(default.ssh_hostname.as_ref())
        .cloned()
        .unwrap_or_else(|| address.to_string());

    let install_ssh_user = host
        .install_ssh_user
        .as_ref()
        .or(default.install_ssh_user.as_ref())
        .cloned()
        .unwrap_or_else(|| String::from("root"));

    let mut public_ssh_keys = vec![];
    public_ssh_keys.extend_from_slice(&host.public_ssh_keys);
    public_ssh_keys.extend_from_slice(&default.public_ssh_keys);
    if public_ssh_keys.is_empty() {
        bail!("no public_ssh_keys provided for hosts.{name}");
    }

    let default_disks = vec![PathBuf::from("/dev/nvme0n1"), PathBuf::from("/dev/nvme1n1")];
    let disks = host
        .disks
        .as_ref()
        .or(default.disks.as_ref())
        .unwrap_or(&default_disks)
        .to_vec();

    if disks.is_empty() {
        bail!("no disks specified for hosts.{name}");
    }

    let default_bitcoind_disks = vec![];

    let bitcoind_disks = host
        .bitcoind_disks
        .as_ref()
        .or(default.bitcoind_disks.as_ref())
        .unwrap_or(&default_bitcoind_disks)
        .to_vec();

    Ok(Host {
        name,
        nixos_module,
        extra_nixos_modules,
        install_ssh_user,
        ssh_hostname,
        mac_address,
        ipv4_address,
        ipv4_cidr,
        ipv4_gateway,
        ipv6_address,
        ipv6_cidr: mask.or(ipv6_cidr),
        ipv6_gateway,
        public_ssh_keys,
        disks,
        bitcoind_disks,
        cockroach_peers: vec![],
    })
}

/// Validated configuration
pub struct Config {
    /// Hosts as defined in the configuration
    pub hosts: BTreeMap<String, Host>,
    /// Configuration affecting all hosts
    pub global: Global,
}

/// Parse toml configuration
pub fn parse_config(content: &str, working_directory: &Path) -> Result<Config> {
    let config: ConfigFile = toml::from_str(content)?;
    let mut hosts = config
        .hosts
        .iter()
        .map(|(name, host)| {
            Ok((
                name.to_string(),
                validate_host(name, host, &config.host_defaults)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let cockroach_peers = hosts
        .iter()
        .map(|(name, host)| CockroachPeer {
            name: name.to_string(),
            ipv4_address: host.ipv4_address,
            ipv6_address: host.ipv6_address,
        })
        .collect::<Vec<_>>();
    for host in hosts.values_mut() {
        host.cockroach_peers = cockroach_peers.clone();
    }
    let kld_nodes = hosts
        .iter()
        .filter(|(_, host)| host.nixos_module == "kld-node")
        .count();
    if kld_nodes != 1 {
        bail!("Exactly one kld-node is required, found {}", kld_nodes);
    }
    let cockroach_nodes = hosts
        .iter()
        .filter(|(_, host)| host.nixos_module == "cockroachdb-node")
        .count();
    if cockroach_nodes != 0 && cockroach_nodes < 2 {
        bail!(
            "Either zero or two cockroach-nodes are required, found {}",
            cockroach_nodes
        );
    }

    let global = validate_global(&config.global, working_directory)?;

    Ok(Config { hosts, global })
}

/// Load configuration from path
pub fn load_configuration(path: &Path) -> Result<Config> {
    let content = fs::read_to_string(path).context("Cannot read file")?;
    let working_directory = path.parent().with_context(|| {
        format!(
            "Cannot determine working directory from path: {}",
            path.display()
        )
    })?;
    parse_config(&content, working_directory)
}

#[cfg(test)]
pub(crate) const TEST_CONFIG: &str = r#"
[global]
flake = "github:myfork/near-staking-knd"

[host_defaults]
public_ssh_keys = [
  '''ssh-ed25519 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA foobar'''
]
ipv4_cidr = 24
ipv6_cidr = 48
ipv4_gateway = "199.127.64.1"
ipv6_gateway = "2605:9880:400::1"

[hosts]
[hosts.kld-00]
nixos_module = "kld-node"
ipv4_address = "199.127.64.2"
ipv6_address = "2605:9880:400::2"
ipv6_cidr = 48

[hosts.db-00]
nixos_module = "cockroachdb-node"
ipv4_address = "199.127.64.3"
ipv6_address = "2605:9880:400::3"

[hosts.db-01]
nixos_module = "cockroachdb-node"
ipv4_address = "199.127.64.4"
ipv6_address = "2605:9880:400::4"
"#;

#[test]
pub fn test_parse_config() -> Result<()> {
    use std::str::FromStr;

    let config = parse_config(TEST_CONFIG, Path::new("/"))?;
    assert_eq!(config.global.flake, "github:myfork/near-staking-knd");

    let hosts = &config.hosts;
    assert_eq!(hosts.len(), 3);
    assert_eq!(
        hosts["kld-00"]
            .ipv4_address
            .context("missing ipv4_address")?,
        IpAddr::from_str("199.127.64.2").unwrap()
    );
    assert_eq!(hosts["kld-00"].ipv4_cidr.context("missing ipv4_cidr")?, 24);
    assert_eq!(
        hosts["db-00"]
            .ipv4_gateway
            .context("missing ipv4_gateway")?,
        IpAddr::from_str("199.127.64.1").unwrap()
    );
    assert_eq!(
        hosts["db-00"].ipv6_address,
        IpAddr::from_str("2605:9880:400::3").ok()
    );
    assert_eq!(hosts["kld-00"].ipv6_cidr, Some(48));
    assert_eq!(
        hosts["kld-00"].ipv6_gateway,
        IpAddr::from_str("2605:9880:400::1").ok()
    );

    parse_config(TEST_CONFIG, Path::new("/"))?;

    Ok(())
}

#[test]
fn test_valid_ip_string_for_ipv6() {
    let ip: IpV6String = "2607:5300:203:5cdf::".into();
    assert_eq!(ip.normalize().unwrap().1, None);

    let subnet_identifire: IpV6String = "2607:5300:203:5cdf::/64".into();
    assert_eq!(
        subnet_identifire.normalize().unwrap().0,
        ip.normalize().unwrap().0
    );
    assert_eq!(subnet_identifire.normalize().unwrap().1, Some(64));
}

#[test]
fn test_invalid_string_for_ipv6() {
    let mut invalid_str: IpV6String = "2607:5300:203:7cdf::/".into();
    assert!(invalid_str.normalize().is_err());

    invalid_str = "/2607:5300:203:7cdf::".into();
    assert!(invalid_str.normalize().is_err());
}

#[test]
fn test_validate_host() -> Result<()> {
    let mut config = HostConfig {
        ipv4_address: Some(
            "192.168.0.1"
                .parse::<IpAddr>()
                .context("Invalid IP address")?,
        ),
        nixos_module: Some("kld-node".to_string()),
        ipv4_cidr: Some(0),
        ipv4_gateway: Some(
            "192.168.255.255"
                .parse::<IpAddr>()
                .context("Invalid IP address")?,
        ),
        ipv6_address: None,
        ipv6_gateway: None,
        ipv6_cidr: None,
        public_ssh_keys: vec!["".to_string()],
        ..Default::default()
    };
    assert_eq!(
        validate_host("ipv4-only", &config, &HostConfig::default()).unwrap(),
        Host {
            name: "ipv4-only".to_string(),
            nixos_module: "kld-node".to_string(),
            extra_nixos_modules: Vec::new(),
            mac_address: None,
            ipv4_address: Some(
                "192.168.0.1"
                    .parse::<IpAddr>()
                    .context("Invalid IP address")?
            ),
            ipv4_cidr: Some(0),
            ipv4_gateway: Some(
                "192.168.255.255"
                    .parse::<IpAddr>()
                    .context("Invalid IP address")?
            ),
            ipv6_address: None,
            ipv6_cidr: None,
            ipv6_gateway: None,
            install_ssh_user: "root".to_string(),
            ssh_hostname: "192.168.0.1".to_string(),
            public_ssh_keys: vec!["".to_string()],
            disks: vec!["/dev/nvme0n1".into(), "/dev/nvme1n1".into()],
            cockroach_peers: vec![],
            bitcoind_disks: vec![],
        }
    );

    // If `ipv6_address` is provied, the `ipv6_gateway` and `ipv6_cidr` should be provided too,
    // else the error will raise
    config.ipv6_address = Some("2607:5300:203:6cdf::".into());
    assert!(validate_host("ipv4-only", &config, &HostConfig::default()).is_err());

    config.ipv6_gateway = Some(
        "2607:5300:0203:6cff:00ff:00ff:00ff:00ff"
            .parse::<IpAddr>()
            .unwrap(),
    );
    assert!(validate_host("ipv4-only", &config, &HostConfig::default()).is_err());

    // The `ipv6_cidr` could be provided by subnet in address field
    config.ipv6_address = Some("2607:5300:203:6cdf::/64".into());
    assert!(validate_host("ipv4-only", &config, &HostConfig::default()).is_ok());

    Ok(())
}
