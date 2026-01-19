//! Embedding generation for semantic search
//!
//! TigerStyle: Trait-based embedder with explicit dimension constraints.
//!
//! Provides local embedding generation for text, enabling semantic search
//! without external API dependencies.

use crate::error::{MemoryError, MemoryResult};
use async_trait::async_trait;

/// Common embedding dimensions for popular models
pub const EMBEDDING_DIM_384: usize = 384; // all-MiniLM-L6-v2
pub const EMBEDDING_DIM_768: usize = 768; // all-mpnet-base-v2
pub const EMBEDDING_DIM_1024: usize = 1024;
pub const EMBEDDING_DIM_1536: usize = 1536; // OpenAI text-embedding-ada-002

/// Trait for generating text embeddings
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Get the dimension of embeddings produced by this embedder
    fn dimension(&self) -> usize;

    /// Get the model name/identifier
    fn model_name(&self) -> &str;

    /// Embed a single text string
    async fn embed(&self, text: &str) -> MemoryResult<Vec<f32>>;

    /// Embed multiple texts in a batch (more efficient for many texts)
    async fn embed_batch(&self, texts: &[&str]) -> MemoryResult<Vec<Vec<f32>>> {
        // Default implementation: embed one at a time
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(self.embed(text).await?);
        }
        Ok(results)
    }
}

/// A simple mock embedder for testing
///
/// Generates deterministic embeddings based on text content.
/// Not suitable for real semantic search - use only for testing.
#[derive(Debug, Clone)]
pub struct MockEmbedder {
    dimension: usize,
}

impl MockEmbedder {
    /// Create a new mock embedder with the specified dimension
    pub fn new(dimension: usize) -> Self {
        assert!(dimension > 0, "embedding dimension must be positive");
        Self { dimension }
    }

    /// Create a mock embedder with 384 dimensions (typical for small models)
    pub fn default_384() -> Self {
        Self::new(EMBEDDING_DIM_384)
    }
}

impl Default for MockEmbedder {
    fn default() -> Self {
        Self::default_384()
    }
}

#[async_trait]
impl Embedder for MockEmbedder {
    fn dimension(&self) -> usize {
        self.dimension
    }

    fn model_name(&self) -> &str {
        "mock-embedder"
    }

    async fn embed(&self, text: &str) -> MemoryResult<Vec<f32>> {
        // Generate a deterministic embedding based on text content
        // This creates a pseudo-hash that maps text to a consistent vector
        let mut embedding = vec![0.0f32; self.dimension];

        // Use a simple hash-based approach for determinism
        let bytes = text.as_bytes();
        let mut seed: u64 = 0;

        for (i, &byte) in bytes.iter().enumerate() {
            seed = seed.wrapping_add(byte as u64 * (i as u64 + 1));
            seed = seed.wrapping_mul(31);
        }

        // Fill embedding with pseudo-random values based on seed
        for (i, value) in embedding.iter_mut().enumerate() {
            let combined = seed.wrapping_add(i as u64);
            let hash = combined.wrapping_mul(0x517cc1b727220a95);
            // Map to [-1, 1] range
            *value = ((hash as i64) as f32) / (i64::MAX as f32);
        }

        // Normalize to unit vector
        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for value in &mut embedding {
                *value /= norm;
            }
        }

        Ok(embedding)
    }
}

/// Configuration for local embedding generation
#[derive(Debug, Clone)]
pub struct EmbedderConfig {
    /// Model name or path
    pub model: String,
    /// Whether to use GPU if available
    pub use_gpu: bool,
    /// Maximum batch size for embedding
    pub batch_size_max: usize,
    /// Maximum text length (will be truncated)
    pub text_length_max: usize,
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            model: "all-MiniLM-L6-v2".to_string(),
            use_gpu: false,
            batch_size_max: 32,
            text_length_max: 512,
        }
    }
}

