//! Embedding providers behind a single trait. Default is the deterministic
//! `stub` (no network, used until a real provider is configured). The settings
//! layer can select Gemini, OpenAI, Ollama, or any OpenAI-compatible embedding
//! endpoint (for example Alibaba Cloud Model Studio / Bailian).

use crate::error::{Error, Result};

pub const STUB_DIM: usize = 256;

/// Settings resolved by the command layer before an embedding request. Keeping
/// this independent from SQLite makes every ingestion and search path share the
/// same provider/model/credential selection without duplicating provider logic.
#[derive(Debug, Clone, Default)]
pub struct EmbedConfig {
    pub provider: String,
    pub model: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub ollama_url: Option<String>,
}

pub trait Embedder: Send + Sync {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn dim(&self) -> usize;
    fn name(&self) -> &'static str;
}

/// Deterministic, dependency-free embedder. Hashes token n-grams into a fixed
/// vector so the same text always yields the same vector and similar texts
/// share dimensions. Good enough to exercise the whole pipeline offline.
pub struct StubEmbedder;

impl StubEmbedder {
    fn embed_one(text: &str) -> Vec<f32> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut v = vec![0.0f32; STUB_DIM];
        let lower = text.to_lowercase();
        for tok in lower.split(|c: char| !c.is_alphanumeric()) {
            if tok.is_empty() {
                continue;
            }
            let mut h = DefaultHasher::new();
            tok.hash(&mut h);
            let idx = (h.finish() as usize) % STUB_DIM;
            // sign from a second hash bit for some cancellation
            let sign = if (h.finish() >> 33) & 1 == 0 { 1.0 } else { -1.0 };
            v[idx] += sign;
        }
        // L2 normalize
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        v
    }
}

impl Embedder for StubEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| Self::embed_one(t)).collect())
    }
    fn dim(&self) -> usize {
        STUB_DIM
    }
    fn name(&self) -> &'static str {
        "stub"
    }
}

/// Gemini text-embedding-004 (BYOK). Requires `gemini_api_key` in settings.
pub struct GeminiEmbedder {
    pub api_key: String,
    pub model: String,
}

impl Embedder for GeminiEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        // batchEmbedContents takes up to 100 contents per call — one round-trip
        // per 100 chunks instead of one per chunk.
        let client = reqwest::blocking::Client::new();
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:batchEmbedContents?key={}",
            self.model, self.api_key
        );
        let mut out = Vec::with_capacity(texts.len());
        for batch in texts.chunks(100) {
            let requests: Vec<_> = batch
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "model": format!("models/{}", self.model),
                        "content": { "parts": [{ "text": t }] }
                    })
                })
                .collect();
            let resp = client
                .post(&url)
                .json(&serde_json::json!({ "requests": requests }))
                .send()?;
            if !resp.status().is_success() {
                return Err(Error::Other(format!(
                    "gemini embed failed: {}",
                    resp.status()
                )));
            }
            let json: serde_json::Value = resp.json()?;
            let embeddings = json["embeddings"]
                .as_array()
                .ok_or_else(|| Error::Other("gemini: no embeddings".into()))?;
            if embeddings.len() != batch.len() {
                return Err(Error::Other(format!(
                    "gemini: expected {} embeddings, got {}",
                    batch.len(),
                    embeddings.len()
                )));
            }
            for e in embeddings {
                let vals = e["values"]
                    .as_array()
                    .ok_or_else(|| Error::Other("gemini: no embedding values".into()))?;
                out.push(
                    vals.iter()
                        .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                        .collect(),
                );
            }
        }
        Ok(out)
    }
    fn dim(&self) -> usize {
        768
    }
    fn name(&self) -> &'static str {
        "gemini"
    }
}

/// OpenAI's `/embeddings` request/response format is also used by many hosted
/// providers. This lets a user keep generation on one provider while using a
/// different vector model such as Bailian's `text-embedding-v4`.
pub struct OpenAiCompatEmbedder {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub label: &'static str,
}

