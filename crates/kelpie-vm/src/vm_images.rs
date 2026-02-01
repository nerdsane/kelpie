//! VM Image Management
//!
//! TigerStyle: Explicit image paths, checksums, and first-run download.
//!
//! This module provides automatic download and caching of VM images required
//! for different VM backends:
//!
//! # Backend-Specific Requirements
//!
//! ## libkrun (recommended)
//! - **Root Filesystem**: Directory-based rootfs with guest agent installed
//! - Location: `~/.cache/kelpie/libkrun-rootfs/`
//! - libkrun bundles its own kernel via libkrunfw
//!
//! ## Apple VZ (legacy)
//! - **Kernel**: Linux kernel built for ARM64 (vmlinuz-aarch64)
//! - **Root Filesystem**: ext4 image with guest agent installed
//! - Location: `~/.cache/kelpie/vm-images/`
//!
//! # Security
//!
//! All downloads are verified by SHA256 checksum before use.

use crate::error::{VmError, VmResult};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{debug, info, warn};

// ============================================================================
// TigerStyle Constants
// ============================================================================

/// Cache directory name under user's cache directory (for ext4 images)
const VM_IMAGES_CACHE_DIR: &str = "kelpie/vm-images";

/// Cache directory name for libkrun rootfs (directory-based)
const LIBKRUN_ROOTFS_CACHE_DIR: &str = "kelpie/libkrun-rootfs";

/// Kernel image filename
const KERNEL_FILENAME: &str = "vmlinuz-aarch64";

/// Root filesystem filename
const ROOTFS_FILENAME: &str = "rootfs-aarch64.ext4";

/// Expected kernel size (approximate, for sanity check)
const KERNEL_SIZE_BYTES_MIN: u64 = 5 * 1024 * 1024; // 5 MiB minimum
const KERNEL_SIZE_BYTES_MAX: u64 = 50 * 1024 * 1024; // 50 MiB maximum

/// Expected rootfs size (approximate, for sanity check)
const ROOTFS_SIZE_BYTES_MIN: u64 = 20 * 1024 * 1024; // 20 MiB minimum
const ROOTFS_SIZE_BYTES_MAX: u64 = 500 * 1024 * 1024; // 500 MiB maximum

/// Download timeout in seconds
#[cfg(feature = "image-download")]
const DOWNLOAD_TIMEOUT_SECONDS: u64 = 600; // 10 minutes

// ============================================================================
// Image Checksums (SHA256)
// ============================================================================

/// SHA256 checksum for kernel image
/// NOTE: Update this when releasing new kernel builds
/// Built from Alpine 3.19 linux-virt kernel for ARM64
/// This is the raw Image format (not EFI stub) required by VZLinuxBootLoader
const KERNEL_SHA256: &str = "9d2a91f12624959d943247e4076c3e54bcb221ae3a2095c6bd9db0182346bc76";

/// SHA256 checksum for rootfs image
/// NOTE: Update this when releasing new rootfs builds
/// Built from Alpine 3.19 with kelpie-guest agent for ARM64
const ROOTFS_SHA256: &str = "ce75f9f5dd49af18766999f09c1fd1a0549e1705a814378bc731b151b172aaf8";

// ============================================================================
// Download URLs
// ============================================================================

/// Base URL for downloading VM images
/// Options:
/// 1. GitHub releases: https://github.com/nerdsane/kelpie/releases/download/vm-images-v1/
/// 2. S3/R2 bucket: https://kelpie-images.example.com/
const IMAGE_BASE_URL: &str = "https://github.com/nerdsane/kelpie/releases/download/vm-images-v1";

// ============================================================================
// VmImagePaths
// ============================================================================

/// Paths to VM images after download/verification
#[derive(Debug, Clone)]
pub struct VmImagePaths {
    /// Path to kernel image
    pub kernel: PathBuf,
    /// Path to root filesystem image
    pub rootfs: PathBuf,
}

impl VmImagePaths {
    /// Create new image paths
    pub fn new(kernel: PathBuf, rootfs: PathBuf) -> Self {
        Self { kernel, rootfs }
    }

