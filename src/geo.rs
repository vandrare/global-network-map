use crate::config::{data_dir, Origin};
use anyhow::{Context, Result};
use maxminddb::Reader;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{net::IpAddr, path::Path};

pub const CONTINENTS: [&str; 9] = [
    "N. America",
    "S. America",
    "Europe",
    "Asia",
    "Africa",
    "Oceania",
    "Antarctica",
    "Local",
    "Unknown",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Location {
    pub city: String,
    pub country: String,
    pub continent: usize,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}
impl Location {
    pub fn unknown(local: bool) -> Self {
        Self {
            city: if local {
                "Local network"
            } else {
                "Unknown city"
            }
            .into(),
            country: if local {
                "Private / special address"
            } else {
                "Unknown country"
            }
            .into(),
            continent: if local { 7 } else { 8 },
            latitude: None,
            longitude: None,
        }
    }
    pub fn coordinates(&self) -> Option<(f64, f64)> {
        Some((self.longitude?, self.latitude?))
    }
}

pub fn country(code: &str) -> (String, usize) {
    for row in include_str!("../assets/countries.tsv").lines() {
        let f: Vec<_> = row.split('\t').collect();
        if f[0].eq_ignore_ascii_case(code) {
            let continent = match f[2] {
                "NA" => 0,
                "SA" => 1,
                "EU" => 2,
                "AS" => 3,
                "AF" => 4,
                "OC" => 5,
                "AN" => 6,
                _ => 8,
            };
            return (f[1].into(), continent);
        }
    }
    (code.into(), 8)
}

pub fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => {
            let o = a.octets();
            a.is_private()
                || a.is_loopback()
                || a.is_link_local()
                || a.is_multicast()
                || a.is_unspecified()
                || a.is_broadcast()
                || a.is_documentation()
                || o[0] == 0
                || o[0] >= 240
                || (o[0] == 100 && (64..128).contains(&o[1]))
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
        }
        IpAddr::V6(a) => {
            if let Some(v4) = a.to_ipv4_mapped() {
                return is_local(v4.into());
            }
            a.is_loopback()
                || a.is_unspecified()
                || a.is_multicast()
                || a.is_unique_local()
                || a.is_unicast_link_local()
                || (a.segments()[0] == 0x2001 && a.segments()[1] == 0xdb8)
        }
    }
}

pub struct GeoDb {
    readers: Vec<Reader<Vec<u8>>>,
    pub status: String,
}
impl GeoDb {
    pub fn open(path: Option<&Path>) -> Result<Self> {
        if let Some(path) = path {
            return Ok(Self {
                readers: vec![Reader::open_readfile(path)
                    .with_context(|| format!("Cannot read {}", path.display()))?],
                status: "Local MMDB".into(),
            });
        }
        let mut readers = vec![];
        for version in [4, 6] {
            let path = data_dir().join(format!("dbip-city-ipv{version}.mmdb"));
            if path.exists() {
                readers.push(
                    Reader::open_readfile(&path)
                        .with_context(|| format!("Invalid {}", path.display()))?,
                );
            }
        }
        let status = match readers.len() {
            0 => "No database · run --update-db",
            1 => "DB-IP Lite · partial database",
            _ => "DB-IP Lite · offline IPv4 + IPv6",
        }
        .into();
        Ok(Self { readers, status })
    }
    pub fn lookup(&self, ip: IpAddr) -> Location {
        if is_local(ip) {
            return Location::unknown(true);
        }
        for reader in &self.readers {
            if let Ok(result) = reader.lookup(ip) {
                if let Ok(Some(value)) = result.decode::<Value>() {
                    return decode_location(&value);
                }
            }
        }
        Location::unknown(false)
    }
}

fn decode_location(v: &Value) -> Location {
    // sapics' flat records and standard GeoIP2 records are both supported.
    let code = v
        .get("country_code")
        .and_then(Value::as_str)
        .or_else(|| v.pointer("/country/iso_code").and_then(Value::as_str))
        .unwrap_or("");
    let (mut name, continent) = country(code);
    if let Some(n) = v.pointer("/country/names/en").and_then(Value::as_str) {
        name = n.into();
    }
    if name.is_empty() {
        name = "Unknown country".into();
    }
    let city = v
        .get("city")
        .and_then(Value::as_str)
        .or_else(|| v.pointer("/city/names/en").and_then(Value::as_str))
        .filter(|c| !c.is_empty())
        .unwrap_or("Unknown city")
        .to_owned();
    let number = |key: &str| {
        v.get(key)
            .or_else(|| v.get("location").and_then(|l| l.get(key)))
            .and_then(|n| {
                n.as_f64()
                    .or_else(|| n.as_str().and_then(|s| s.parse().ok()))
            })
    };
    let mut latitude = number("latitude").filter(|n| n.is_finite() && (-90.0..=90.0).contains(n));
    let mut longitude =
        number("longitude").filter(|n| n.is_finite() && (-180.0..=180.0).contains(n));
    if city == "Unknown city" {
        latitude = None;
        longitude = None;
    }
    Location {
        city,
        country: name,
        continent,
        latitude,
        longitude,
    }
}