impl OpenAiCompatEmbedder {
    fn endpoint(&self) -> Result<String> {
        let base = self.base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            return Err(Error::Other(format!(
                "{} embedding endpoint is empty — set it in Settings → API keys.",
                self.label
            )));
        }
        let endpoint = if base.ends_with("/embeddings") {
            base.to_string()
        } else {
            format!("{base}/embeddings")
        };
        let url = reqwest::Url::parse(&endpoint).map_err(|_| {
            Error::Other(format!(
                "{} embedding endpoint must be a valid http:// or https:// URL",
                self.label
            ))
        })?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(Error::Other(format!(
                "{} embedding endpoint must use http:// or https://",
                self.label
            )));
        }
        Ok(endpoint)
    }
}

impl Embedder for OpenAiCompatEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let key = self.api_key.trim();
        if key.is_empty() {
            return Err(Error::Other(format!(
                "{} embedding API key is empty — set it in Settings → API keys.",
                self.label
            )));
        }
        let client = reqwest::blocking::Client::new();
        let url = self.endpoint()?;
        let mut out = Vec::with_capacity(texts.len());

        // The OpenAI-compatible contract accepts an array. Small batches are
        // accepted by OpenAI, Bailian, and common self-hosted gateways alike.
        for batch in texts.chunks(64) {
            let resp = client
                .post(&url)
                .bearer_auth(key)
                .json(&serde_json::json!({
                    "model": self.model,
                    "input": batch,
                    "encoding_format": "float"
                }))
                .send()?;
            if !resp.status().is_success() {
                return Err(Error::Other(format!(
                    "{} embedding failed: HTTP {}",
                    self.label,
                    resp.status()
                )));
            }
            let json: serde_json::Value = resp.json()?;
            let data = json["data"]
                .as_array()
                .ok_or_else(|| Error::Other(format!("{}: no embedding data", self.label)))?;
            if data.len() != batch.len() {
                return Err(Error::Other(format!(
                    "{}: expected {} embeddings, got {}",
                    self.label,
                    batch.len(),
                    data.len()
                )));
            }

            // OpenAI returns an `index`; sort by it so a compliant server that
            // reorders a batch still aligns every vector with its source chunk.
            let mut ordered: Vec<(usize, &serde_json::Value)> = data
                .iter()
                .enumerate()
                .map(|(fallback, item)| {
                    let index = item["index"]
                        .as_u64()
                        .map(|n| n as usize)
                        .unwrap_or(fallback);
                    (index, item)
                })
                .collect();
            ordered.sort_by_key(|(index, _)| *index);
            for (_, item) in ordered {
                let values = item["embedding"]
                    .as_array()
                    .ok_or_else(|| Error::Other(format!("{}: bad embedding shape", self.label)))?;
                if values.is_empty() {
                    return Err(Error::Other(format!("{}: empty embedding", self.label)));
                }
                out.push(
                    values
                        .iter()
                        .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                        .collect(),
                );
            }
        }
        Ok(out)
    }

    // Compatible providers can expose different dimensions (and Bailian v4 can
    // be configured with several). The actual vector length is stored per chunk;
    // callers intentionally do not rely on this advisory value.
    fn dim(&self) -> usize {
        0
    }
    fn name(&self) -> &'static str {
        "openai-compatible"
    }
}

/// Ollama nomic-embed-text on localhost (offline, $0). Honors `ollama_url`.
pub struct OllamaEmbedder {
    pub base_url: String,
    pub model: String,
}

impl OllamaEmbedder {
    /// Legacy one-text-per-request endpoint, kept as a fallback for Ollama
    /// versions that predate the batched `/api/embed`.
    fn embed_singly(&self, client: &reqwest::blocking::Client, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/api/embeddings", self.base_url.trim_end_matches('/'));
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            let body = serde_json::json!({ "model": self.model, "prompt": t });
            let resp = client.post(&url).json(&body).send()?;
            if !resp.status().is_success() {
                return Err(Error::Other(format!(
                    "ollama embed failed: {}",
                    resp.status()
                )));
            }
            let json: serde_json::Value = resp.json()?;
            let vals = json["embedding"]
                .as_array()
                .ok_or_else(|| Error::Other("ollama: no embedding".into()))?;
            out.push(
                vals.iter()
                    .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                    .collect(),
            );
        }
        Ok(out)
    }
}

