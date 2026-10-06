//! Replay a real PCAP container with deliberately truncated snapshots. Byte
//! accounting must use the IP lengths, not captured-header lengths or Ethernet.
use global_network_map::{
    geo::{GeoDb, Location},
    model::Model,
    packet,
};
use std::{collections::HashSet, fs, net::IpAddr, time::Instant};

fn frame(source: [u8; 4], destination: [u8; 4], bytes: u16) -> Vec<u8> {
    let mut frame = vec![0; 34];
    frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
    frame[14] = 0x45;
    frame[16..18].copy_from_slice(&bytes.to_be_bytes());
    frame[26..30].copy_from_slice(&source);
    frame[30..34].copy_from_slice(&destination);
    frame
}

#[test]
fn replay_filters_forwarded_packets_and_preserves_ip_byte_totals() {
    let path =
        std::env::temp_dir().join(format!("global-map-pcap-test-{}.pcap", std::process::id()));
    let mut file = vec![];
    file.extend_from_slice(&0xa1b2c3d4u32.to_le_bytes());
    file.extend_from_slice(&2u16.to_le_bytes());
    file.extend_from_slice(&4u16.to_le_bytes());
    for value in [0u32, 0, 128, 1] {
        file.extend_from_slice(&value.to_le_bytes());
    }
    let packets = [
        (frame([8, 8, 8, 8], [10, 0, 0, 2], 1200), 1214u32),
        (frame([10, 0, 0, 2], [8, 8, 8, 8], 600), 614),
        (frame([10, 0, 0, 7], [8, 8, 8, 8], 300), 314), // Not this host.
        (frame([10, 0, 0, 2], [10, 0, 0, 2], 200), 214), // Local self traffic.
    ];
    for (packet, original_length) in packets {
        for value in [1u32, 0, packet.len() as u32, original_length] {
            file.extend_from_slice(&value.to_le_bytes());
        }
        file.extend_from_slice(&packet);
    }
    fs::write(&path, file).unwrap();
    let mut capture = pcap::Capture::from_file(&path).unwrap();
    fs::remove_file(path).unwrap(); // libpcap retains the open file.
    let link = capture.get_datalink().0;
    let local: HashSet<IpAddr> = ["10.0.0.2".parse().unwrap()].into_iter().collect();
    let mut model = Model::default();
    let db = GeoDb::open(None).unwrap();
    let fixture_city = Location {
        city: "Fixture City".into(),
        country: "Fixture Country".into(),
        continent: 2,
        latitude: Some(50.),
        longitude: Some(10.),
    };
    while let Ok(packet) = capture.next_packet() {
        if let Some(traffic) =
            packet::decode(packet.data, link).and_then(|p| packet::classify(p, &local))
        {
            model.ingest_at(traffic, Some(fixture_city.clone()), &db, Instant::now());
        }
    }
    assert_eq!(model.total.total, [1200, 600]);
    assert_eq!(model.endpoints.len(), 1);
    assert_eq!(model.continents[2].total, [1200, 600]);
    assert_eq!(
        model.continents.iter().map(|c| c.total[0]).sum::<u64>(),
        model.total.total[0]
    );
    assert_eq!(
        model.continents.iter().map(|c| c.total[1]).sum::<u64>(),
        model.total.total[1]
    );
}
