use crate::extension::SorsaExtensionProvisioner;
use anyhow::{bail, Context, Result};
use std::env;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserType {
    Chrome,
    Brave,
    Edge,
    Firefox,
}

impl BrowserType {
    pub fn from_str(s: &str) -> Option<Self> {
        let lower = s.to_lowercase();
        if lower.contains("chrome") {
            Some(BrowserType::Chrome)
        } else if lower.contains("brave") {
            Some(BrowserType::Brave)
        } else if lower.contains("edge") {
            Some(BrowserType::Edge)
        } else if lower.contains("firefox") {
            Some(BrowserType::Firefox)
        } else {
            None
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            BrowserType::Chrome => "Google Chrome",
            BrowserType::Brave => "Brave Browser",
            BrowserType::Edge => "Microsoft Edge",
            BrowserType::Firefox => "Mozilla Firefox",
        }
    }

    pub fn exe_name(&self) -> &'static str {
        match self {
            BrowserType::Chrome => "chrome.exe",
            BrowserType::Brave => "brave.exe",
            BrowserType::Edge => "msedge.exe",
            BrowserType::Firefox => "firefox.exe",
        }
    }
}

pub struct BrowserShortcutManager;

impl BrowserShortcutManager {
    /// Detect standard installation paths on Windows for a given browser type
    pub fn detect_browser_path(browser: BrowserType) -> Option<PathBuf> {
        let local_app_data = env::var("LOCALAPPDATA").unwrap_or_default();
        let program_files = env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".to_string());
        let program_files_x86 = env::var("ProgramFiles(x86)").unwrap_or_else(|_| "C:\\Program Files (x86)".to_string());

        let candidates: Vec<PathBuf> = match browser {
            BrowserType::Chrome => vec![
                Path::new(&program_files).join("Google\\Chrome\\Application\\chrome.exe"),
                Path::new(&program_files_x86).join("Google\\Chrome\\Application\\chrome.exe"),
                Path::new(&local_app_data).join("Google\\Chrome\\Application\\chrome.exe"),
            ],
            BrowserType::Brave => vec![
                Path::new(&program_files).join("BraveSoftware\\Brave-Browser\\Application\\brave.exe"),
                Path::new(&program_files_x86).join("BraveSoftware\\Brave-Browser\\Application\\brave.exe"),
                Path::new(&local_app_data).join("BraveSoftware\\Brave-Browser\\Application\\brave.exe"),
            ],
            BrowserType::Edge => vec![
                Path::new(&program_files_x86).join("Microsoft\\Edge\\Application\\msedge.exe"),
                Path::new(&program_files).join("Microsoft\\Edge\\Application\\msedge.exe"),
                Path::new(&local_app_data).join("Microsoft\\Edge\\Application\\msedge.exe"),
            ],
            BrowserType::Firefox => vec![
                Path::new(&program_files).join("Mozilla Firefox\\firefox.exe"),
                Path::new(&program_files_x86).join("Mozilla Firefox\\firefox.exe"),
                Path::new(&local_app_data).join("Mozilla Firefox\\firefox.exe"),
            ],
        };

        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }

