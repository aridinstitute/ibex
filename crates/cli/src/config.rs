use std::io::{self, Read};

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use ibex_app::config::{ConfigFile, Table};

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

/// Store `value` under `key`, or remove the key when `value` is blank. Every
/// other key, including the desktop app's, is kept as written.
fn set_or_clear(config: &ConfigFile, key: &str, value: &str) -> Result<()> {
    config.update(|table: &mut Table| {
        if value.is_empty() {
            table.remove(key);
        } else {
            table.insert(key.to_owned(), toml::Value::String(value.to_owned()));
        }
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(())
}

pub fn run(cmd: ConfigCmd, config: &ConfigFile) -> Result<()> {
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
            set_or_clear(config, "openrouter_api_key", key)?;
            println!("OpenRouter API key saved");
        }
        ConfigAction::Set(ConfigSet {
            setting: ConfigSetting::ClassificationModel { model },
        }) => {
            let model = model.trim();
            if model.chars().any(char::is_whitespace) {
                bail!("model ID cannot contain spaces");
            }
            set_or_clear(config, "jev_model", model)?;
            println!("classification model saved");
        }
    }
    Ok(())
}