    /// Verify both images exist and are readable
    pub async fn verify(&self) -> VmResult<()> {
        // Check kernel exists and has reasonable size
        let kernel_meta = fs::metadata(&self.kernel)
            .await
            .map_err(|e| VmError::ConfigInvalid {
                reason: format!("kernel image not found at {:?}: {}", self.kernel, e),
            })?;

        if kernel_meta.len() < KERNEL_SIZE_BYTES_MIN {
            return Err(VmError::ConfigInvalid {
                reason: format!(
                    "kernel image too small: {} bytes (minimum: {} bytes)",
                    kernel_meta.len(),
                    KERNEL_SIZE_BYTES_MIN
                ),
            });
        }

        if kernel_meta.len() > KERNEL_SIZE_BYTES_MAX {
            return Err(VmError::ConfigInvalid {
                reason: format!(
                    "kernel image too large: {} bytes (maximum: {} bytes)",
                    kernel_meta.len(),
                    KERNEL_SIZE_BYTES_MAX
                ),
            });
        }

        // Check rootfs exists and has reasonable size
        let rootfs_meta = fs::metadata(&self.rootfs)
            .await
            .map_err(|e| VmError::ConfigInvalid {
                reason: format!("rootfs image not found at {:?}: {}", self.rootfs, e),
            })?;

        if rootfs_meta.len() < ROOTFS_SIZE_BYTES_MIN {
            return Err(VmError::ConfigInvalid {
                reason: format!(
                    "rootfs image too small: {} bytes (minimum: {} bytes)",
                    rootfs_meta.len(),
                    ROOTFS_SIZE_BYTES_MIN
                ),
            });
        }

        if rootfs_meta.len() > ROOTFS_SIZE_BYTES_MAX {
            return Err(VmError::ConfigInvalid {
                reason: format!(
                    "rootfs image too large: {} bytes (maximum: {} bytes)",
                    rootfs_meta.len(),
                    ROOTFS_SIZE_BYTES_MAX
                ),
            });
        }

        Ok(())
    }
}

// ============================================================================
// VmImageManager
// ============================================================================

/// Manager for VM image download and caching
#[derive(Debug, Clone)]
pub struct VmImageManager {
    /// Cache directory for images
    cache_dir: PathBuf,
}

impl VmImageManager {
    /// Create a new image manager using the default cache directory
    pub fn new() -> VmResult<Self> {
        let cache_dir = get_cache_dir()?;
        Ok(Self { cache_dir })
    }

