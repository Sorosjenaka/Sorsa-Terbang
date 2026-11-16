use anyhow::{bail, Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::info;

pub const SORSA_EXTENSION_ID: &str = "fbjnlbncangbglgcinecoijkkeejcjan";
pub const SORSA_CRX_URL: &str = "https://clients2.google.com/service/update2/crx?response=redirect&prodversion=130.0.0.0&acceptformat=crx2,crx3&x=id%3Dfbjnlbncangbglgcinecoijkkeejcjan%26uc";

pub struct SorsaExtensionProvisioner;

impl SorsaExtensionProvisioner {
    /// Ensure Sorsa extension exists in <workdir>/sorsa_extension
    /// 1. Check if <workdir>/sorsa_extension already has manifest.json
    /// 2. If not, scan local Windows Chrome / Brave / Edge extension folders
    /// 3. If not found locally, download CRX from Chrome Web Store and unpack
    pub fn ensure_sorsa_extension(workdir: &Path) -> Result<PathBuf> {
        let ext_dir = workdir.join("sorsa_extension");

        if ext_dir.exists() && ext_dir.join("manifest.json").exists() {
            info!("Sorsa extension is already installed at {:?}", ext_dir);
            return Ok(ext_dir);
        }

        info!("Sorsa extension not found in workdir. Checking local browser profiles...");
        if let Some(local_path) = Self::find_local_installed_extension() {
            info!("Found locally installed Sorsa extension at {:?}. Copying...", local_path);
            if Self::copy_dir_recursive(&local_path, &ext_dir).is_ok() {
                info!("Successfully copied Sorsa extension to {:?}", ext_dir);
                return Ok(ext_dir);
            }
        }

        info!("Downloading Sorsa extension CRX from Chrome Web Store...");
        Self::download_and_extract_crx(&ext_dir)?;
        info!("Sorsa extension successfully provisioned at {:?}", ext_dir);
        Ok(ext_dir)
    }

    /// Search local Chrome, Brave, and Edge User Data for the extension
    fn find_local_installed_extension() -> Option<PathBuf> {
        let local_app_data = env::var("LOCALAPPDATA").ok()?;
        let search_dirs = [
            Path::new(&local_app_data).join(r"Google\Chrome\User Data\Default\Extensions").join(SORSA_EXTENSION_ID),
            Path::new(&local_app_data).join(r"BraveSoftware\Brave-Browser\User Data\Default\Extensions").join(SORSA_EXTENSION_ID),
            Path::new(&local_app_data).join(r"Microsoft\Edge\User Data\Default\Extensions").join(SORSA_EXTENSION_ID),
        ];

        for base in search_dirs {
            if base.exists() {
                if let Ok(entries) = fs::read_dir(&base) {
                    let mut version_dirs: Vec<PathBuf> = entries
                        .filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| p.is_dir())
                        .collect();
                    version_dirs.sort();
                    if let Some(latest) = version_dirs.pop() {
                        if latest.join("manifest.json").exists() {
                            return Some(latest);
                        }
                    }
                }
            }
        }
        None
    }

    /// Download .crx from Google Web Store, slice off CRX header to ZIP (PK\x03\x04), and extract
    pub fn download_and_extract_crx(target_dir: &Path) -> Result<()> {
        let temp_crx = target_dir.parent().unwrap_or(Path::new(".")).join("sorsa_download_temp.crx");
        let temp_zip = target_dir.parent().unwrap_or(Path::new(".")).join("sorsa_download_temp.zip");

        // Download CRX file using PowerShell
        let dl_script = format!(
            r#"Invoke-WebRequest -Uri '{url}' -OutFile '{out}'"#,
            url = SORSA_CRX_URL,
            out = temp_crx.to_string_lossy().replace('\'', "''")
        );

        let dl_output = Command::new("powershell")
            .args(["-NoProfile", "-Command", &dl_script])
            .output()
            .with_context(|| "Failed to invoke PowerShell to download Sorsa CRX")?;

        if !dl_output.status.success() {
            let err = String::from_utf8_lossy(&dl_output.stderr);
            bail!("Failed to download CRX from Chrome Web Store: {}", err);
        }

        let crx_bytes = fs::read(&temp_crx)
            .with_context(|| format!("Failed to read downloaded CRX at {:?}", temp_crx))?;

        // Locate ZIP signature 0x50, 0x4B, 0x03, 0x04 (PK\x03\x04)
        let zip_sig = [0x50, 0x4B, 0x03, 0x04];
        let zip_offset = crx_bytes
            .windows(4)
            .position(|window| window == zip_sig)
            .context("Could not find ZIP header signature (PK\\x03\\x04) in downloaded CRX")?;

        info!("Found ZIP header offset in CRX at byte {}", zip_offset);
        let zip_bytes = &crx_bytes[zip_offset..];
        fs::write(&temp_zip, zip_bytes)
            .with_context(|| format!("Failed to write sliced ZIP to {:?}", temp_zip))?;

        if !target_dir.exists() {
            fs::create_dir_all(target_dir)?;
        }

        // Unpack ZIP using PowerShell Expand-Archive
        let unpack_script = format!(
            r#"Expand-Archive -Path '{zip}' -DestinationPath '{dest}' -Force"#,
            zip = temp_zip.to_string_lossy().replace('\'', "''"),
            dest = target_dir.to_string_lossy().replace('\'', "''")
        );

        let unpack_output = Command::new("powershell")
            .args(["-NoProfile", "-Command", &unpack_script])
            .output()
            .with_context(|| "Failed to invoke PowerShell Expand-Archive")?;

        // Clean up temporary archive files
        let _ = fs::remove_file(&temp_crx);
        let _ = fs::remove_file(&temp_zip);

        if !unpack_output.status.success() {
            let err = String::from_utf8_lossy(&unpack_output.stderr);
            bail!("Failed to unpack Sorsa extension ZIP: {}", err);
        }

        if !target_dir.join("manifest.json").exists() {
            bail!("Extracted extension folder does not contain manifest.json");
        }

        Ok(())
    }

    fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
        if !dst.exists() {
            fs::create_dir_all(dst)?;
        }
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let path = entry.path();
            let dest_path = dst.join(entry.file_name());
            if path.is_dir() {
                Self::copy_dir_recursive(&path, &dest_path)?;
            } else {
                fs::copy(&path, &dest_path)?;
            }
        }
        Ok(())
    }
}
