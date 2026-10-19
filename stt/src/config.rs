// Settings, read from the environment (the systemd unit loads them from
// ~/.config/ears-stt/env).

use anyhow::{bail, Result};
use std::net::IpAddr;
use std::path::PathBuf;

pub struct Config {
    pub host: String,
    pub port: u16,
    pub model: String,
    pub models_dir: PathBuf,
    pub use_gpu: bool,
    pub vad: bool,
    pub api_key: String,
}

fn var(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn flag(name: &str, default: bool) -> bool {
    match std::env::var(name).as_deref() {
        Ok("0" | "false" | "no" | "off") => false,
        Ok("1" | "true" | "yes" | "on") => true,
        _ => default,
    }
}

impl Config {
    pub fn from_env() -> Result<Config> {
        let data_dir = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("."));
        let cfg = Config {
            host: var("EARS_STT_HOST", "127.0.0.1"),
            port: var("EARS_STT_PORT", "13278").parse()?,
            model: var("EARS_STT_MODEL", "whisper-medium.en"),
            models_dir: std::env::var_os("EARS_STT_MODELS_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| data_dir.join("ears-stt/models")),
            use_gpu: flag("EARS_STT_GPU", true),
            vad: flag("EARS_STT_VAD", true),
            api_key: std::env::var("EARS_STT_API_KEY").unwrap_or_default(),
        };
        cfg.check()?;
        Ok(cfg)
    }

    /// Refuse configurations that would expose the server without a key.
    pub fn check(&self) -> Result<()> {
        if !crate::models::is_known(&self.model) {
            bail!("unknown EARS_STT_MODEL '{}'", self.model);
        }
        if !is_loopback(&self.host) && self.api_key.is_empty() {
            bail!(
                "refusing to listen on {} without EARS_STT_API_KEY; set a key or bind to 127.0.0.1",
                self.host
            );
        }
        Ok(())
    }
}

pub fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(host: &str, key: &str) -> Config {
        Config {
            host: host.into(),
            port: 13278,
            model: "whisper-small.en".into(),
            models_dir: PathBuf::new(),
            use_gpu: false,
            vad: false,
            api_key: key.into(),
        }
    }

    #[test]
    fn exposed_host_needs_key() {
        assert!(cfg("127.0.0.1", "").check().is_ok());
        assert!(cfg("::1", "").check().is_ok());
        assert!(cfg("localhost", "").check().is_ok());
        assert!(cfg("0.0.0.0", "").check().is_err());
        assert!(cfg("192.168.1.5", "").check().is_err());
        assert!(cfg("0.0.0.0", "k").check().is_ok());
    }
}
