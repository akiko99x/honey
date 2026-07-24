//! WireGuard / AmneziaWG key generation, IP allocation and client-config
//! rendering. Keys are Curve25519 (same as REALITY) encoded as *standard*
//! base64 — the format `wg`/`awg` tools and clients expect.
use anyhow::{anyhow, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use std::net::Ipv4Addr;
use x25519_dalek::{PublicKey, StaticSecret};

pub struct Keypair {
    pub private_key: String,
    pub public_key: String,
}

/// Fresh Curve25519 keypair in standard base64.
pub fn generate() -> Result<Keypair> {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| anyhow!("rng failed: {e}"))?;
    let secret = StaticSecret::from(seed);
    let public = PublicKey::from(&secret);
    Ok(Keypair {
        private_key: STANDARD.encode(secret.to_bytes()),
        public_key: STANDARD.encode(public.to_bytes()),
    })
}

/// Derive the public key from a stored standard-base64 private key.
pub fn public_from_private(private_b64: &str) -> Result<String> {
    let bytes = STANDARD
        .decode(private_b64.trim())
        .map_err(|e| anyhow!("bad wg private key: {e}"))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow!("wg private key must be 32 bytes"))?;
    let secret = StaticSecret::from(arr);
    Ok(STANDARD.encode(PublicKey::from(&secret).to_bytes()))
}

/// Parse an `a.b.c.d/prefix` IPv4 CIDR into (network base, prefix).
pub fn parse_cidr(cidr: &str) -> Result<(Ipv4Addr, u8)> {
    let (addr, prefix) = cidr
        .split_once('/')
        .ok_or_else(|| anyhow!("cidr must be a.b.c.d/nn"))?;
    let ip: Ipv4Addr = addr
        .trim()
        .parse()
        .map_err(|_| anyhow!("bad cidr address"))?;
    let prefix: u8 = prefix
        .trim()
        .parse()
        .map_err(|_| anyhow!("bad cidr prefix"))?;
    if prefix > 32 {
        return Err(anyhow!("cidr prefix out of range"));
    }
    Ok((ip, prefix))
}

/// The server's own address inside the pool (first usable host, `.1`).
pub fn server_address(cidr: &str) -> Result<Ipv4Addr> {
    let (base, prefix) = parse_cidr(cidr)?;
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    let net = u32::from(base) & mask;
    Ok(Ipv4Addr::from(net + 1))
}

/// Allocate the lowest free host `/32` in the pool, skipping the network,
/// broadcast, the server's `.1`, and any already-taken addresses.
pub fn allocate_ip(cidr: &str, taken: &[Ipv4Addr]) -> Result<Ipv4Addr> {
    let (base, prefix) = parse_cidr(cidr)?;
    if prefix >= 31 {
        return Err(anyhow!("wg pool too small (need /30 or larger)"));
    }
    let mask = u32::MAX << (32 - prefix);
    let net = u32::from(base) & mask;
    let broadcast = net | !mask;
    let taken: std::collections::HashSet<u32> = taken.iter().map(|a| u32::from(*a)).collect();
    // usable hosts are net+2 .. broadcast-1 (net+1 is the server).
    for host in (net + 2)..broadcast {
        if !taken.contains(&host) {
            return Ok(Ipv4Addr::from(host));
        }
    }
    Err(anyhow!("wg address pool exhausted"))
}

/// AmneziaWG obfuscation parameters. Must match between server and client.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AmneziaParams {
    pub jc: u32,
    pub jmin: u32,
    pub jmax: u32,
    pub s1: u32,
    pub s2: u32,
    pub h1: u32,
    pub h2: u32,
    pub h3: u32,
    pub h4: u32,
}

impl AmneziaParams {
    /// Reasonable defaults (Amnezia client presets) with randomized magic headers.
    pub fn generate() -> Result<Self> {
        let mut buf = [0u8; 16];
        getrandom::getrandom(&mut buf).map_err(|e| anyhow!("rng failed: {e}"))?;
        // headers must be distinct and >= 5 to avoid colliding with the 4 real
        // WireGuard message types; derive four large distinct values.
        let h = |i: usize| {
            5 + (u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]) % 0x7FFF_FFF0)
        };
        Ok(Self {
            jc: 4,
            jmin: 40,
            jmax: 70,
            s1: 0,
            s2: 0,
            h1: h(0),
            h2: h(4),
            h3: h(8),
            h4: h(12),
        })
    }

    /// Render the AmneziaWG `[Interface]` lines (shared by server & client).
    fn lines(&self) -> String {
        format!(
            "Jc = {}\nJmin = {}\nJmax = {}\nS1 = {}\nS2 = {}\nH1 = {}\nH2 = {}\nH3 = {}\nH4 = {}\n",
            self.jc, self.jmin, self.jmax, self.s1, self.s2, self.h1, self.h2, self.h3, self.h4
        )
    }
}

/// Inputs for one client's config file.
pub struct ClientConfig<'a> {
    pub client_private: &'a str,
    pub client_address: &'a str, // "10.7.0.2"
    pub dns: &'a str,
    pub mtu: i32,
    pub server_public: &'a str,
    pub endpoint: &'a str, // "1.2.3.4:51820"
    pub amnezia: Option<&'a AmneziaParams>,
}

/// Render a WireGuard / AmneziaWG client `.conf`.
pub fn client_config(c: &ClientConfig) -> String {
    let mut s = String::from("[Interface]\n");
    s.push_str(&format!("PrivateKey = {}\n", c.client_private));
    s.push_str(&format!("Address = {}/32\n", c.client_address));
    if !c.dns.is_empty() {
        s.push_str(&format!("DNS = {}\n", c.dns));
    }
    if c.mtu > 0 {
        s.push_str(&format!("MTU = {}\n", c.mtu));
    }
    if let Some(a) = c.amnezia {
        s.push_str(&a.lines());
    }
    s.push_str("\n[Peer]\n");
    s.push_str(&format!("PublicKey = {}\n", c.server_public));
    s.push_str(&format!("Endpoint = {}\n", c.endpoint));
    s.push_str("AllowedIPs = 0.0.0.0/0, ::/0\n");
    s.push_str("PersistentKeepalive = 25\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_roundtrip_and_derive() {
        let kp = generate().unwrap();
        assert_eq!(public_from_private(&kp.private_key).unwrap(), kp.public_key);
    }

    #[test]
    fn allocates_sequential_hosts() {
        assert_eq!(
            server_address("10.7.0.0/24").unwrap(),
            Ipv4Addr::new(10, 7, 0, 1)
        );
        let first = allocate_ip("10.7.0.0/24", &[]).unwrap();
        assert_eq!(first, Ipv4Addr::new(10, 7, 0, 2));
        let second = allocate_ip("10.7.0.0/24", &[first]).unwrap();
        assert_eq!(second, Ipv4Addr::new(10, 7, 0, 3));
    }

    #[test]
    fn renders_client_config() {
        let cfg = client_config(&ClientConfig {
            client_private: "priv",
            client_address: "10.7.0.2",
            dns: "1.1.1.1",
            mtu: 1420,
            server_public: "spub",
            endpoint: "1.2.3.4:51820",
            amnezia: None,
        });
        assert!(cfg.contains("Address = 10.7.0.2/32"));
        assert!(cfg.contains("Endpoint = 1.2.3.4:51820"));
        assert!(cfg.contains("AllowedIPs = 0.0.0.0/0"));
    }
}