    /// Create an image manager with a custom cache directory
    pub fn with_cache_dir(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    /// Get path to kernel image
    pub fn kernel_path(&self) -> PathBuf {
        self.cache_dir.join(KERNEL_FILENAME)
    }

    /// Get path to rootfs image (ext4 format for Apple VZ)
    pub fn rootfs_path(&self) -> PathBuf {
        self.cache_dir.join(ROOTFS_FILENAME)
    }

    /// Get path to libkrun rootfs directory
    ///
    /// libkrun uses a directory-based rootfs, not an ext4 image.
    /// The rootfs should contain a standard Linux directory structure
    /// with the kelpie-guest binary at /usr/local/bin/kelpie-guest.
    pub fn libkrun_rootfs_path(&self) -> VmResult<PathBuf> {
        let libkrun_cache_dir = get_libkrun_cache_dir()?;
        let guest_agent_path = libkrun_cache_dir.join("usr/local/bin/kelpie-guest");

        if !libkrun_cache_dir.exists() {
            return Err(VmError::ConfigInvalid {
                reason: format!(
                    "libkrun rootfs not found at {:?}. \
                    Build it with: cd images && ./build-libkrun-rootfs.sh",
                    libkrun_cache_dir
                ),
            });
        }

        if !guest_agent_path.exists() {
            return Err(VmError::ConfigInvalid {
                reason: format!(
                    "kelpie-guest not found at {:?}. \
                    Rebuild the rootfs with: cd images && ./build-libkrun-rootfs.sh",
                    guest_agent_path
                ),
            });
        }

        Ok(libkrun_cache_dir)
    }

    /// Ensure images are available, downloading if necessary
    ///
    /// This is the main entry point for image management.
    /// Call this before creating VZ VMs.
    pub async fn ensure_images(&self) -> VmResult<VmImagePaths> {
        // Create cache directory if needed
        fs::create_dir_all(&self.cache_dir)
            .await
            .map_err(|e| VmError::ConfigInvalid {
                reason: format!(
                    "failed to create cache directory {:?}: {}",
                    self.cache_dir, e
                ),
            })?;

        let kernel_path = self.kernel_path();
        let rootfs_path = self.rootfs_path();

        // Check if images exist
        let kernel_exists = kernel_path.exists();
        let rootfs_exists = rootfs_path.exists();

        if kernel_exists && rootfs_exists {
            debug!(
                kernel = ?kernel_path,
                rootfs = ?rootfs_path,
                "VM images found in cache"
            );

            let paths = VmImagePaths::new(kernel_path, rootfs_path);
            paths.verify().await?;
            return Ok(paths);
        }

        // Download missing images
        info!("VM images not found, downloading...");

        if !kernel_exists {
            self.download_kernel(&kernel_path).await?;
        }

        if !rootfs_exists {
            self.download_rootfs(&rootfs_path).await?;
        }

        let paths = VmImagePaths::new(kernel_path, rootfs_path);
        paths.verify().await?;

        info!("VM images ready");
        Ok(paths)
    }

    /// Check if images are already cached
    pub fn images_cached(&self) -> bool {
        self.kernel_path().exists() && self.rootfs_path().exists()
    }

    /// Download kernel image
    async fn download_kernel(&self, dest: &Path) -> VmResult<()> {
        let url = format!("{}/{}", IMAGE_BASE_URL, KERNEL_FILENAME);
        info!(url = %url, dest = ?dest, "Downloading kernel image...");

        download_file(&url, dest, KERNEL_SHA256).await?;

        info!(dest = ?dest, "Kernel image downloaded successfully");
        Ok(())
    }

    /// Download rootfs image
    async fn download_rootfs(&self, dest: &Path) -> VmResult<()> {
        let url = format!("{}/{}", IMAGE_BASE_URL, ROOTFS_FILENAME);
        info!(url = %url, dest = ?dest, "Downloading rootfs image...");

        download_file(&url, dest, ROOTFS_SHA256).await?;

        info!(dest = ?dest, "Rootfs image downloaded successfully");
        Ok(())
    }

    /// Clear the image cache
    pub async fn clear_cache(&self) -> VmResult<()> {
        if self.cache_dir.exists() {
            fs::remove_dir_all(&self.cache_dir)
                .await
                .map_err(|e| VmError::ConfigInvalid {
                    reason: format!("failed to clear cache directory: {}", e),
                })?;
        }
        Ok(())
    }
}

impl Default for VmImageManager {
    fn default() -> Self {
        Self::new().expect("failed to create VmImageManager with default cache directory")
    }
}

// ============================================================================
// Download Functions
// ============================================================================

/// Get the cache directory for VM images (ext4 format)
fn get_cache_dir() -> VmResult<PathBuf> {
    // Try to get user's cache directory
    if let Some(cache_dir) = dirs::cache_dir() {
        return Ok(cache_dir.join(VM_IMAGES_CACHE_DIR));
    }

    // Fallback to home directory
    if let Some(home_dir) = dirs::home_dir() {
        return Ok(home_dir.join(".cache").join(VM_IMAGES_CACHE_DIR));
    }

    // Last resort: use temp directory
    warn!("Could not determine cache directory, using temp directory");
    Ok(std::env::temp_dir().join(VM_IMAGES_CACHE_DIR))
}

/// Get the cache directory for libkrun rootfs (directory format)
fn get_libkrun_cache_dir() -> VmResult<PathBuf> {
    // Try to get user's cache directory
    if let Some(cache_dir) = dirs::cache_dir() {
        return Ok(cache_dir.join(LIBKRUN_ROOTFS_CACHE_DIR));
    }

    // Fallback to home directory
    if let Some(home_dir) = dirs::home_dir() {
        return Ok(home_dir.join(".cache").join(LIBKRUN_ROOTFS_CACHE_DIR));
    }

    // Last resort: use temp directory
    warn!("Could not determine cache directory, using temp directory");
    Ok(std::env::temp_dir().join(LIBKRUN_ROOTFS_CACHE_DIR))
}

/// Download a file from URL to destination path with checksum verification
async fn download_file(url: &str, dest: &Path, expected_sha256: &str) -> VmResult<()> {
    // For now, return an error with instructions
    // Real implementation would use reqwest to download
    if expected_sha256.starts_with("PLACEHOLDER") {
        return Err(VmError::ConfigInvalid {
            reason: format!(
                "VM images not yet available for automatic download. \
                Please manually download from {} and place at {:?}. \
                Contact the maintainers for pre-built images.",
                url, dest
            ),
        });
    }

    // Download with reqwest (requires the feature to be enabled)
    #[cfg(feature = "image-download")]
    {
        use sha2::{Digest, Sha256};
        use std::time::Duration;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECONDS))
            .build()
            .map_err(|e| VmError::ConfigInvalid {
                reason: format!("failed to create HTTP client: {}", e),
            })?;