impl EmbedderConfig {
    /// Create configuration for a specific model
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..Default::default()
        }
    }

    /// Enable GPU acceleration
    pub fn with_gpu(mut self) -> Self {
        self.use_gpu = true;
        self
    }

    /// Set maximum batch size
    pub fn with_batch_size(mut self, size: usize) -> Self {
        self.batch_size_max = size;
        self
    }
}

/// Local embedding using fastembed
///
/// Provides fast, local embedding generation using ONNX models.
/// Requires the `local-embeddings` feature to be enabled.
#[cfg(feature = "local-embeddings")]
#[allow(dead_code)]
pub struct LocalEmbedder {
    model: std::sync::Mutex<fastembed::TextEmbedding>,
    model_name: String,
    dimension: usize,
    config: EmbedderConfig,
}

#[cfg(feature = "local-embeddings")]
impl std::fmt::Debug for LocalEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalEmbedder")
            .field("model_name", &self.model_name)
            .field("dimension", &self.dimension)
            .field("config", &self.config)
            .finish()
    }
}

// Safety: LocalEmbedder uses Mutex for internal synchronization
#[cfg(feature = "local-embeddings")]
unsafe impl Send for LocalEmbedder {}
#[cfg(feature = "local-embeddings")]
unsafe impl Sync for LocalEmbedder {}

#[cfg(feature = "local-embeddings")]
#[allow(dead_code)]
impl LocalEmbedder {
    /// Create a new local embedder
    ///
    /// Downloads and initializes the specified embedding model.
    /// Models are cached locally after first download.
    pub fn new(config: EmbedderConfig) -> MemoryResult<Self> {
        use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

        // Map model name to fastembed enum
        let (model_enum, dimension) = match config.model.as_str() {
            "all-MiniLM-L6-v2" | "BAAI/bge-small-en-v1.5" => {
                (EmbeddingModel::BGESmallENV15, EMBEDDING_DIM_384)
            }
            "all-mpnet-base-v2" | "BAAI/bge-base-en-v1.5" => {
                (EmbeddingModel::BGEBaseENV15, EMBEDDING_DIM_768)
            }
            "BAAI/bge-large-en-v1.5" => (EmbeddingModel::BGELargeENV15, EMBEDDING_DIM_1024),
            _ => {
                return Err(MemoryError::EmbeddingError {
                    message: format!(
                        "Unsupported embedding model: {}. Supported: all-MiniLM-L6-v2, all-mpnet-base-v2, BAAI/bge-*",
                        config.model
                    ),
                });
            }
        };

        let init_options = InitOptions::new(model_enum).with_show_download_progress(true);

        let model =
            TextEmbedding::try_new(init_options).map_err(|e| MemoryError::EmbeddingError {
                message: format!("Failed to initialize embedding model: {}", e),
            })?;

        tracing::info!(
            model = %config.model,
            dimension = dimension,
            "Initialized local embedder"
        );

        Ok(Self {
            model: std::sync::Mutex::new(model),
            model_name: config.model.clone(),
            dimension,
            config,
        })
    }

    /// Create with the default model (all-MiniLM-L6-v2)
    pub fn default_model() -> MemoryResult<Self> {
        Self::new(EmbedderConfig::default())
    }
}

#[cfg(feature = "local-embeddings")]
#[async_trait]
impl Embedder for LocalEmbedder {
    fn dimension(&self) -> usize {
        self.dimension
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }

