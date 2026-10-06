use anyhow::{bail, Context, Result};
use global_network_map::packet::{self, Batch, Traffic};
use std::{
    collections::{HashMap, HashSet},
    io::{self, Read, Write},
    net::IpAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

fn drop_privileges() -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        return Ok(());
    }
    let uid: u32 = std::env::var("PKEXEC_UID")
        .or_else(|_| std::env::var("SUDO_UID"))
        .context("Launch this helper through sudo or pkexec as a normal user")?
        .parse()?;
    // Resolve the account's primary group for both sudo and polkit launches.
    // This happens before any threads are created, so getpwuid's static buffer is safe here.
    let account = unsafe { libc::getpwuid(uid) };
    if account.is_null() {
        bail!("Cannot resolve invoking user's account");
    }
    let gid = unsafe { (*account).pw_gid };
    if uid == 0 {
        bail!("The capture helper requires a non-root invoking user");
    }
    // Open the capture socket first, then permanently discard root and supplementary groups.
    if unsafe { libc::setgroups(0, std::ptr::null()) } != 0
        || unsafe { libc::setgid(gid) } != 0
        || unsafe { libc::setuid(uid) } != 0
    {
        bail!(
            "Could not drop capture privileges: {}",
            io::Error::last_os_error()
        );
    }
    if unsafe { libc::geteuid() } != uid {
        bail!("Privilege drop failed");
    }
    Ok(())
}

fn run() -> Result<()> {
    let interface = std::env::args()
        .nth(1)
        .context("Usage: global-network-map-capture INTERFACE (internal helper)")?;
    if interface == "any" || interface == "lo" {
        bail!("Select one non-loopback interface");
    }
    let device = pcap::Device::list()?
        .into_iter()
        .find(|d| d.name == interface)
        .context("Interface not found")?;
    let mut local: HashSet<IpAddr> = device.addresses.iter().map(|a| a.addr).collect();
    if local.is_empty() {
        bail!("The selected interface has no IP addresses");
    }
    let cap = pcap::Capture::from_device(device)?
        .promisc(false)
        .snaplen(128)
        .timeout(100)
        .buffer_size(4 * 1024 * 1024)
        .immediate_mode(true)
        .open()
        .context("Cannot open capture interface")?;
    let mut cap = cap.setnonblock()?;
    let link = cap.get_datalink().0;
    if ![1, 113, 276, 12, 101, 228, 229, 0, 108].contains(&link) {
        bail!("Unsupported interface link type {link}");
    }
    drop_privileges()?;
    unsafe {
        libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
    }
    let alive = Arc::new(AtomicBool::new(true));
    let watching = alive.clone();
    // The parent holds stdin open; closing it stops capture even when there is no traffic.
    std::thread::spawn(move || {
        let mut b = [0; 1];
        let _ = io::stdin().read(&mut b);
        watching.store(false, Ordering::Relaxed);
    });
    let mut pending: HashMap<IpAddr, [u64; 2]> = HashMap::new();
    let mut flush = Instant::now() - Duration::from_secs(1);
    let mut refresh = Instant::now();
    let mut out = io::BufWriter::new(io::stdout().lock());
    while alive.load(Ordering::Relaxed) {
        // A bounded batch prevents busy interfaces from starving shutdown and stats.
        for _ in 0..4096 {
            match cap.next_packet() {
                Ok(packet) => {
                    if let Some(t) =
                        packet::decode(packet.data, link).and_then(|p| packet::classify(p, &local))
                    {
                        let value = pending.entry(t.remote).or_default();
                        value[0] += t.bytes[0];
                        value[1] += t.bytes[1];
                    }
                }
                Err(pcap::Error::TimeoutExpired) => break,
                Err(e) => return Err(e.into()),
            }
        }
        if flush.elapsed() >= Duration::from_millis(200) || pending.len() >= 8192 {
            let batch = Batch {
                traffic: pending
                    .drain()
                    .map(|(remote, bytes)| Traffic { remote, bytes })
                    .collect(),
                dropped: cap
                    .stats()
                    .map(|s| s.dropped.saturating_add(s.if_dropped))
                    .unwrap_or(0),
                interface: interface.clone(),
                addresses: local.iter().copied().collect(),
            };
            serde_json::to_writer(&mut out, &batch)?;
            writeln!(out)?;
            out.flush()?;
            flush = Instant::now();
        }
        if refresh.elapsed() >= Duration::from_secs(2) {
            let devices = pcap::Device::list()?;
            let device = devices
                .into_iter()
                .find(|d| d.name == interface)
                .context("Interface disappeared; restart and select the current interface")?;
            local = device.addresses.iter().map(|a| a.addr).collect();
            refresh = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
