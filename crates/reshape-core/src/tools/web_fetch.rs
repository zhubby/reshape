use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::config::WebFetchConfig;
use crate::error::{ReshapeError, Result};

use super::{Tool, ToolContext, ToolDefinition, ToolResult};

#[derive(Debug, Clone)]
pub struct WebFetchTool {
    config: WebFetchConfig,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct FetchArgs {
    url: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    overwrite: Option<bool>,
    #[serde(default)]
    max_bytes: Option<usize>,
    #[serde(default)]
    expected_content_type: Option<String>,
}

struct FetchResponse {
    url: String,
    final_url: String,
    redirects: Vec<String>,
    content_type: String,
    bytes: Vec<u8>,
}

impl WebFetchTool {
    pub fn new(config: WebFetchConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| ReshapeError::Config(format!("build web_fetch client: {error}")))?;
        Ok(Self { config, client })
    }

    fn parse_args(args: Value) -> Result<std::result::Result<FetchArgs, ToolResult>> {
        let args = match serde_json::from_value::<FetchArgs>(args) {
            Ok(args) => args,
            Err(error) => {
                return Ok(Err(Self::error_result(
                    "invalid_args",
                    format!("invalid arguments: {error}"),
                    None,
                )?));
            }
        };
        if args.url.trim().is_empty() {
            return Ok(Err(Self::error_result(
                "invalid_args",
                "`url` cannot be empty",
                None,
            )?));
        }
        if args.max_bytes.is_some_and(|value| value == 0) {
            return Ok(Err(Self::error_result(
                "invalid_args",
                "`max_bytes` must be greater than 0",
                None,
            )?));
        }
        Ok(Ok(args))
    }

    fn error_result(
        code: &str,
        error: impl Into<String>,
        status: Option<u16>,
    ) -> Result<ToolResult> {
        let mut content = json!({
            "tool": "web_fetch",
            "success": false,
            "recoverable": true,
            "code": code,
            "error": error.into(),
        });
        if let Some(status) = status {
            content["status"] = json!(status);
        }
        ToolResult::json(content, None)
    }

