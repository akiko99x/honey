//! IP → location lookup for the usage heatmap.
//!
//! Two layers:
//!   * a built-in table of **reserved / special-use** IPv4 ranges (RFC 1918,
//!     loopback, CGNAT, link-local, …) — these are exact and need no data file;
//!   * an optional operator-supplied country table loaded from
//!     `HONEY_GEOIP_FILE` (CSV `start_ip,end_ip,CC` — e.g. exported from
//!     MaxMind GeoLite2). Country attribution is only as good as that file.
//!
//! Nothing here guesses a country from a hardcoded national block list: shipping
//! invented ranges would produce confident-looking but wrong geography. Without
//! a loaded table every public address resolves to `unknown`, and the panel says
//! so explicitly.
use std::net::{IpAddr, Ipv4Addr};
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct Range {
    pub start: u32,
    pub end: u32,
    pub code: String,
}

static TABLE: OnceLock<Vec<Range>> = OnceLock::new();
static LOADED_COUNTRIES: OnceLock<usize> = OnceLock::new();

/// Exact special-use ranges (RFC 1122/1918/3927/6598/5771). `local` covers
/// anything that can never be geolocated to a country.
fn builtin() -> Vec<Range> {
    let r = |a: [u8; 4], b: [u8; 4], code: &str| Range {
        start: u32::from(Ipv4Addr::new(a[0], a[1], a[2], a[3])),
        end: u32::from(Ipv4Addr::new(b[0], b[1], b[2], b[3])),
        code: code.to_string(),
    };
    vec![
        r([0, 0, 0, 0], [0, 255, 255, 255], "local"), // "this network"
        r([10, 0, 0, 0], [10, 255, 255, 255], "local"), // RFC1918
        r([100, 64, 0, 0], [100, 127, 255, 255], "local"), // RFC6598 CGNAT
        r([127, 0, 0, 0], [127, 255, 255, 255], "local"), // loopback
        r([169, 254, 0, 0], [169, 254, 255, 255], "local"), // link-local
        r([172, 16, 0, 0], [172, 31, 255, 255], "local"), // RFC1918
        r([192, 0, 0, 0], [192, 0, 0, 255], "local"), // IETF protocol
        r([192, 0, 2, 0], [192, 0, 2, 255], "local"), // TEST-NET-1
        r([192, 168, 0, 0], [192, 168, 255, 255], "local"), // RFC1918
        r([198, 18, 0, 0], [198, 19, 255, 255], "local"), // benchmarking
        r([198, 51, 100, 0], [198, 51, 100, 255], "local"), // TEST-NET-2
        r([203, 0, 113, 0], [203, 0, 113, 255], "local"), // TEST-NET-3
        r([224, 0, 0, 0], [239, 255, 255, 255], "local"), // multicast
        r([240, 0, 0, 0], [255, 255, 255, 255], "local"), // reserved
    ]
}

fn parse_line(line: &str) -> Option<Range> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut parts = line.split(',');
    let start: Ipv4Addr = parts.next()?.trim().parse().ok()?;
    let end: Ipv4Addr = parts.next()?.trim().parse().ok()?;
    let code = parts.next()?.trim().trim_matches('"');
    if code.is_empty() || code.len() > 8 {
        return None;
    }
    Some(Range {
        start: u32::from(start),
        end: u32::from(end),
        code: code.to_ascii_uppercase(),
    })
}

/// Load the table once: built-in special-use ranges plus, when configured, the
/// operator's country CSV. Safe to call repeatedly.
pub fn init() {
    if TABLE.get().is_some() {
        return;
    }
    let mut table = builtin();
    let mut countries = 0usize;
    if let Ok(path) = std::env::var("HONEY_GEOIP_FILE") {
        let path = path.trim().to_string();
        if !path.is_empty() {
            match std::fs::read_to_string(&path) {
                Ok(body) => {
                    for line in body.lines() {
                        if let Some(range) = parse_line(line) {
                            table.push(range);
                            countries += 1;
                        }
                    }
                    tracing::info!(
                        code = "M0115",
                        "geoip table loaded: {countries} country ranges from {path}"
                    );
                }
                Err(error) => tracing::warn!(
                    code = "M0115",
                    "geoip table at {path} could not be read: {error}"
                ),
            }
        }
    }
    // longest-prefix-ish: sort by start, then by narrower range first so a
    // country entry inside a broader block wins the binary search neighbourhood.
    table.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then((a.end - a.start).cmp(&(b.end - b.start)))
    });
    let _ = TABLE.set(table);
    let _ = LOADED_COUNTRIES.set(countries);
}

/// How many country ranges were loaded from the operator's file (0 = none, so
/// public addresses resolve to `unknown`).
pub fn country_ranges() -> usize {
    LOADED_COUNTRIES.get().copied().unwrap_or(0)
}

/// Resolve an address to a code: a country code, `local` for special-use ranges,
/// or `unknown` when no table entry covers it.
pub fn lookup(ip: IpAddr) -> &'static str {
    let v4 = match ip {
        IpAddr::V4(v4) => v4,
        // only v4-mapped v6 is resolvable with an IPv4 table.
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4,
            None => return "unknown",
        },
    };
    let key = u32::from(v4);
    let Some(table) = TABLE.get() else {
        return "unknown";
    };
    // narrowest covering range wins.
    let mut best: Option<&Range> = None;
    for range in table.iter() {
        if range.start > key {
            break;
        }
        if key <= range.end {
            let better = match best {
                None => true,
                Some(b) => (range.end - range.start) < (b.end - b.start),
            };
            if better {
                best = Some(range);
            }
        }
    }
    match best {
        Some(r) => r.code.as_str(),
        None => "unknown",
    }
}

/// Parse a textual address (as reported by the Clash API) into a code.
pub fn lookup_str(addr: &str) -> &'static str {
    // strip a :port suffix if one slipped in.
    let host = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr);
    match host.trim().parse::<IpAddr>() {
        Ok(ip) => lookup(ip),
        Err(_) => match addr.trim().parse::<IpAddr>() {
            Ok(ip) => lookup(ip),
            Err(_) => "unknown",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_special_use_ranges() {
        init();
        assert_eq!(lookup("10.1.2.3".parse().unwrap()), "local");
        assert_eq!(lookup("192.168.4.5".parse().unwrap()), "local");
        assert_eq!(lookup("127.0.0.1".parse().unwrap()), "local");
        assert_eq!(lookup("100.64.0.1".parse().unwrap()), "local");
    }

    #[test]
    fn public_without_table_is_unknown() {
        init();
        // no country file in tests → public space is honestly "unknown"
        assert_eq!(lookup("8.8.8.8".parse().unwrap()), "unknown");
    }

    #[test]
    fn parses_csv_lines() {
        assert!(parse_line("# comment").is_none());
        assert!(parse_line("").is_none());
        let r = parse_line("1.0.0.0, 1.0.0.255, de").unwrap();
        assert_eq!(r.code, "DE");
        assert_eq!(r.start, u32::from(Ipv4Addr::new(1, 0, 0, 0)));
    }

    #[test]
    fn strips_port_suffix() {
        init();
        assert_eq!(lookup_str("10.0.0.7:51820"), "local");
    }
}
