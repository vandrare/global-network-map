use crate::{
    geo::{GeoDb, Location},
    packet::Traffic,
};
use std::{
    collections::{HashMap, VecDeque},
    net::IpAddr,
    time::{Duration, Instant},
};

#[derive(Clone, Default, Debug)]
pub struct Counter {
    pub total: [u64; 2],
    pub rate: [f64; 2],
    pending: [u64; 2],
}
impl Counter {
    fn add(&mut self, bytes: [u64; 2]) {
        for (i, b) in bytes.iter().enumerate() {
            self.total[i] += b;
            self.pending[i] += b;
        }
    }
    fn tick(&mut self, seconds: f64) {
        for i in 0..2 {
            self.rate[i] = self.pending[i] as f64 / seconds;
            self.pending[i] = 0;
        }
    }
}
pub struct Endpoint {
    pub location: Location,
    pub counter: Counter,
    pub seen: Instant,
    pub last: [Option<Instant>; 2],
}
pub struct Model {
    pub total: Counter,
    pub continents: [Counter; 9],
    pub endpoints: HashMap<IpAddr, Endpoint>,
    pub history: [VecDeque<u64>; 2],
    pub started: Instant,
    pub dropped: u32,
    pub evicted: u64,
    ticked: Instant,
}
impl Default for Model {
    fn default() -> Self {
        Self::starting_at(Instant::now())
    }
}
impl Model {
    pub fn starting_at(now: Instant) -> Self {
        Self {
            total: Counter::default(),
            continents: Default::default(),
            endpoints: HashMap::new(),
            history: Default::default(),
            started: now,
            dropped: 0,
            evicted: 0,
            ticked: now,
        }
    }
}
impl Model {
    pub fn ingest(&mut self, traffic: Traffic, db: &GeoDb) {
        self.ingest_at(traffic, None, db, Instant::now());
    }
    pub fn ingest_at(
        &mut self,
        traffic: Traffic,
        location: Option<Location>,
        db: &GeoDb,
        now: Instant,
    ) {
        if !self.endpoints.contains_key(&traffic.remote) && self.endpoints.len() >= 8192 {
            // Totals never depend on endpoint retention; removing old nodes does not lose totals.
            if let Some(oldest) = self
                .endpoints
                .iter()
                .min_by_key(|(_, e)| e.seen)
                .map(|(ip, _)| *ip)
            {
                self.endpoints.remove(&oldest);
                self.evicted += 1;
            }
        }
        let endpoint = self
            .endpoints
            .entry(traffic.remote)
            .or_insert_with(|| Endpoint {
                location: location.unwrap_or_else(|| db.lookup(traffic.remote)),
                counter: Counter::default(),
                seen: now,
                last: [None, None],
            });
        endpoint.counter.add(traffic.bytes);
        endpoint.seen = now;
        for i in 0..2 {
            if traffic.bytes[i] > 0 {
                endpoint.last[i] = Some(now);
            }
        }
        self.continents[endpoint.location.continent].add(traffic.bytes);
        self.total.add(traffic.bytes);
    }
    pub fn tick(&mut self) {
        self.tick_at(Instant::now());
    }
    pub fn tick_at(&mut self, now: Instant) {
        let seconds = now.duration_since(self.ticked).as_secs_f64();
        if seconds < 1.0 {
            return;
        }
        self.total.tick(seconds);
        for c in &mut self.continents {
            c.tick(seconds);
        }
        for e in self.endpoints.values_mut() {
            e.counter.tick(seconds);
        }
        for i in 0..2 {
            self.history[i].push_back(self.total.rate[i] as u64);
            if self.history[i].len() > 120 {
                self.history[i].pop_front();
            }
        }
        self.ticked = now;
    }
    pub fn active(&self) -> Vec<(&IpAddr, &Endpoint)> {
        let mut endpoints: Vec<_> = self
            .endpoints
            .iter()
            .filter(|(_, e)| e.seen.elapsed() < Duration::from_secs(30))
            .collect();
        endpoints.sort_by(|a, b| {
            b.1.counter
                .rate
                .iter()
                .sum::<f64>()
                .total_cmp(&a.1.counter.rate.iter().sum::<f64>())
                .then_with(|| a.0.cmp(b.0))
        });
        endpoints
    }
}
pub fn bytes(value: f64) -> String {
    if value >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GiB", value / (1024.0 * 1024.0 * 1024.0))
    } else if value >= 1024.0 * 1024.0 {
        format!("{:.1} MiB", value / (1024.0 * 1024.0))
    } else if value >= 1024.0 {
        format!("{:.1} KiB", value / 1024.0)
    } else {
        format!("{value:.0} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn totals_reconcile_and_idle_rate_reaches_zero() {
        let db = GeoDb::open(None).unwrap();
        let mut m = Model::default();
        let start = m.ticked;
        for (ip, b) in [("10.0.0.1", [100, 40]), ("192.0.2.1", [50, 60])] {
            m.ingest_at(
                Traffic {
                    remote: ip.parse().unwrap(),
                    bytes: b,
                },
                None,
                &db,
                start,
            );
        }
        m.tick_at(start + Duration::from_secs(2));
        assert_eq!(m.total.total, [150, 100]);
        assert_eq!(m.total.rate, [75., 50.]);
        assert_eq!(m.continents.iter().map(|c| c.total[0]).sum::<u64>(), 150);
        m.tick_at(start + Duration::from_secs(3));
        assert_eq!(m.total.rate, [0., 0.]);
        assert_eq!(m.total.total, [150, 100]);
    }
}
