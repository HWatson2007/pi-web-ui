use std::{net::IpAddr, path::PathBuf};

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Debug, Clone, Parser)]
#[command(author, version, about)]
pub struct Config {
    /// Address to listen on. Keep loopback-only behind Caddy/Tailscale.
    #[arg(long, env = "PI_MOBILE_HOST", default_value = "127.0.0.1")]
    pub host: IpAddr,

    /// HTTP/WebSocket port.
    #[arg(long, env = "PI_MOBILE_PORT", default_value_t = 3003)]
    pub port: u16,

    /// Working directory for newly created Pi sessions.
    #[arg(long, env = "PI_MOBILE_CWD")]
    pub cwd: Option<PathBuf>,

    /// Pi executable. Use an absolute path when running from systemd.
    #[arg(long, env = "PI_MOBILE_PI", default_value = "pi")]
    pub pi_binary: PathBuf,

    /// Pi agent data directory. Defaults to PI_CODING_AGENT_DIR or ~/.pi/agent.
    #[arg(long, env = "PI_CODING_AGENT_DIR")]
    pub agent_dir: Option<PathBuf>,

    /// Maximum number of live Pi RPC processes.
    #[arg(long, env = "PI_MOBILE_MAX_SESSIONS", default_value_t = 8)]
    pub max_sessions: usize,
}

impl Config {
    pub fn resolve(mut self) -> Result<Self> {
        if self.cwd.is_none() {
            self.cwd = Some(std::env::current_dir().context("cannot resolve current directory")?);
        }
        if self.agent_dir.is_none() {
            let home = dirs::home_dir().context("cannot resolve home directory")?;
            self.agent_dir = Some(home.join(".pi/agent"));
        }
        Ok(self)
    }

    pub fn cwd(&self) -> &PathBuf {
        self.cwd.as_ref().expect("config is resolved")
    }

    pub fn agent_dir(&self) -> &PathBuf {
        self.agent_dir.as_ref().expect("config is resolved")
    }
}