impl Embedder for OllamaEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let client = reqwest::blocking::Client::new();
        // Batched endpoint (Ollama ≥0.1.45): all texts in one request.
        let url = format!("{}/api/embed", self.base_url.trim_end_matches('/'));
        let body = serde_json::json!({ "model": self.model, "input": texts });
        let resp = client.post(&url).json(&body).send()?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return self.embed_singly(&client, texts);
        }
        if !resp.status().is_success() {
            return Err(Error::Other(format!(
                "ollama embed failed: {}",
                resp.status()
            )));
        }
        let json: serde_json::Value = resp.json()?;
        let embeddings = json["embeddings"]
            .as_array()
            .ok_or_else(|| Error::Other("ollama: no embeddings".into()))?;
        if embeddings.len() != texts.len() {
            return Err(Error::Other(format!(
                "ollama: expected {} embeddings, got {}",
                texts.len(),
                embeddings.len()
            )));
        }
        embeddings
            .iter()
            .map(|e| {
                e.as_array()
                    .map(|vals| {
                        vals.iter()
                            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                            .collect::<Vec<f32>>()
                    })
                    .ok_or_else(|| Error::Other("ollama: bad embedding shape".into()))
            })
            .collect()
    }
    fn dim(&self) -> usize {
        768
    }
    fn name(&self) -> &'static str {
        "ollama"
    }
}

/// Build an embedder from resolved settings. Falls back to the stub on missing
/// credentials so the app always works offline with zero configuration.
pub fn from_config(config: &EmbedConfig) -> Box<dyn Embedder> {
    let model = config.model.trim();
    match config.provider.as_str() {
        "gemini" => match config.api_key.as_deref() {
            Some(k) if !k.is_empty() => Box::new(GeminiEmbedder {
                api_key: k.to_string(),
                model: if model.is_empty() {
                    "text-embedding-004".to_string()
                } else {
                    model.to_string()
                },
            }),
            _ => Box::new(StubEmbedder),
        },
        "openai" | "custom" => match (config.api_key.as_deref(), config.base_url.as_deref()) {
            (Some(key), Some(base_url)) if !key.trim().is_empty() && !base_url.trim().is_empty() => {
                Box::new(OpenAiCompatEmbedder {
                    base_url: base_url.to_string(),
                    api_key: key.to_string(),
                    model: if model.is_empty() {
                        if config.provider == "openai" {
                            "text-embedding-3-small".to_string()
                        } else {
                            "text-embedding-v4".to_string()
                        }
                    } else {
                        model.to_string()
                    },
                    label: if config.provider == "openai" { "OpenAI" } else { "Custom" },
                })
            }
            _ => Box::new(StubEmbedder),
        },
        "ollama" => Box::new(OllamaEmbedder {
            base_url: config
                .ollama_url
                .as_deref()
                .unwrap_or("http://localhost:11434")
                .to_string(),
            model: if model.is_empty() {
                "nomic-embed-text".to_string()
            } else {
                model.to_string()
            },
        }),
        _ => Box::new(StubEmbedder),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_is_deterministic_and_normalized() {
        let e = StubEmbedder;
        let a = e.embed(&["recursion base case".into()]).unwrap();
        let b = e.embed(&["recursion base case".into()]).unwrap();
        assert_eq!(a, b);
        assert_eq!(a[0].len(), STUB_DIM);
        let norm: f32 = a[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[test]
    fn similar_text_scores_higher_than_unrelated() {
        use crate::vector::cosine;
        let e = StubEmbedder;
        let q = &e.embed(&["dynamic programming memoization".into()]).unwrap()[0];
        let close = &e
            .embed(&["memoization in dynamic programming".into()])
            .unwrap()[0];
        let far = &e.embed(&["the cat sat on the mat".into()]).unwrap()[0];
        assert!(cosine(q, close) > cosine(q, far));
    }

    #[test]
    fn compatible_endpoint_accepts_https_and_local_http() {
        let base = OpenAiCompatEmbedder {
            base_url: "https://example.com/v1/".into(),
            api_key: "test".into(),
            model: "text-embedding-test".into(),
            label: "Custom",
        };
        assert_eq!(base.endpoint().unwrap(), "https://example.com/v1/embeddings");
        let full = OpenAiCompatEmbedder {
            base_url: "https://example.com/v1/embeddings".into(),
            ..base
        };
        assert_eq!(full.endpoint().unwrap(), "https://example.com/v1/embeddings");

        let local = OpenAiCompatEmbedder {
            base_url: "http://192.168.1.42:8000/v1".into(),
            ..full
        };
        assert_eq!(
            local.endpoint().unwrap(),
            "http://192.168.1.42:8000/v1/embeddings"
        );
    }
}
