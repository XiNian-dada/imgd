use std::{env, fs, net::SocketAddr, path::PathBuf};

#[derive(Clone)]
pub struct AppConfig {
    pub bind_addr: SocketAddr,
    pub upload_token: Option<String>,
    pub tokens_file: Option<PathBuf>,
    pub public_base_url: String,
    pub data_dir: PathBuf,
    pub max_upload_bytes: usize,
    pub max_concurrent_uploads: usize,
    pub rate_limit_per_minute: usize,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let port = env::var("PORT").unwrap_or_else(|_| "3000".to_owned());
        let bind_addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;

        let upload_token = env::var("UPLOAD_TOKEN").ok();
        let tokens_file = env::var("TOKENS_FILE").ok().map(PathBuf::from);
        let public_base_url = env::var("PUBLIC_BASE_URL")?;
        let data_dir = env::var("DATA_DIR").unwrap_or_else(|_| "/data/images".to_owned());

        if upload_token.is_none() && tokens_file.is_none() {
            return Err("UPLOAD_TOKEN or TOKENS_FILE must be set".into());
        }

        Ok(Self {
            bind_addr,
            upload_token,
            tokens_file,
            public_base_url,
            data_dir: PathBuf::from(data_dir),
            max_upload_bytes: 5 * 1024 * 1024,
            max_concurrent_uploads: env::var("MAX_CONCURRENT_UPLOADS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(16),
            rate_limit_per_minute: env::var("RATE_LIMIT_PER_MINUTE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60),
        })
    }

    pub fn ensure_data_dir_ready(&self) -> Result<(), Box<dyn std::error::Error>> {
        fs::create_dir_all(&self.data_dir)?;
        let tmp_dir = self.data_dir.join(".tmp");
        fs::create_dir_all(&tmp_dir)?;

        let probe = tmp_dir.join(".write_probe");
        fs::write(&probe, b"ok")?;
        fs::remove_file(probe)?;
        Ok(())
    }
}
