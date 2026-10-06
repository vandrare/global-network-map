use crate::{config::data_dir, geo::GeoDb, packet::Batch};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct Interface {
    pub name: String,
    pub addresses: String,
    pub tunnel: bool,
    pub up: bool,
}
pub fn interfaces() -> Result<Vec<Interface>> {
    let links = Command::new("ip")
        .args(["-j", "-d", "link", "show"])
        .output()
        .ok()
        .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
        .unwrap_or(Value::Null);
    let mut result: Vec<_> = pcap::Device::list()?
        .into_iter()
        .filter(|d| d.name != "any" && d.name != "lo" && !d.addresses.is_empty())
        .map(|d| {
            let info = links
                .as_array()
                .and_then(|links| links.iter().find(|l| l["ifname"].as_str() == Some(&d.name)));
            let kind = info
                .and_then(|l| l.pointer("/linkinfo/info_kind"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let tunnel = ["wireguard", "tun", "tap", "ipip", "sit", "gre"].contains(&kind)
                || ["tun", "tap", "wg", "tailscale", "proton", "nordlynx"]
                    .iter()
                    .any(|s| d.name.starts_with(s))
                || std::path::Path::new("/sys/class/net")
                    .join(&d.name)
                    .join("tun_flags")
                    .exists();
            Interface {
                name: d.name,
                addresses: d
                    .addresses
                    .iter()
                    .map(|a| a.addr.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                tunnel,
                up: d.flags.is_up(),
            }
        })
        .collect();
    result.sort_by_key(|d| (!d.up, !d.tunnel, d.name.clone()));
    Ok(result)
}
pub fn choose_interface(list: &[Interface], requested: Option<&str>) -> Result<Interface> {
    if let Some(name) = requested {
        return list
            .iter()
            .find(|d| d.name == name)
            .cloned()
            .with_context(|| format!("Interface {name:?} is unavailable. Run --list-interfaces."));
    }
    // Ask the kernel how an ordinary Internet destination is routed; this includes VPN policy tables.
    let route = Command::new("ip")
        .args(["-j", "route", "get", "1.1.1.1"])
        .output()
        .ok()
        .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok());
    if let Some(name) = route
        .as_ref()
        .and_then(|v| v.get(0))
        .and_then(|r| r["dev"].as_str())
    {
        if let Some(d) = list.iter().find(|d| d.name == name && d.up) {
            return Ok(d.clone());
        }
    }
    list.iter()
        .find(|d| d.up)
        .cloned()
        .context("No active interface. Connect to a network, or use --demo.")
}

pub struct Capture {
    child: Child,
    pub receiver: mpsc::Receiver<Batch>,
    pub error: Arc<Mutex<String>>,
}
impl Capture {
    pub fn start(interface: &str) -> Result<Self> {
        let helper = std::env::current_exe()?.with_file_name("global-network-map-capture");
        if !helper.is_file() {
            bail!("Missing {}. Build/install both binaries.", helper.display());
        }
        // Authenticate before entering raw mode, so sudo's prompt remains usable.
        eprintln!("Capturing this computer's IP traffic on {interface}. The helper opens the interface, then drops root.");
        if !Command::new("sudo")
            .arg("-v")
            .status()
            .context("sudo is required for live capture")?
            .success()
        {
            bail!("Capture authentication cancelled; use --demo to preview without privileges");
        }
        let mut command = Command::new("sudo");
        command.arg("-n").arg("--").arg(helper).arg(interface);
        Self::spawn(command)
    }
    /// Authentication occurs in the desktop polkit agent, never inside the GUI.
    pub fn start_graphical(interface: &str) -> Result<Self> {
        let helper = std::env::current_exe()?.with_file_name("global-network-map-capture");
        if !helper.is_file() {
            bail!("Missing capture helper: {}", helper.display());
        }
        let mut command = Command::new("pkexec");
        command.arg(helper).arg(interface);
        Self::spawn(command)
    }
    fn spawn(mut command: Command) -> Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let error = Arc::new(Mutex::new(String::new()));
        let errors = error.clone();
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = BufReader::new(stderr).take(16384).read_to_string(&mut s);
            *errors.lock().unwrap() = s;
        });
        let (sender, receiver) = mpsc::sync_channel(64);
        let errors = error.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line.and_then(|line| {
                    serde_json::from_str::<Batch>(&line).map_err(std::io::Error::other)
                }) {
                    Ok(batch) => {
                        if sender.send(batch).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        *errors.lock().unwrap() = format!("Invalid capture stream: {e}");
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            receiver,
            error,
        })
    }
    pub fn exited(&mut self) -> Option<String> {
        self.child.try_wait().ok().flatten().map(|status| {
            let error = self.error.lock().unwrap().clone();
            if error.trim().is_empty() {
                format!("Capture stopped ({status}). Restart to reconnect.")
            } else {
                format!("Capture stopped: {}", error.trim())
            }
        })
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.child.stdin.take();
        // Drain batches while waiting, otherwise a full pipe could block helper shutdown.
        for _ in 0..40 {
            while self.receiver.try_recv().is_ok() {}
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn curl(url: &str, destination: &std::path::Path) -> Result<()> {
    let status = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--proto",
            "=https",
            "--connect-timeout",
            "15",
            "--max-time",
            "240",
            "--retry",
            "2",
            "--output",
        ])
        .arg(destination)
        .arg(url)
        .status()
        .context("Install curl to download databases")?;
    if !status.success() {
        bail!("Download failed for {url}; existing databases were preserved");
    }
    Ok(())
}
pub fn update_db() -> Result<()> {
    let dir = data_dir();
    fs::create_dir_all(&dir)?;
    println!("Downloading DB-IP Lite via sapics/ip-location-db. Attribution: https://db-ip.com (CC BY 4.0).");
    // Hashes attached to release assets refer to the actual uploaded bytes. The
    // repository's separately updated checksum release can lag behind these assets.
    let manifest = dir.join(format!("release.{}.json", std::process::id()));
    let release = (|| -> Result<Value> {
        curl(
            "https://api.github.com/repos/sapics/ip-location-db/releases/tags/latest",
            &manifest,
        )?;
        Ok(serde_json::from_slice(&fs::read(&manifest)?)?)
    })();
    let _ = fs::remove_file(&manifest);
    let release = release?;
    let mut staged: Vec<(PathBuf, PathBuf)> = vec![];
    for version in [4, 6] {
        let name = format!("dbip-city-ipv{version}.mmdb");
        let target = dir.join(&name);
        let temp = dir.join(format!("{name}.{}.part", std::process::id()));
        let checksum = temp.with_extension("sha256");
        let result = (|| -> Result<()> {
            curl(
                &format!(
                    "https://github.com/sapics/ip-location-db/releases/download/latest/{name}"
                ),
                &temp,
            )?;
            let expected = if let Some(digest) = asset_digest(&release, &name) {
                digest
            } else {
                curl(&format!("https://github.com/sapics/ip-location-db/releases/download/checksum/{name}.sha256"),&checksum)?;
                fs::read_to_string(&checksum)?
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase()
            };
            let output = Command::new("sha256sum").arg(&temp).output()?;
            let actual = String::from_utf8(output.stdout)?
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_owned();
            if !output.status.success() || expected.len() != 64 || actual != expected {
                bail!("Checksum mismatch for {name}; retry the update");
            }
            let _ = GeoDb::open(Some(&temp))?;
            Ok(())
        })();
        let _ = fs::remove_file(&checksum);
        if let Err(e) = result {
            let _ = fs::remove_file(&temp);
            for (temp, _) in &staged {
                let _ = fs::remove_file(temp);
            }
            return Err(e);
        }
        staged.push((temp, target));
    }
    for (temp, target) in staged {
        fs::rename(temp, target)?;
    }
    println!(
        "Verified IPv4 and IPv6 databases installed in {}",
        dir.display()
    );
    Ok(())
}
fn asset_digest(release: &Value, name: &str) -> Option<String> {
    let digest = release
        .get("assets")?
        .as_array()?
        .iter()
        .find(|asset| asset["name"].as_str() == Some(name))?
        .get("digest")?
        .as_str()?
        .strip_prefix("sha256:")?;
    (digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| digest.to_ascii_lowercase())
}
pub fn suggest_origin(db: &GeoDb) -> Result<()> {
    eprintln!("Contacting api.ipify.org to learn your public exit IP. With a VPN this suggests its exit city, not your physical location.");
    let (ip, location) = public_location(db)?;
    println!(
        "Public exit IP: {ip}\nSuggested location: {}, {}",
        location.city, location.country
    );
    if let Some((lon, lat)) = location.coordinates() {
        println!("Coordinates: {lat}, {lon}\nUse --setup to choose your actual city; this suggestion has not changed your settings.");
    } else {
        println!("City unavailable. Run --update-db if the database is missing, or select your city with --setup.");
    }
    Ok(())
}
pub fn public_location(db: &GeoDb) -> Result<(std::net::IpAddr, crate::geo::Location)> {
    let output = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--connect-timeout",
            "10",
            "--max-time",
            "20",
            "https://api.ipify.org",
        ])
        .output()?;
    if !output.status.success() {
        bail!(
            "Public IP lookup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let ip = String::from_utf8(output.stdout)?.trim().parse()?;
    let location = db.lookup(ip);
    Ok((ip, location))
}
