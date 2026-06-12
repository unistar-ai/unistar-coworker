use std::path::Path;

use crate::config::Config;
use crate::error::Result;
use crate::store::Digest;

pub fn maybe_export_digest(config: &Config, digest: &Digest) -> Result<()> {
    if !config.output.export_digest_md {
        return Ok(());
    }
    let path = config
        .output
        .digest_export_path
        .replace("{date}", &digest.date.to_string());
    if let Some(parent) = Path::new(&path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &digest.body_md)?;
    Ok(())
}
