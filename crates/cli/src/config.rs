use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};

#[derive(Debug, Args)]
pub struct ConfigCmd {
    #[command(subcommand)]
    action: ConfigAction,
}

#[derive(Debug, Subcommand)]
enum ConfigAction {
    /// Set a desktop/CLI classification preference.
    Set(ConfigSet),
}

#[derive(Debug, Args)]
struct ConfigSet {
    #[command(subcommand)]
    setting: ConfigSetting,
}

#[derive(Debug, Subcommand)]
enum ConfigSetting {
    /// Set the OpenRouter API key (prompt securely, or read from stdin).
    OpenrouterApiKey {
        #[arg(long)]
        stdin: bool,
    },
    /// Set the Jev model ID.
    ClassificationModel { model: String },
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("IBEX_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("ibex")
        })
}

pub fn load() -> Result<toml::Value> {
    let path = config_dir().join("config.toml");
    match fs::read_to_string(&path) {
        Ok(raw) => toml::from_str(&raw).with_context(|| format!("parse {}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(toml::Value::Table(Default::default()))
        }
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

fn save(config: &toml::Value) -> Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    let staged = dir.join("config.toml.tmp");
    match fs::remove_file(&staged) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&staged)?;
    file.write_all(toml::to_string(config)?.as_bytes())?;
    file.sync_all()?;
    fs::rename(&staged, dir.join("config.toml"))?;
    Ok(())
}

pub fn run(cmd: ConfigCmd) -> Result<()> {
    let mut config = load()?;
    let table = config
        .as_table_mut()
        .context("config.toml must contain a table")?;
    match cmd.action {
        ConfigAction::Set(ConfigSet {
            setting: ConfigSetting::OpenrouterApiKey { stdin },
        }) => {
            let value = if stdin {
                let mut text = String::new();
                io::stdin().read_to_string(&mut text)?;
                text
            } else {
                dialoguer::Password::new()
                    .with_prompt("OpenRouter API key")
                    .allow_empty_password(true)
                    .interact()?
            };
            let key = value.trim();
            if key.chars().any(char::is_whitespace) {
                bail!("API keys cannot contain spaces");
            }
            if key.is_empty() {
                table.remove("openrouter_api_key");
            } else {
                table.insert(
                    "openrouter_api_key".into(),
                    toml::Value::String(key.to_owned()),
                );
            }
            save(&config)?;
            println!("OpenRouter API key saved");
        }
        ConfigAction::Set(ConfigSet {
            setting: ConfigSetting::ClassificationModel { model },
        }) => {
            let model = model.trim();
            if model.chars().any(char::is_whitespace) {
                bail!("model ID cannot contain spaces");
            }
            if model.is_empty() {
                table.remove("jev_model");
            } else {
                table.insert("jev_model".into(), toml::Value::String(model.to_owned()));
            }
            save(&config)?;
            println!("classification model saved");
        }
    }
    Ok(())
}