    async fn fetch_url(&self, raw_url: &str, max_bytes: usize) -> Result<FetchResponse> {
        let original_url = raw_url.trim().to_string();
        let mut current_url = reqwest::Url::parse(&original_url)
            .map_err(|error| ReshapeError::Config(format!("invalid url: {error}")))?;
        match current_url.scheme() {
            "http" | "https" => {}
            scheme => {
                return Err(ReshapeError::Config(format!(
                    "unsupported URL scheme: {scheme}"
                )));
            }
        }

        let mut redirects = Vec::new();
        for _ in 0..=self.config.max_redirects {
            self.check_ssrf(&current_url).await?;
            let response = self
                .client
                .get(current_url.clone())
                .send()
                .await
                .map_err(|error| {
                    ReshapeError::Provider(format!("web_fetch request failed: {error}"))
                })?;
            let status = response.status();
            if status.is_redirection() {
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| {
                        ReshapeError::Provider("redirect missing Location".to_string())
                    })?;
                redirects.push(current_url.to_string());
                current_url = current_url.join(location).map_err(|error| {
                    ReshapeError::Provider(format!("invalid redirect location: {error}"))
                })?;
                continue;
            }
            if !status.is_success() {
                return Err(ReshapeError::Provider(format!(
                    "web_fetch request failed with status {status}"
                )));
            }
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .map(normalize_content_type)
                .unwrap_or_default();
            if let Some(content_length) = response.content_length()
                && content_length as usize > max_bytes
            {
                return Err(ReshapeError::Provider(format!(
                    "download exceeds max_bytes ({content_length} > {max_bytes})"
                )));
            }

            let mut bytes = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|error| {
                    ReshapeError::Provider(format!("failed reading response body: {error}"))
                })?;
                if bytes.len() + chunk.len() > max_bytes {
                    return Err(ReshapeError::Provider(format!(
                        "download exceeds max_bytes ({max_bytes})"
                    )));
                }
                bytes.extend_from_slice(&chunk);
            }

            return Ok(FetchResponse {
                url: original_url,
                final_url: current_url.to_string(),
                redirects,
                content_type,
                bytes,
            });
        }

        Err(ReshapeError::Provider(format!(
            "too many redirects (max {})",
            self.config.max_redirects
        )))
    }

    async fn check_ssrf(&self, url: &reqwest::Url) -> Result<()> {
        let host = url
            .host_str()
            .ok_or_else(|| ReshapeError::Config("URL has no host".to_string()))?;
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_private_ip(&ip) && !is_allowlisted(&ip, &self.config.ssrf_allowlist) {
                return Err(ReshapeError::Provider(format!(
                    "SSRF blocked: {host} resolves to private IP {ip}"
                )));
            }
            return Ok(());
        }

        let port = url.port_or_known_default().unwrap_or(443);
        let addrs = tokio::net::lookup_host(format!("{host}:{port}"))
            .await
            .map_err(|error| ReshapeError::Provider(format!("DNS lookup failed: {error}")))?
            .collect::<Vec<_>>();
        if addrs.is_empty() {
            return Err(ReshapeError::Provider(format!(
                "DNS resolution failed for {host}"
            )));
        }
        for addr in addrs {
            let ip = addr.ip();
            if is_private_ip(&ip) && !is_allowlisted(&ip, &self.config.ssrf_allowlist) {
                return Err(ReshapeError::Provider(format!(
                    "SSRF blocked: {host} resolves to private IP {ip}"
                )));
            }
        }
        Ok(())
    }

    fn validate_content_type(
        &self,
        content_type: &str,
        expected_content_type: Option<&str>,
    ) -> Result<std::result::Result<(), ToolResult>> {
        if let Some(expected) = expected_content_type.map(normalize_content_type)
            && expected != content_type
        {
            return Ok(Err(Self::error_result(
                "unexpected_content_type",
                format!("expected content type `{expected}`, got `{content_type}`"),
                None,
            )?));
        }

        if !self
            .config
            .allowed_content_types
            .iter()
            .any(|allowed| allowed == content_type)
        {
            return Ok(Err(Self::error_result(
                "unsupported_content_type",
                format!("unsupported content type `{content_type}`"),
                None,
            )?));
        }
        Ok(Ok(()))
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "web_fetch".to_string(),
            description: "Download an internet media or binary resource into the configured workspace and return saved-file metadata.".to_string(),
            parameters: json!({
                "type": "object",
                "description": "Download one HTTP/HTTPS media resource to the workspace. This tool does not extract webpage text.",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "HTTP/HTTPS URL for an image, video, audio file, or PDF."
                    },
                    "path": {
                        "type": "string",
                        "description": "Optional workspace-relative destination path. Defaults under configured download_dir."
                    },
                    "overwrite": {
                        "type": "boolean",
                        "description": "Whether to overwrite an existing workspace file. Defaults to false.",
                        "default": false
                    },
                    "max_bytes": {
                        "type": "integer",
                        "description": "Maximum bytes to download. Defaults to configured max_bytes.",
                        "minimum": 1
                    },
                    "expected_content_type": {
                        "type": "string",
                        "description": "Optional expected MIME type, for example image/png."
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value, context: &ToolContext) -> Result<ToolResult> {
        let args = match Self::parse_args(args)? {
            Ok(args) => args,
            Err(result) => return Ok(result),
        };
        let max_bytes = args.max_bytes.unwrap_or(self.config.max_bytes);
        let fetched = match self.fetch_url(&args.url, max_bytes).await {
            Ok(fetched) => fetched,
            Err(error) => return Self::error_result("request_failed", error.to_string(), None),
        };
        match self
            .validate_content_type(&fetched.content_type, args.expected_content_type.as_deref())?
        {
            Ok(()) => {}
            Err(result) => return Ok(result),
        }

        let sha256 = sha256_hex(&fetched.bytes);
        let path = args.path.unwrap_or_else(|| {
            default_download_path(
                &self.config.download_dir,
                &fetched.final_url,
                &fetched.content_type,
                &sha256,
            )
        });
        if !extension_matches_content_type(&path, &fetched.content_type) {
            return Self::error_result(
                "extension_mismatch",
                format!(
                    "destination extension does not match content type `{}`",
                    fetched.content_type
                ),
                None,
            );
        }
        if !args.overwrite.unwrap_or(false)
            && context
                .workspace
                .list_files()
                .await?
                .iter()
                .any(|existing| existing == Path::new(&path))
        {
            return Self::error_result(
                "file_exists",
                format!("workspace file `{path}` already exists"),
                None,
            );
        }

        let written_path = context.workspace.write_bytes(&path, &fetched.bytes).await?;
        let path = written_path.to_string_lossy().to_string();
        ToolResult::json(
            json!({
                "tool": "web_fetch",
                "success": true,
                "url": fetched.url,
                "final_url": fetched.final_url,
                "path": path,
                "content_type": fetched.content_type,
                "bytes_written": fetched.bytes.len(),
                "sha256": sha256,
                "redirects": fetched.redirects,
                "cached": false,
                "suggested_html": suggested_html(&path, &fetched.content_type),
            }),
            Some(format!("Downloaded {path}")),
        )
    }
}

fn normalize_content_type(value: impl AsRef<str>) -> String {
    value
        .as_ref()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn default_download_path(
    download_dir: &str,
    url: &str,
    content_type: &str,
    sha256: &str,
) -> String {
    let stem = reqwest::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(Iterator::last)
                .map(ToString::to_string)
        })
        .and_then(|name| {
            Path::new(&name)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(ToString::to_string)
        })
        .map(|stem| sanitize_stem(&stem))
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "download".to_string());
    let extension = extension_for_content_type(content_type).unwrap_or("bin");
    format!(
        "{}/{}-{}.{}",
        download_dir.trim_end_matches('/'),
        stem,
        &sha256[..8],
        extension
    )
}