pub struct City {
    pub origin: Origin,
    pub code: String,
    pub region: String,
    ascii: String,
}
pub fn cities() -> Vec<City> {
    // Country metadata is tiny, but build it once rather than scanning it for every city.
    let countries: std::collections::HashMap<_, _> = include_str!("../assets/countries.tsv")
        .lines()
        .map(|l| {
            let mut f = l.split('\t');
            (f.next().unwrap(), f.next().unwrap())
        })
        .collect();
    include_str!("../assets/cities.tsv")
        .lines()
        .filter_map(|line| {
            let f: Vec<_> = line.split('\t').collect();
            Some(City {
                origin: Origin {
                    city: f[0].into(),
                    country: countries.get(f[2]).unwrap_or(&f[2]).to_string(),
                    latitude: f[3].parse().ok()?,
                    longitude: f[4].parse().ok()?,
                },
                code: f[2].into(),
                region: f.get(5).unwrap_or(&"").to_string(),
                ascii: f[1].to_lowercase(),
            })
        })
        .collect()
}
pub fn search_cities(cities: &[City], query: &str) -> Vec<usize> {
    let q = query.to_lowercase();
    let terms: Vec<_> = q.split([',', ' ']).filter(|s| !s.is_empty()).collect();
    if terms.is_empty() {
        return vec![];
    }
    let mut matches: Vec<_> = cities
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            let hay = format!(
                "{} {} {} {} {}",
                c.ascii,
                c.origin.city.to_lowercase(),
                c.code.to_lowercase(),
                c.origin.country.to_lowercase(),
                c.region.to_lowercase()
            );
            terms.iter().all(|t| hay.contains(t))
        })
        .map(|(i, _)| i)
        .collect();
    matches.sort_by_key(|i| !cities[*i].ascii.starts_with(terms[0]));
    matches.truncate(20);
    matches
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires --update-db to have installed the real city databases"]
    fn downloaded_databases_locate_ipv4_and_ipv6() {
        let db = GeoDb::open(None).unwrap();
        assert_eq!(db.readers.len(), 2);
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
        ] {
            let location = db.lookup(ip.parse().unwrap());
            assert!(location.continent < 7, "{ip}: {location:?}");
            assert!(location.coordinates().is_some(), "{ip}: {location:?}");
            println!("{ip}: {}, {}", location.city, location.country);
        }
    }
    #[test]
    fn flat_and_standard_records() {
        let flat = serde_json::json!({"country_code":"JP","city":"Tokyo","latitude":"35.68","longitude":"139.69"});
        let l = decode_location(&flat);
        assert_eq!(l.continent, 3);
        assert_eq!(l.coordinates(), Some((139.69, 35.68)));
        let standard = serde_json::json!({"country":{"iso_code":"DE"},"city":{"names":{"en":"Berlin"}},"location":{"latitude":52.5,"longitude":13.4}});
        assert_eq!(decode_location(&standard).country, "Germany");
        assert!(decode_location(
            &serde_json::json!({"country_code":"US","latitude":0,"longitude":0})
        )
        .coordinates()
        .is_none());
    }
    #[test]
    fn private_special_and_public() {
        for ip in [
            "10.0.0.1",
            "100.64.0.1",
            "::1",
            "fe80::1",
            "fd00::1",
            "224.0.0.1",
            "::ffff:192.168.1.1",
        ] {
            assert!(is_local(ip.parse().unwrap()), "{ip}");
        }
        assert!(!is_local("8.8.8.8".parse().unwrap()));
    }
    #[test]
    fn city_search_is_offline() {
        let cities = cities();
        let matches = search_cities(&cities, "Phoenix, US");
        assert_eq!(cities[matches[0]].origin.city, "Phoenix");
    }
}