        let response = client
            .get(url)
            .send()
            .await
            .map_err(|e| VmError::ConfigInvalid {
                reason: format!("failed to download from {}: {}", url, e),
            })?;

        if !response.status().is_success() {
            return Err(VmError::ConfigInvalid {
                reason: format!("download failed with status {}: {}", response.status(), url),
            });
        }

        let bytes = response.bytes().await.map_err(|e| VmError::ConfigInvalid {
            reason: format!("failed to read response body: {}", e),
        })?;

        // Verify checksum
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let hash = hasher.finalize();
        let actual_sha256 = hex::encode(hash);

        if actual_sha256 != expected_sha256 {
            return Err(VmError::ConfigInvalid {
                reason: format!(
                    "checksum mismatch for {}: expected {}, got {}",
                    url, expected_sha256, actual_sha256
                ),
            });
        }

        // Write to file
        let mut file = fs::File::create(dest)
            .await
            .map_err(|e| VmError::ConfigInvalid {
                reason: format!("failed to create file {:?}: {}", dest, e),
            })?;

        file.write_all(&bytes)
            .await
            .map_err(|e| VmError::ConfigInvalid {
                reason: format!("failed to write file {:?}: {}", dest, e),
            })?;

        return Ok(());
    }

    #[cfg(not(feature = "image-download"))]
    {
        Err(VmError::ConfigInvalid {
            reason: format!(
                "Automatic image download not enabled. \
                Either enable the 'image-download' feature, or manually download \
                {} to {:?}",
                url, dest
            ),
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants_valid() {
        assert!(KERNEL_SIZE_BYTES_MIN < KERNEL_SIZE_BYTES_MAX);
        assert!(ROOTFS_SIZE_BYTES_MIN < ROOTFS_SIZE_BYTES_MAX);
    }

    #[test]
    #[cfg(feature = "image-download")]
    fn test_download_constants_valid() {
        assert!(DOWNLOAD_TIMEOUT_SECONDS > 0);
    }

    #[test]
    fn test_vm_image_paths() {
        let paths = VmImagePaths::new(
            PathBuf::from("/path/to/kernel"),
            PathBuf::from("/path/to/rootfs"),
        );
        assert_eq!(paths.kernel, PathBuf::from("/path/to/kernel"));
        assert_eq!(paths.rootfs, PathBuf::from("/path/to/rootfs"));
    }

    #[test]
    fn test_vm_image_manager_with_custom_dir() {
        let manager = VmImageManager::with_cache_dir(PathBuf::from("/tmp/test-images"));
        assert_eq!(
            manager.kernel_path(),
            PathBuf::from("/tmp/test-images/vmlinuz-aarch64")
        );
        assert_eq!(
            manager.rootfs_path(),
            PathBuf::from("/tmp/test-images/rootfs-aarch64.ext4")
        );
    }

    #[test]
    fn test_get_cache_dir() {
        let result = get_cache_dir();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.to_string_lossy().contains("kelpie"));
    }

    #[tokio::test]
    async fn test_vm_image_paths_verify_missing() {
        let paths = VmImagePaths::new(
            PathBuf::from("/nonexistent/kernel"),
            PathBuf::from("/nonexistent/rootfs"),
        );
        let result = paths.verify().await;
        assert!(result.is_err());
    }
}