fn sanitize_stem(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn extension_for_content_type(content_type: &str) -> Option<&'static str> {
    match content_type {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "video/mp4" => Some("mp4"),
        "video/webm" => Some("webm"),
        "audio/mpeg" => Some("mp3"),
        "audio/wav" => Some("wav"),
        "application/pdf" => Some("pdf"),
        _ => None,
    }
}

fn extension_matches_content_type(path: &str, content_type: &str) -> bool {
    let Some(extension) = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
    else {
        return false;
    };
    match content_type {
        "image/jpeg" => matches!(extension.as_str(), "jpg" | "jpeg"),
        _ => extension_for_content_type(content_type).is_some_and(|expected| extension == expected),
    }
}

fn suggested_html(path: &str, content_type: &str) -> Option<String> {
    match content_type {
        value if value.starts_with("image/") => Some(format!(r#"<img src="{path}" alt="">"#)),
        value if value.starts_with("video/") => {
            Some(format!(r#"<video src="{path}" controls></video>"#))
        }
        value if value.starts_with("audio/") => {
            Some(format!(r#"<audio src="{path}" controls></audio>"#))
        }
        "application/pdf" => Some(format!(r#"<a href="{path}">Download PDF</a>"#)),
        _ => None,
    }
}

fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(value) => {
            value.is_loopback()
                || value.is_private()
                || value.is_link_local()
                || value.is_broadcast()
                || value.is_unspecified()
        }
        IpAddr::V6(value) => {
            value.is_loopback()
                || value.is_unspecified()
                || is_unique_local_v6(value)
                || is_unicast_link_local_v6(value)
        }
    }
}

fn is_unique_local_v6(value: &Ipv6Addr) -> bool {
    (value.segments()[0] & 0xfe00) == 0xfc00
}

fn is_unicast_link_local_v6(value: &Ipv6Addr) -> bool {
    (value.segments()[0] & 0xffc0) == 0xfe80
}

fn is_allowlisted(ip: &IpAddr, allowlist: &[String]) -> bool {
    allowlist.iter().any(|entry| match ip {
        IpAddr::V4(ip) => allowlist_v4_contains(entry, ip),
        IpAddr::V6(ip) => allowlist_v6_contains(entry, ip),
    })
}

fn allowlist_v4_contains(entry: &str, ip: &Ipv4Addr) -> bool {
    let Some((base, prefix)) = entry.split_once('/') else {
        return entry.parse::<Ipv4Addr>().is_ok_and(|base| base == *ip);
    };
    let Ok(base) = base.parse::<Ipv4Addr>() else {
        return false;
    };
    let Ok(prefix) = prefix.parse::<u32>() else {
        return false;
    };
    if prefix > 32 {
        return false;
    }
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    u32::from(base) & mask == u32::from(*ip) & mask
}

fn allowlist_v6_contains(entry: &str, ip: &Ipv6Addr) -> bool {
    let Some((base, prefix)) = entry.split_once('/') else {
        return entry.parse::<Ipv6Addr>().is_ok_and(|base| base == *ip);
    };
    let Ok(base) = base.parse::<Ipv6Addr>() else {
        return false;
    };
    let Ok(prefix) = prefix.parse::<u32>() else {
        return false;
    };
    if prefix > 128 {
        return false;
    }
    let base = u128::from_be_bytes(base.octets());
    let ip = u128::from_be_bytes(ip.octets());
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix)
    };
    base & mask == ip & mask
}
