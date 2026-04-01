//! Application configuration and secrets management.
//!
//! Non-secret settings are persisted to a JSON file in the OS config directory.
//! Secrets are stored in the OS keyring.
//! Environment variables override both stores when present.

use std::{collections::HashMap, env, error::Error, fs, io};

fn config_path() -> Result<std::path::PathBuf, Box<dyn Error>> {
    Ok(dirs::config_dir()
        .ok_or("could not find config directory")?
        .join("mood")
        .join("config.json"))
}

fn prompt(label: &str) -> Result<String, Box<dyn Error>> {
    print!("{label}: ");
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

fn prompt_with_status(label: &str, is_set: bool) -> Result<Option<String>, Box<dyn Error>> {
    if is_set {
        print!("{label} [set, press Enter to keep]: ");
    } else {
        print!("{label} [not set]: ");
    }
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let value = input.trim().to_string();
    Ok(if value.is_empty() { None } else { Some(value) })
}

struct ConfigEntry {
    label: &'static str,
    value: String,
}

/// Non-secret application configuration loaded from the OS config directory.
///
/// Missing values are prompted for interactively on first run and saved for subsequent runs.
/// Any key can be overridden at runtime by setting the corresponding environment variable.
pub struct AppConfig {
    entries: HashMap<&'static str, ConfigEntry>,
    dirty: bool,
}

impl AppConfig {
    fn new() -> Self {
        AppConfig {
            entries: [
                ("BRIDGE_ADDRESS", "Bridge IP address"),
                ("BRIDGE_PORT", "Bridge port"),
                ("APP_ID", "App ID"),
                ("ENTERTAINMENT_CONFIG_ID", "Entertainment config ID"),
            ]
            .into_iter()
            .map(|(key, label)| {
                (
                    key,
                    ConfigEntry {
                        label,
                        value: String::new(),
                    },
                )
            })
            .collect(),
            dirty: false,
        }
    }

    fn load() -> Result<Self, Box<dyn Error>> {
        let mut config = AppConfig::new();
        let path = config_path()?;
        if path.exists() {
            let saved: HashMap<String, String> = serde_json::from_str(&fs::read_to_string(&path)?)?;
            for (k, v) in saved {
                if let Some(entry) = config.entries.get_mut(k.as_str()) {
                    entry.value = v;
                }
            }
        }
        let mut dirty = false;
        for (key, entry) in config.entries.iter_mut() {
            if let Ok(v) = env::var(key) {
                entry.value = v;
            } else if entry.value.is_empty() {
                entry.value = prompt(entry.label)?;
                dirty = true;
            }
        }
        config.dirty = dirty;
        config.save()?;
        Ok(config)
    }

    /// Retrieves values for the given keys in one call, returning them as a fixed-size array.
    ///
    /// # Errors
    ///
    /// Returns an error if any key is not a known config field.
    pub fn get<const N: usize>(&self, keys: [&str; N]) -> Result<[&String; N], Box<dyn Error>> {
        let mut results = Vec::with_capacity(N);
        for key in keys {
            results.push(
                self.entries
                    .get(key)
                    .map(|e| &e.value)
                    .ok_or_else(|| format!("unknown config field: {key}"))?,
            );
        }
        Ok(results.try_into().unwrap())
    }

    fn save(&self) -> Result<(), Box<dyn Error>> {
        if !self.dirty {
            return Ok(());
        }
        let path = config_path()?;
        fs::create_dir_all(path.parent().ok_or("invalid config path")?)?;
        let values: HashMap<&str, &str> = self
            .entries
            .iter()
            .map(|(k, e)| (*k, e.value.as_str()))
            .collect();
        fs::write(&path, serde_json::to_string_pretty(&values)?)?;
        Ok(())
    }

    fn setup(&mut self) -> Result<(), Box<dyn Error>> {
        let mut changed = false;
        for entry in self.entries.values_mut() {
            if let Some(v) = prompt_with_status(entry.label, !entry.value.is_empty())? {
                entry.value = v;
                changed = true;
            }
        }
        if changed {
            self.dirty = true;
        }
        self.save()
    }
}

struct SecretEntry {
    /// Human readable label describing what the secret holds
    label: &'static str,
    keyring_key: &'static str,
    value: String,
    dirty: bool,
}

/// Secrets (API keys) loaded from the OS keyring.
///
/// Missing values are prompted for interactively on first run and persisted to the keyring.
/// Any key can be overridden at runtime by setting the corresponding environment variable.
///
/// The `keyring_key` field on each entry is used as a label within the keyring to
/// distinguish secrets — it is not an OS username.
pub struct AppSecrets {
    entries: HashMap<&'static str, SecretEntry>,
}

impl AppSecrets {
    fn new() -> Self {
        AppSecrets {
            entries: [
                (
                    "APP_KEY",
                    SecretEntry {
                        label: "App key",
                        keyring_key: "app-key",
                        value: String::new(),
                        dirty: false,
                    },
                ),
                (
                    "CLIENT_KEY",
                    SecretEntry {
                        label: "Client key",
                        keyring_key: "client-key",
                        value: String::new(),
                        dirty: false,
                    },
                ),
            ]
            .into_iter()
            .collect(),
        }
    }

    fn load() -> Result<Self, Box<dyn Error>> {
        let mut secrets = AppSecrets::new();
        for (key, entry) in secrets.entries.iter_mut() {
            if let Ok(v) = env::var(key) {
                entry.value = v;
            } else {
                entry.value = keyring::Entry::new("mood", entry.keyring_key)
                    .ok()
                    .and_then(|e| e.get_password().ok())
                    .unwrap_or_default();
                if entry.value.is_empty() {
                    entry.value = prompt(entry.label)?;
                    entry.dirty = true;
                }
            }
        }
        secrets.save()?;
        Ok(secrets)
    }

    /// Retrieves values for the given keys in one call, returning them as a fixed-size array.
    ///
    /// # Errors
    ///
    /// Returns an error if any key is not a known secret.
    pub fn get<const N: usize>(&self, keys: [&str; N]) -> Result<[&String; N], Box<dyn Error>> {
        let mut results = Vec::with_capacity(N);
        for key in keys {
            results.push(
                self.entries
                    .get(key)
                    .map(|e| &e.value)
                    .ok_or_else(|| format!("unknown secret: {key}"))?,
            );
        }
        Ok(results.try_into().unwrap())
    }

    fn save(&self) -> Result<(), Box<dyn Error>> {
        for entry in self.entries.values() {
            if entry.dirty {
                keyring::Entry::new("mood", entry.keyring_key)?.set_password(&entry.value)?;
            }
        }
        Ok(())
    }

    fn setup(&mut self) -> Result<(), Box<dyn Error>> {
        for entry in self.entries.values_mut() {
            if let Some(v) = prompt_with_status(entry.label, !entry.value.is_empty())? {
                entry.value = v;
                entry.dirty = true;
            }
        }
        self.save()
    }
}

/// Combined application settings: non-secret config and secrets.
pub struct AppSettings {
    pub config: AppConfig,
    pub secrets: AppSecrets,
}

impl AppSettings {
    /// Loads both config and secrets, prompting for any missing values.
    pub fn load() -> Result<Self, Box<dyn Error>> {
        Ok(AppSettings {
            config: AppConfig::load()?,
            secrets: AppSecrets::load()?,
        })
    }

    /// Runs the interactive setup wizard, prompting for all config and secret values.
    pub fn run_setup(&mut self) -> Result<(), Box<dyn Error>> {
        self.config.setup()?;
        self.secrets.setup()?;
        Ok(())
    }
}