    async fn embed(&self, text: &str) -> MemoryResult<Vec<f32>> {
        // Truncate if too long
        let text = if text.len() > self.config.text_length_max {
            &text[..self.config.text_length_max]
        } else {
            text
        };

        let mut model = self.model.lock().map_err(|e| MemoryError::EmbeddingError {
            message: format!("Failed to acquire model lock: {}", e),
        })?;

        let embeddings =
            model
                .embed(vec![text], None)
                .map_err(|e| MemoryError::EmbeddingError {
                    message: format!("Failed to generate embedding: {}", e),
                })?;

        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| MemoryError::EmbeddingError {
                message: "No embedding returned".to_string(),
            })
    }

    async fn embed_batch(&self, texts: &[&str]) -> MemoryResult<Vec<Vec<f32>>> {
        // Truncate texts if needed
        let truncated: Vec<&str> = texts
            .iter()
            .map(|t| {
                if t.len() > self.config.text_length_max {
                    &t[..self.config.text_length_max]
                } else {
                    *t
                }
            })
            .collect();

        let mut model = self.model.lock().map_err(|e| MemoryError::EmbeddingError {
            message: format!("Failed to acquire model lock: {}", e),
        })?;

        // Process in batches
        let mut all_embeddings = Vec::with_capacity(texts.len());

        for chunk in truncated.chunks(self.config.batch_size_max) {
            let embeddings = model
                .embed(chunk, None)
                .map_err(|e| MemoryError::EmbeddingError {
                    message: format!("Failed to generate embeddings: {}", e),
                })?;

            all_embeddings.extend(embeddings);
        }

        Ok(all_embeddings)
    }
}

/// Placeholder LocalEmbedder when feature is disabled
#[cfg(not(feature = "local-embeddings"))]
#[derive(Debug)]
#[allow(dead_code)]
pub struct LocalEmbedder {
    _config: EmbedderConfig,
}

#[cfg(not(feature = "local-embeddings"))]
#[allow(dead_code)]
impl LocalEmbedder {
    /// Create a new local embedder (requires local-embeddings feature)
    pub fn new(_config: EmbedderConfig) -> MemoryResult<Self> {
        Err(MemoryError::EmbeddingError {
            message: "Local embeddings require the 'local-embeddings' feature. Enable it in Cargo.toml or use MockEmbedder for testing.".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_embedder_dimension() {
        let embedder = MockEmbedder::new(128);
        assert_eq!(embedder.dimension(), 128);
        assert_eq!(embedder.model_name(), "mock-embedder");
    }

    #[tokio::test]
    async fn test_mock_embedder_deterministic() {
        let embedder = MockEmbedder::default_384();

        let text = "hello world";
        let embedding1 = embedder.embed(text).await.unwrap();
        let embedding2 = embedder.embed(text).await.unwrap();

        assert_eq!(embedding1.len(), 384);
        assert_eq!(embedding1, embedding2, "embeddings should be deterministic");
    }

    #[tokio::test]
    async fn test_mock_embedder_different_texts() {
        let embedder = MockEmbedder::default_384();

        let embedding1 = embedder.embed("hello").await.unwrap();
        let embedding2 = embedder.embed("world").await.unwrap();

        assert_ne!(
            embedding1, embedding2,
            "different texts should have different embeddings"
        );
    }

    #[tokio::test]
    async fn test_mock_embedder_normalized() {
        let embedder = MockEmbedder::default_384();
        let embedding = embedder.embed("test text").await.unwrap();

        // Check that it's normalized (length ~= 1.0)
        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-5,
            "embedding should be normalized, got norm = {}",
            norm
        );
    }

    #[tokio::test]
    async fn test_mock_embedder_batch() {
        let embedder = MockEmbedder::default_384();
        let texts = &["hello", "world", "test"];

        let embeddings = embedder.embed_batch(texts).await.unwrap();
        assert_eq!(embeddings.len(), 3);

        // Verify each embedding matches individual calls
        for (i, text) in texts.iter().enumerate() {
            let individual = embedder.embed(text).await.unwrap();
            assert_eq!(embeddings[i], individual);
        }
    }

    #[test]
    fn test_embedder_config_builder() {
        let config = EmbedderConfig::new("custom-model")
            .with_gpu()
            .with_batch_size(64);

        assert_eq!(config.model, "custom-model");
        assert!(config.use_gpu);
        assert_eq!(config.batch_size_max, 64);
    }
}
