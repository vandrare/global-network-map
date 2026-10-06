use crate::{
    config::Origin,
    geo::{GeoDb, Location},
    model::Model,
    packet::Traffic,
};
use std::time::Instant;
pub fn origin() -> Origin {
    Origin {
        city: "Phoenix".into(),
        country: "United States".into(),
        latitude: 33.4484,
        longitude: -112.074,
    }
}
pub fn step(model: &mut Model, db: &GeoDb, step: u64) {
    step_at(model, db, step, Instant::now());
}
pub fn step_at(model: &mut Model, db: &GeoDb, step: u64, now: Instant) {
    let cities = [
        ("New York", "US", 40.71, -74.01, 0),
        ("London", "GB", 51.51, -0.13, 2),
        ("Frankfurt", "DE", 50.11, 8.68, 2),
        ("Tokyo", "JP", 35.68, 139.69, 3),
        ("Singapore", "SG", 1.35, 103.82, 3),
        ("Sydney", "AU", -33.87, 151.21, 5),
        ("São Paulo", "BR", -23.55, -46.63, 1),
        ("Cape Town", "ZA", -33.92, 18.42, 4),
        ("Seattle", "US", 47.61, -122.33, 0),
        ("Mumbai", "IN", 19.08, 72.88, 3),
        ("Paris", "FR", 48.86, 2.35, 2),
        ("Toronto", "CA", 43.65, -79.38, 0),
    ];
    for (i, (city, code, latitude, longitude, continent)) in cities.iter().enumerate() {
        let wave = ((step as f64 * 0.12 + i as f64 * 1.8).sin() + 1.1) * 0.5;
        let down = (wave * (180000. / (i + 1) as f64)) as u64 + 100;
        let up =
            (((step as f64 * 0.16 + i as f64).cos() + 1.1) * 17000. / (i + 1) as f64) as u64 + 64;
        model.ingest_at(
            Traffic {
                remote: std::net::Ipv4Addr::new(198, 51, 100, i as u8 + 1).into(),
                bytes: [down, up],
            },
            Some(Location {
                city: (*city).into(),
                country: crate::geo::country(code).0,
                latitude: Some(*latitude),
                longitude: Some(*longitude),
                continent: *continent,
            }),
            db,
            now,
        );
    }
}
