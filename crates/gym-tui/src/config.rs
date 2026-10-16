use anyhow::{Context, Result};
use serde::Deserialize;
use std::{env, fs, path::PathBuf};
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub database: PathBuf,
    pub server_url: Option<String>,
    pub token_file: Option<PathBuf>,
    pub timezone: String,
    pub sync_interval_secs: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            database: xdg("XDG_DATA_HOME", ".local/share").join("larpgym/client.sqlite"),
            server_url: None,
            token_file: None,
            timezone: "America/Mazatlan".into(),
            sync_interval_secs: 60,
        }
    }
}
pub fn xdg(variable: &str, fallback: &str) -> PathBuf {
    env::var_os(variable)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(fallback))
}
pub fn load() -> Result<(Config, bool, Option<PathBuf>)> {
    let mut args = env::args().skip(1);
    let mut path = xdg("XDG_CONFIG_HOME", ".config").join("larpgym/client.toml");
    let mut db = None;
    let mut offline = false;
    let mut backup = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => path = args.next().context("Falta ruta para --config")?.into(),
            "--db" => db = Some(PathBuf::from(args.next().context("Falta ruta para --db")?)),
            "--offline" => offline = true,
            "--backup" => {
                backup = Some(PathBuf::from(
                    args.next().context("Falta destino para --backup")?,
                ))
            }
            "--help" | "-h" => {
                println!("LarpGym · Registro local de gimnasio\nUso: gym-tui [--config ARCHIVO] [--db RUTA] [--offline] [--backup DESTINO]\nConfiguración: $XDG_CONFIG_HOME/larpgym/client.toml\nDatos: $XDG_DATA_HOME/larpgym/client.sqlite\nTeclas: h/j/k/l navegar, gg/G inicio/final, Ctrl+d/u media página, gt/gT paneles, D fecha, ? ayuda, q salir");
                std::process::exit(0);
            }
            _ => anyhow::bail!("Opción desconocida: {arg}; usa --help"),
        }
    }
    let mut cfg: Config = if path.exists() {
        toml::from_str(&fs::read_to_string(&path)?).context("Configuración de cliente inválida")?
    } else {
        Config::default()
    };
    if let Some(p) = env::var_os("GYM_CLIENT_DB") {
        cfg.database = p.into();
    }
    if let Some(p) = db {
        cfg.database = p;
    }
    if let Ok(url) = env::var("GYM_SERVER_URL") {
        cfg.server_url = Some(url);
    }
    if let Some(p) = env::var_os("GYM_TOKEN_FILE") {
        cfg.token_file = Some(p.into());
    }
    anyhow::ensure!(
        (15..=3600).contains(&cfg.sync_interval_secs),
        "sync_interval_secs debe estar entre 15 y 3600"
    );
    Ok((cfg, offline, backup))
}
