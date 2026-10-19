// The model catalog (models.toml) and locating or downloading GGUF files.

use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    /// Model loaded at startup and used when a request names none.
    pub default: String,
    #[serde(default)]
    pub defaults: Defaults,
    pub models: BTreeMap<String, Spec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Defaults {
    pub context: u32,
    pub max_tokens: usize,
}

impl Default for Defaults {
    fn default() -> Self {
        Defaults {
            context: 4096,
            max_tokens: 1024,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Follows the client's prompt and uses screen context.
    #[default]
    General,
    /// Fine-tuned only for dictation cleanup: always gets `system_prompt` and
    /// the bare transcript.
    Cleanup,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    /// Hugging Face repo and file, or a local `path`.
    pub repo: Option<String>,
    pub file: Option<String>,
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub kind: Kind,
    /// Required for cleanup models: the prompt they were trained on.
    pub system_prompt: Option<String>,
    /// Text appended after the assistant turn starts, e.g. an empty think
    /// block so a reasoning model answers directly.
    pub assistant_prefix: Option<String>,
    pub context: Option<u32>,
    pub description: Option<String>,
}

impl Catalog {
    pub fn load(path: &Path) -> Result<Catalog> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let catalog: Catalog =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        catalog.check()?;
        Ok(catalog)
    }

    fn check(&self) -> Result<()> {
        if !self.models.contains_key(&self.default) {
            bail!("default model '{}' is not in [models]", self.default);
        }
        for (id, spec) in &self.models {
            match (&spec.repo, &spec.file, &spec.path) {
                (Some(_), Some(_), None) | (None, None, Some(_)) => {}
                _ => bail!("model '{id}': set either repo and file, or path"),
            }
            if spec.kind == Kind::Cleanup && spec.system_prompt.is_none() {
                bail!("model '{id}': cleanup models need a system_prompt");
            }
        }
        Ok(())
    }
}

/// The GGUF file for a model: its local path, a copy already in the Hugging
/// Face cache (e.g. from llama.cpp), or a fresh download into `models_dir`.
pub async fn ensure(spec: &Spec, models_dir: &Path) -> Result<PathBuf> {
    if let Some(path) = &spec.path {
        return Ok(path.clone());
    }
    let (repo, file) = (spec.repo.as_deref().unwrap(), spec.file.as_deref().unwrap());
    if let Some(cached) = hf_cache(repo, file) {
        return Ok(cached);
    }
    let path = models_dir.join(file);
    if path.exists() {
        return Ok(path);
    }
    download(
        &format!("https://huggingface.co/{repo}/resolve/main/{file}"),
        &path,
    )
    .await?;
    Ok(path)
}

fn hf_cache(repo: &str, file: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HF_HUB_CACHE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HF_HOME").map(|h| PathBuf::from(h).join("hub")))
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/huggingface/hub"))
        })?;
    let snapshots = home
        .join(format!("models--{}", repo.replace('/', "--")))
        .join("snapshots");
    std::fs::read_dir(snapshots)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path().join(file))
        .find(|p| p.exists())
}

async fn download(url: &str, path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    eprintln!("downloading {url}");
    let resp = reqwest::get(url).await?.error_for_status()?;
    let part = path.with_extension("part");
    let mut file = tokio::fs::File::create(&part).await?;
    let mut body = resp.bytes_stream();
    while let Some(chunk) = body.next().await {
        file.write_all(&chunk?).await?;
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&part, path).await?;
    eprintln!("saved {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_catalog_is_valid() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("models.toml");
        let catalog = Catalog::load(&path).unwrap();
        assert!(catalog.models.len() >= 2);
    }

    #[test]
    fn rejects_bad_specs() {
        let bad = |s: &str| toml::from_str::<Catalog>(s).map(|c| c.check());
        assert!(bad("default = \"a\"\n[models.b]\npath = \"/x.gguf\"")
            .unwrap()
            .is_err());
        assert!(bad("default = \"a\"\n[models.a]\nrepo = \"r\"")
            .unwrap()
            .is_err());
        assert!(
            bad("default = \"a\"\n[models.a]\npath = \"/x\"\nkind = \"cleanup\"")
                .unwrap()
                .is_err()
        );
        assert!(bad("default = \"a\"\n[models.a]\npath = \"/x\"\nwat = 1").is_err());
    }
}
