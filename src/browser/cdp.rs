#![allow(dead_code)]

use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::sleep;
use tokio_tungstenite::{
    connect_async,
    tungstenite::protocol::Message,
    MaybeTlsStream, WebSocketStream,
};
use tracing::{debug, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetInfo {
    pub id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub target_type: String,
    pub url: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    pub ws_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    #[serde(rename = "Browser")]
    pub browser: Option<String>,
    #[serde(rename = "Protocol-Version")]
    pub protocol_version: Option<String>,
    #[serde(rename = "User-Agent")]
    pub user_agent: Option<String>,
    #[serde(rename = "webSocketDebuggerUrl")]
    pub ws_url: Option<String>,
}

pub struct CdpSession {
    port: u16,
    target_id: String,
    ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    msg_id: AtomicU64,
}

impl CdpSession {
    /// Connect to a specific target tab on browser remote debugging port
    pub async fn connect(port: u16, target_id: Option<&str>) -> Result<Self> {
        let targets = Self::list_targets(port).await?;
        if targets.is_empty() {
            bail!("No open targets/tabs found on browser debugging port {}", port);
        }

        let selected = if let Some(tid) = target_id {
            targets
                .into_iter()
                .find(|t| t.id == tid)
                .context("Specified target id not found")?
        } else {
            // Prefer an X.com tab or Sorsa tab, or default to the first page target
            targets
                .iter()
                .find(|t| t.target_type == "page" && (t.url.contains("x.com") || t.url.contains("sorsa.io")))
                .cloned()
                .or_else(|| targets.iter().find(|t| t.target_type == "page").cloned())
                .context("No page targets found")?
        };

        let ws_url = selected
            .ws_url
            .with_context(|| format!("Target {} does not have a webSocketDebuggerUrl", selected.id))?;

        debug!("Connecting CDP WebSocket to {}", ws_url);
        let (ws_stream, _) = connect_async(&ws_url)
            .await
            .with_context(|| format!("Failed to connect to CDP WebSocket at {}", ws_url))?;

        let session = CdpSession {
            port,
            target_id: selected.id,
            ws_stream,
            msg_id: AtomicU64::new(1),
        };

        Ok(session)
    }

    /// Query /json/version endpoint
    pub async fn get_version(port: u16) -> Result<VersionInfo> {
        let url = format!("http://127.0.0.1:{}/json/version", port);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()?;
        let resp = client.get(&url).send().await
            .with_context(|| format!("Failed to connect to browser on port {}", port))?;
        let v: VersionInfo = resp.json().await?;
        Ok(v)
    }

    /// Query /json/list endpoint to get all tabs
    pub async fn list_targets(port: u16) -> Result<Vec<TargetInfo>> {
        let url = format!("http://127.0.0.1:{}/json/list", port);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()?;
        let resp = client.get(&url).send().await
            .with_context(|| format!("Failed to fetch targets from {}", url))?;
        let targets: Vec<TargetInfo> = resp.json().await?;
        Ok(targets)
    }

    /// Open a new tab with specified URL
    pub async fn new_tab(port: u16, target_url: &str) -> Result<TargetInfo> {
        let url = format!("http://127.0.0.1:{}/json/new?{}", port, target_url);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()?;
        let resp = client.put(&url).send().await
            .with_context(|| format!("Failed to open new tab at {}", target_url))?;
        let target: TargetInfo = resp.json().await?;
        Ok(target)
    }

    /// Send a CDP request and await the corresponding response
    pub async fn call_method(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let id = self.msg_id.fetch_add(1, Ordering::SeqCst);
        let payload = json!({
            "id": id,
            "method": method,
            "params": params
        });

        let text = payload.to_string();
        self.ws_stream.send(Message::Text(text)).await
            .context("Failed to send CDP WebSocket frame")?;

        // Await matching response ID with 15s timeout
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        while tokio::time::Instant::now() < deadline {
            tokio::select! {
                Some(msg_res) = self.ws_stream.next() => {
                    let msg = msg_res.context("Error receiving CDP frame")?;
                    if let Message::Text(txt) = msg {
                        let parsed: serde_json::Value = serde_json::from_str(&txt)?;
                        if parsed.get("id").and_then(|v| v.as_u64()) == Some(id) {
                            if let Some(err) = parsed.get("error") {
                                bail!("CDP error in {}: {}", method, err);
                            }
                            return Ok(parsed.get("result").cloned().unwrap_or(json!({})));
                        }
                    }
                }
                _ = sleep(Duration::from_millis(50)) => {}
            }
        }

        bail!("Timeout waiting for CDP response for method {}", method);
    }

    /// Evaluate a JavaScript expression in the tab's DOM
    pub async fn eval_js(&mut self, script: &str) -> Result<serde_json::Value> {
        let res = self
            .call_method(
                "Runtime.evaluate",
                json!({
                    "expression": script,
                    "returnByValue": true,
                    "awaitPromise": true
                }),
            )
            .await?;

        if let Some(exc) = res.get("exceptionDetails") {
            bail!("JS Evaluation Exception: {}", exc);
        }

        let val = res.get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        Ok(val)
    }

    /// Navigate current tab to a URL and wait for load
    pub async fn navigate(&mut self, url: &str) -> Result<()> {
        info!("Navigating browser tab to {}", url);
        self.call_method("Page.navigate", json!({ "url": url })).await?;
        sleep(Duration::from_millis(1500)).await;
        Ok(())
    }

    /// Wait until a CSS selector exists in the page DOM
    pub async fn wait_for_selector(&mut self, selector: &str, timeout_secs: u64) -> Result<bool> {
        let js = format!(
            r#"document.querySelector('{}') !== null"#,
            selector.replace('\'', "\\'")
        );
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(timeout_secs) {
            if let Ok(val) = self.eval_js(&js).await {
                if val.as_bool() == Some(true) {
                    return Ok(true);
                }
            }
            sleep(Duration::from_millis(500)).await;
        }
        Ok(false)
    }

    /// Click an element matching a CSS selector
    pub async fn click_selector(&mut self, selector: &str) -> Result<bool> {
        let js = format!(
            r#"(() => {{
                const el = document.querySelector('{}');
                if (!el) return false;
                el.scrollIntoView({{ behavior: 'instant', block: 'center' }});
                el.click();
                return true;
            }})()"#,
            selector.replace('\'', "\\'")
        );
        let res = self.eval_js(&js).await?;
        Ok(res.as_bool() == Some(true))
    }

    /// Type text into an input or textarea
    pub async fn type_into_selector(&mut self, selector: &str, text: &str) -> Result<bool> {
        let js = format!(
            r#"(() => {{
                const el = document.querySelector('{}');
                if (!el) return false;
                el.focus();
                document.execCommand('insertText', false, {});
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                return true;
            }})()"#,
            selector.replace('\'', "\\'"),
            serde_json::to_string(text)?
        );
        let res = self.eval_js(&js).await?;
        Ok(res.as_bool() == Some(true))
    }

    pub fn target_id(&self) -> &str {
        &self.target_id
    }
}
