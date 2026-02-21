use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::AppConfig;

#[derive(Clone)]
pub struct AuthorizedToken {
    pub name: String,
    pub token_id: String,
    pub rate_limit_per_minute: Option<usize>,
}

#[derive(Clone)]
pub struct TokenStore {
    tokens: Arc<HashMap<String, TokenPolicy>>,
}

#[derive(Clone)]
struct TokenPolicy {
    name: String,
    token_id: String,
    expires_at: Option<DateTime<Utc>>,
    rate_limit_per_minute: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TokenFile {
    pub tokens: Vec<TokenEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TokenEntry {
    pub name: String,
    pub token: String,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub rate_limit_per_minute: Option<usize>,
}

impl TokenStore {
    pub fn from_config(config: &AppConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let mut map = HashMap::new();

        if let Some(path) = &config.tokens_file {
            let file = load_token_file(path)?;
            for entry in file.tokens {
                let raw_token = entry.token.clone();
                let policy = TokenPolicy::from_entry(entry)?;
                map.insert(raw_token, policy);
            }
        }

        if let Some(legacy) = &config.upload_token {
            let policy = TokenPolicy {
                name: "legacy-default".to_string(),
                token_id: token_fingerprint(legacy),
                expires_at: None,
                rate_limit_per_minute: None,
            };
            map.entry(legacy.clone()).or_insert(policy);
        }

        if map.is_empty() {
            return Err("no upload token configured; set UPLOAD_TOKEN or TOKENS_FILE".into());
        }

        Ok(Self {
            tokens: Arc::new(map),
        })
    }

    pub fn authorize(&self, raw: &str) -> Option<AuthorizedToken> {
        let policy = self.tokens.get(raw)?;

        if let Some(exp) = policy.expires_at {
            if Utc::now() > exp {
                return None;
            }
        }

        Some(AuthorizedToken {
            name: policy.name.clone(),
            token_id: policy.token_id.clone(),
            rate_limit_per_minute: policy.rate_limit_per_minute,
        })
    }
}

impl TokenPolicy {
    fn from_entry(entry: TokenEntry) -> Result<Self, Box<dyn std::error::Error>> {
        let expires_at = if let Some(raw) = &entry.expires_at {
            Some(DateTime::parse_from_rfc3339(raw)?.with_timezone(&Utc))
        } else {
            None
        };

        Ok(Self {
            token_id: token_fingerprint(&entry.token),
            name: entry.name,
            expires_at,
            rate_limit_per_minute: entry.rate_limit_per_minute,
        })
    }
}

fn load_token_file(path: &Path) -> Result<TokenFile, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(TokenFile { tokens: vec![] });
    }

    let data = fs::read_to_string(path)?;
    let file: TokenFile = serde_json::from_str(&data)?;
    Ok(file)
}

pub fn resolve_tokens_file(arg: Option<&str>) -> PathBuf {
    if let Some(v) = arg {
        return PathBuf::from(v);
    }
    if let Ok(v) = std::env::var("TOKENS_FILE") {
        return PathBuf::from(v);
    }
    PathBuf::from("/opt/imgd/conf/tokens.json")
}

pub fn token_cli(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.is_empty() {
        print_token_help();
        return Ok(());
    }

    match args[0].as_str() {
        "create" => token_create(&args[1..]),
        "list" => token_list(&args[1..]),
        "revoke" => token_revoke(&args[1..]),
        _ => {
            print_token_help();
            Ok(())
        }
    }
}

