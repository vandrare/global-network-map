use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{env, fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Origin {
    pub city: String,
    pub country: String,
    pub latitude: f64,
    pub longitude: f64,
}

impl Origin {
    pub fn validate(&self) -> Result<()> {
        if self.city.trim().is_empty() || self.country.trim().is_empty() {
            bail!("City and country are required");
        }
        if !self.latitude.is_finite()
            || !self.longitude.is_finite()
            || !(-90.0..=90.0).contains(&self.latitude)
            || !(-180.0..=180.0).contains(&self.longitude)
        {
            bail!("Latitude must be -90..90 and longitude -180..180");
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub origin: Option<Origin>,
    pub interface: Option<String>,
    pub database: Option<PathBuf>,
}

pub fn home() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
fn xdg(key: &str, fallback: &str) -> PathBuf {
    env::var_os(key)
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(fallback))
}
pub fn config_path() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("global-network-map/config.toml")
}
pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("global-network-map")
}
pub fn theme_paths() -> Vec<PathBuf> {
    vec![
        xdg("XDG_STATE_HOME", ".local/state").join("omarchy/current/theme/colors.toml"),
        xdg("XDG_CONFIG_HOME", ".config").join("omarchy/current/theme/colors.toml"),
    ]
}
impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let config: Self = toml::from_str(&fs::read_to_string(&path)?)
            .with_context(|| format!("Invalid configuration: {}", path.display()))?;
        if let Some(o) = &config.origin {
            o.validate()?;
        }
        Ok(config)
    }
    pub fn save(&self) -> Result<()> {
        let path = config_path();
        fs::create_dir_all(path.parent().unwrap())?;
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temporary, toml::to_string_pretty(self)?)?;
        fs::rename(temporary, path)?;
        Ok(())
    }
}