        // Check Windows Registry App Paths as fallback
        Self::detect_from_registry(browser.exe_name())
    }

    /// Query registry App Paths for the executable
    fn detect_from_registry(exe_name: &str) -> Option<PathBuf> {
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    r#"(Get-ItemProperty "HKLM:\Software\Microsoft\Windows\CurrentVersion\App Paths\{}" -ErrorAction SilentlyContinue).'(Default)'"#,
                    exe_name
                ),
            ])
            .output()
            .ok()?;

        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_str.is_empty() {
                let p = PathBuf::from(path_str);
                if p.exists() {
                    return Some(p);
                }
            }
        }
        None
    }

    /// Scan and detect all installed supported browsers on this machine
    pub fn detect_all_installed() -> Vec<(BrowserType, PathBuf)> {
        let mut list = Vec::new();
        for b in [BrowserType::Chrome, BrowserType::Brave, BrowserType::Edge, BrowserType::Firefox] {
            if let Some(p) = Self::detect_browser_path(b) {
                list.push((b, p));
            }
        }
        list
    }

    /// Check if TCP port is completely free to bind
    pub fn is_port_available(port: u16) -> bool {
        TcpListener::bind(("127.0.0.1", port)).is_ok()
    }

    /// Check if port is running a valid browser CDP instance (responds on /json/version)
    pub fn is_cdp_endpoint(port: u16) -> bool {
        let addr = format!("127.0.0.1:{}", port);
        if let Ok(mut stream) = TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(500)) {
            let req = format!("GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", port);
            if stream.write_all(req.as_bytes()).is_ok() {
                let mut buf = [0u8; 256];
                if let Ok(n) = stream.read(&mut buf) {
                    let s = String::from_utf8_lossy(&buf[..n]);
                    return s.contains("200 OK") || s.contains("Browser") || s.contains("webSocketDebuggerUrl");
                }
            }
        }
        false
    }

    /// Check if debugging port is active on 127.0.0.1
    pub fn is_port_active(port: u16) -> bool {
        let addr = format!("127.0.0.1:{}", port);
        TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(500)).is_ok()
    }

    /// Intelligently resolve port conflicts:
    /// 1. If preferred_port is already running our browser CDP -> reuse it!
    /// 2. If preferred_port is completely free to bind -> use it!
    /// 3. If preferred_port is occupied by a conflict (another app) -> auto-increment to find next free port!
    pub fn resolve_port(preferred_port: u16) -> u16 {
        if Self::is_cdp_endpoint(preferred_port) {
            info!("Port {} has an active browser CDP session. Reusing.", preferred_port);
            return preferred_port;
        }

        if Self::is_port_available(preferred_port) {
            return preferred_port;
        }

        warn!(
            "PORT CONFLICT: Port {} is in use by another non-CDP process. Searching for available fallback port...",
            preferred_port
        );

        for offset in 1..=50 {
            let candidate = preferred_port + offset;
            if Self::is_cdp_endpoint(candidate) {
                info!("Found active browser CDP on fallback port {}. Reusing.", candidate);
                return candidate;
            }
            if Self::is_port_available(candidate) {
                info!("Resolved port conflict: Using available port {}.", candidate);
                return candidate;
            }
        }

        preferred_port
    }

    /// Format debugging command line arguments for the browser including --load-extension
    pub fn get_debugging_arguments(
        browser: BrowserType,
        port: u16,
        profile_dir: &Path,
        extension_dir: &Path,
    ) -> String {
        let prof_str = profile_dir.to_string_lossy();
        let ext_str = extension_dir.to_string_lossy();

        match browser {
            BrowserType::Chrome | BrowserType::Brave | BrowserType::Edge => {
                format!(
                    r#"--remote-debugging-port={} --user-data-dir="{}" --load-extension="{}" --no-first-run --no-default-browser-check"#,
                    port, prof_str, ext_str
                )
            }
            BrowserType::Firefox => {
                format!(
                    r#"-marionette -start-debugger-server {} --remote-debugging-port {} -profile "{}""#,
                    port, port, prof_str
                )
            }
        }
    }

    /// Automatically provision Sorsa extension and generate a Windows .lnk shortcut file in workdir
    pub fn create_shortcut(
        browser: BrowserType,
        port: u16,
        workdir: &Path,
        custom_shortcut_name: Option<&str>,
    ) -> Result<PathBuf> {
        let exe_path = Self::detect_browser_path(browser)
            .with_context(|| format!("Browser '{}' is not installed or not found in standard paths", browser.display_name()))?;

        // 1. Provision Sorsa extension
        let ext_dir = SorsaExtensionProvisioner::ensure_sorsa_extension(workdir)?;

        // 2. Prepare profile directory
        let profile_dir = workdir.join("sorsa_browser_profile");
        if !profile_dir.exists() {
            std::fs::create_dir_all(&profile_dir)
                .with_context(|| format!("Failed to create browser profile dir at {:?}", profile_dir))?;
        }

        let default_name = format!("Sorsa-{}-Debug.lnk", browser.display_name().replace(' ', ""));
        let shortcut_name = custom_shortcut_name.unwrap_or(&default_name);
        let shortcut_path = workdir.join(shortcut_name);

        let args = Self::get_debugging_arguments(browser, port, &profile_dir, &ext_dir);
        let exe_str = exe_path.to_string_lossy();
        let sc_str = shortcut_path.to_string_lossy();
        let work_str = workdir.to_string_lossy();

        // Use PowerShell WScript.Shell to generate the .lnk shortcut
        let script = format!(
            r#"
            $sh = New-Object -ComObject WScript.Shell
            $sc = $sh.CreateShortcut('{sc}')
            $sc.TargetPath = '{exe}'
            $sc.Arguments = '{args}'
            $sc.WorkingDirectory = '{work}'
            $sc.Description = 'Sorsa CRM Watcher Automated Browser Debug Session with Sorsa Extension'
            $sc.Save()
            "#,
            sc = sc_str.replace('\'', "''"),
            exe = exe_str.replace('\'', "''"),
            args = args.replace('\'', "''"),
            work = work_str.replace('\'', "''"),
        );

        let output = Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
            .with_context(|| "Failed to execute PowerShell to create shortcut")?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            bail!("PowerShell shortcut creation failed: {}", err);
        }

        info!("Created debug shortcut at {:?} (Port: {})", shortcut_path, port);
        Ok(shortcut_path)
    }

    /// Auto-launch browser if port is not yet active (with auto port conflict resolution)
    pub fn ensure_browser_running(
        browser: BrowserType,
        requested_port: u16,
        workdir: &Path,
    ) -> Result<(PathBuf, u16)> {
        let port = Self::resolve_port(requested_port);
        let sc_path = Self::create_shortcut(browser, port, workdir, None)?;

        if Self::is_cdp_endpoint(port) {
            info!("Browser debugging session is already active on port {}", port);
            return Ok((sc_path, port));
        }

        info!(
            "Spawning {} on port {} in background with Sorsa extension...",
            browser.display_name(),
            port
        );

        let ext_dir = workdir.join("sorsa_extension");
        let profile_dir = workdir.join("sorsa_browser_profile");
        let exe_path = Self::detect_browser_path(browser).context("Browser executable not found")?;
        let args = Self::get_debugging_arguments(browser, port, &profile_dir, &ext_dir);

        // Spawn process directly in background without blocking
        let mut cmd = Command::new(&exe_path);
        for arg in args.split_whitespace() {
            cmd.arg(arg.trim_matches('"'));
        }
        cmd.current_dir(workdir);

        let child = cmd.spawn().with_context(|| format!("Failed to spawn browser process {:?}", exe_path))?;
        info!("Browser process spawned (PID: {}). Waiting for port {}...", child.id(), port);

        // Wait up to 10 seconds for the debugging port to become active
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(500));
            if Self::is_port_active(port) {
                info!("Debugging port {} is now active and ready!", port);
                return Ok((sc_path, port));
            }
        }

        warn!("Browser spawned, waiting for port {} to finish initializing.", port);
        Ok((sc_path, port))
    }

    /// Read and inspect an existing Windows .lnk shortcut file
    pub fn inspect_shortcut(lnk_path: &Path) -> Result<(String, String, String)> {
        let p_str = lnk_path.to_string_lossy();
        let script = format!(
            r#"
            $sh = New-Object -ComObject WScript.Shell
            $sc = $sh.CreateShortcut('{path}')
            [PSCustomObject]@{{
                TargetPath = $sc.TargetPath
                Arguments = $sc.Arguments
                WorkingDirectory = $sc.WorkingDirectory
            }} | ConvertTo-Json
            "#,
            path = p_str.replace('\'', "''")
        );

        let output = Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
            .with_context(|| "Failed to inspect shortcut via PowerShell")?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to read shortcut: {}", err);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let v: serde_json::Value = serde_json::from_str(&stdout)
            .with_context(|| format!("Failed to parse shortcut JSON: {}", stdout))?;

        let target = v["TargetPath"].as_str().unwrap_or_default().to_string();
        let args = v["Arguments"].as_str().unwrap_or_default().to_string();
        let work = v["WorkingDirectory"].as_str().unwrap_or_default().to_string();

        Ok((target, args, work))
    }
}