fn token_create(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut name = "default".to_string();
    let mut expires_at: Option<String> = None;
    let mut rate_limit: Option<usize> = None;
    let mut never_expire = false;
    let mut days: Option<i64> = None;
    let mut file_arg: Option<String> = None;

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--name" => {
                name = args.get(i + 1).ok_or("missing value for --name")?.clone();
                i += 2;
            }
            "--expires-at" => {
                expires_at = Some(
                    args.get(i + 1)
                        .ok_or("missing value for --expires-at")?
                        .clone(),
                );
                i += 2;
            }
            "--days" => {
                days = Some(args.get(i + 1).ok_or("missing value for --days")?.parse()?);
                i += 2;
            }
            "--never-expire" => {
                never_expire = true;
                i += 1;
            }
            "--rate-limit" => {
                rate_limit = Some(
                    args.get(i + 1)
                        .ok_or("missing value for --rate-limit")?
                        .parse()?,
                );
                i += 2;
            }
            "--tokens-file" => {
                file_arg = Some(
                    args.get(i + 1)
                        .ok_or("missing value for --tokens-file")?
                        .clone(),
                );
                i += 2;
            }
            other => return Err(format!("unknown arg: {other}").into()),
        }
    }

    if never_expire {
        expires_at = None;
    } else if let Some(d) = days {
        expires_at = Some((Utc::now() + chrono::Duration::days(d)).to_rfc3339());
    }

    if let Some(raw) = &expires_at {
        let _ = DateTime::parse_from_rfc3339(raw)?;
    }

    let token = generate_token();
    let path = resolve_tokens_file(file_arg.as_deref());
    let mut file = load_token_file(&path)?;

    file.tokens.push(TokenEntry {
        name: name.clone(),
        token: token.clone(),
        expires_at: expires_at.clone(),
        rate_limit_per_minute: rate_limit,
    });

    save_token_file(&path, &file)?;

    println!("token created");
    println!("name: {name}");
    println!("token: {token}");
    println!(
        "expires_at: {}",
        expires_at.unwrap_or_else(|| "never".to_string())
    );
    println!(
        "rate_limit_per_minute: {}",
        rate_limit
            .map(|v| v.to_string())
            .unwrap_or_else(|| "inherit-global".to_string())
    );
    println!("tokens_file: {}", path.display());
    println!("restart imgd service to apply: sudo systemctl restart imgd");

    Ok(())
}

fn token_list(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut file_arg: Option<String> = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--tokens-file" => {
                file_arg = Some(
                    args.get(i + 1)
                        .ok_or("missing value for --tokens-file")?
                        .clone(),
                );
                i += 2;
            }
            other => return Err(format!("unknown arg: {other}").into()),
        }
    }

    let path = resolve_tokens_file(file_arg.as_deref());
    let file = load_token_file(&path)?;

    println!("tokens_file: {}", path.display());
    for entry in file.tokens {
        println!(
            "name={} expires_at={} rate_limit_per_minute={} token_id={}",
            entry.name,
            entry.expires_at.unwrap_or_else(|| "never".to_string()),
            entry
                .rate_limit_per_minute
                .map(|v| v.to_string())
                .unwrap_or_else(|| "inherit-global".to_string()),
            token_fingerprint(&entry.token)
        );
    }

    Ok(())
}

fn token_revoke(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut by_name: Option<String> = None;
    let mut by_token: Option<String> = None;
    let mut file_arg: Option<String> = None;

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--name" => {
                by_name = Some(args.get(i + 1).ok_or("missing value for --name")?.clone());
                i += 2;
            }
            "--token" => {
                by_token = Some(args.get(i + 1).ok_or("missing value for --token")?.clone());
                i += 2;
            }
            "--tokens-file" => {
                file_arg = Some(
                    args.get(i + 1)
                        .ok_or("missing value for --tokens-file")?
                        .clone(),
                );
                i += 2;
            }
            other => return Err(format!("unknown arg: {other}").into()),
        }
    }

    if by_name.is_none() && by_token.is_none() {
        return Err("revoke requires --name or --token".into());
    }

    let path = resolve_tokens_file(file_arg.as_deref());
    let mut file = load_token_file(&path)?;

    let before = file.tokens.len();
    file.tokens.retain(|entry| {
        if let Some(name) = &by_name {
            if &entry.name == name {
                return false;
            }
        }
        if let Some(token) = &by_token {
            if &entry.token == token {
                return false;
            }
        }
        true
    });

    save_token_file(&path, &file)?;
    println!(
        "removed {} token(s)",
        before.saturating_sub(file.tokens.len())
    );
    println!("restart imgd service to apply: sudo systemctl restart imgd");

    Ok(())
}

fn save_token_file(path: &Path, file: &TokenFile) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp = path.with_extension("tmp");
    let data = serde_json::to_string_pretty(file)?;
    fs::write(&tmp, data)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn generate_token() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn token_fingerprint(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let full = hex::encode(hasher.finalize());
    full.chars().take(12).collect()
}

fn print_token_help() {
    println!("imgd token commands:");
    println!("  imgd token create [--name N] [--expires-at RFC3339 | --days N | --never-expire] [--rate-limit N] [--tokens-file PATH]");
    println!("  imgd token list [--tokens-file PATH]");
    println!("  imgd token revoke (--name N | --token TOKEN) [--tokens-file PATH]");
}
