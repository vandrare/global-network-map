//! Header-only decoding. Totals are IP-layer bytes, including retransmissions.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Traffic {
    pub remote: IpAddr,
    pub bytes: [u64; 2],
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Batch {
    pub traffic: Vec<Traffic>,
    pub dropped: u32,
    pub interface: String,
    pub addresses: Vec<IpAddr>,
}
#[derive(Debug)]
pub struct IpPacket {
    pub source: IpAddr,
    pub destination: IpAddr,
    pub bytes: u64,
}

fn be16(d: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        d.get(offset..offset + 2)?.try_into().ok()?,
    ))
}
pub fn decode(data: &[u8], link: i32) -> Option<IpPacket> {
    let (mut offset, mut protocol) = match link {
        1 => (14, be16(data, 12)?),   // Ethernet
        113 => (16, be16(data, 14)?), // Linux cooked v1
        276 => (20, be16(data, 0)?),  // Linux cooked v2
        12 | 101 => (
            0,
            if data.first()? >> 4 == 4 {
                0x0800
            } else {
                0x86dd
            },
        ), // RAW
        228 => (0, 0x0800),
        229 => (0, 0x86dd),
        0 | 108 => (
            4,
            if data.get(4)? >> 4 == 4 {
                0x0800
            } else {
                0x86dd
            },
        ),
        _ => return None,
    };
    for _ in 0..2 {
        if protocol != 0x8100 && protocol != 0x88a8 {
            break;
        }
        protocol = be16(data, offset + 2)?;
        offset += 4;
    }
    let d = data.get(offset..)?;
    match protocol {
        0x0800 => {
            if d.len() < 20 || d[0] >> 4 != 4 {
                return None;
            }
            let header = usize::from(d[0] & 15) * 4;
            let bytes = be16(d, 2)? as u64;
            if header < 20 || d.len() < header || bytes < header as u64 {
                return None;
            }
            Some(IpPacket {
                source: Ipv4Addr::new(d[12], d[13], d[14], d[15]).into(),
                destination: Ipv4Addr::new(d[16], d[17], d[18], d[19]).into(),
                bytes,
            })
        }
        0x86dd => {
            if d.len() < 40 || d[0] >> 4 != 6 {
                return None;
            }
            let source = Ipv6Addr::from(<[u8; 16]>::try_from(&d[8..24]).ok()?);
            let destination = Ipv6Addr::from(<[u8; 16]>::try_from(&d[24..40]).ok()?);
            let bytes = be16(d, 4)? as u64 + 40;
            Some(IpPacket {
                source: source.into(),
                destination: destination.into(),
                bytes,
            })
        }
        _ => None,
    }
}
pub fn classify(packet: IpPacket, local: &HashSet<IpAddr>) -> Option<Traffic> {
    let source = local.contains(&packet.source);
    let destination = local.contains(&packet.destination);
    match (source, destination) {
        (true, false) => Some(Traffic {
            remote: packet.destination,
            bytes: [0, packet.bytes],
        }),
        (false, true) => Some(Traffic {
            remote: packet.source,
            bytes: [packet.bytes, 0],
        }),
        _ => None, // Neither forwarded traffic nor host-to-host loopback is counted.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ipv4() -> Vec<u8> {
        let mut p = vec![0u8; 20];
        p[0] = 0x45;
        p[2..4].copy_from_slice(&1500u16.to_be_bytes());
        p[12..16].copy_from_slice(&[10, 0, 0, 2]);
        p[16..20].copy_from_slice(&[8, 8, 8, 8]);
        p
    }
    #[test]
    fn ethernet_vlan_raw_and_cooked() {
        let p = ipv4();
        let mut ethernet = vec![0; 14];
        ethernet[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        ethernet.extend(&p);
        let mut vlan = vec![0; 18];
        vlan[12..14].copy_from_slice(&0x8100u16.to_be_bytes());
        vlan[16..18].copy_from_slice(&0x0800u16.to_be_bytes());
        vlan.extend(&p);
        let mut cooked = vec![0; 16];
        cooked[14..16].copy_from_slice(&0x0800u16.to_be_bytes());
        cooked.extend(&p);
        for (data, link) in [(p, 12), (ethernet, 1), (vlan, 1), (cooked, 113)] {
            let packet = decode(&data, link).unwrap();
            let local = ["10.0.0.2".parse().unwrap()].into_iter().collect();
            let t = classify(packet, &local).unwrap();
            assert_eq!(t.bytes, [0, 1500]);
        }
    }
    #[test]
    fn ipv6_and_direction() {
        let mut p = vec![0; 40];
        p[0] = 0x60;
        p[4..6].copy_from_slice(&1000u16.to_be_bytes());
        p[8..24].copy_from_slice(&"2606:4700:4700::1111".parse::<Ipv6Addr>().unwrap().octets());
        p[24..40].copy_from_slice(&"fd00::2".parse::<Ipv6Addr>().unwrap().octets());
        let local = ["fd00::2".parse().unwrap()].into_iter().collect();
        assert_eq!(
            classify(decode(&p, 12).unwrap(), &local).unwrap().bytes,
            [1040, 0]
        );
        assert!(classify(decode(&p, 12).unwrap(), &HashSet::new()).is_none());
    }
    #[test]
    fn truncated_and_non_ip_frames_are_safe() {
        for len in 0..20 {
            assert!(decode(&ipv4()[..len], 12).is_none());
        }
        assert!(decode(&[0; 100], 1).is_none());
        let mut p = ipv4();
        p[0] = 0x4f;
        assert!(decode(&p, 12).is_none());
    }
}
