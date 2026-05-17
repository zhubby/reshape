use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::WebSearchConfig;
use crate::error::{ReshapeError, Result};

use super::{Tool, ToolContext, ToolDefinition, ToolResult};

const MAX_RESULTS_LIMIT: usize = 20;

#[derive(Debug, Clone)]
pub struct WebSearchTool {
    config: WebSearchConfig,
    client: reqwest::Client,
    api_key: String,
}

#[derive(Debug, Deserialize)]
struct SearchArgs {
    query: String,
    #[serde(default)]
    max_results: Option<usize>,
    #[serde(default)]
    search_depth: Option<String>,
    #[serde(default)]
    topic: Option<String>,
    #[serde(default)]
    time_range: Option<String>,
    #[serde(default)]
    include_domains: Option<Vec<String>>,
    #[serde(default)]
    exclude_domains: Option<Vec<String>>,
    #[serde(default)]
    include_answer: Option<bool>,
    #[serde(default)]
    include_images: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct TavilyResponse {
    #[serde(default)]
    answer: Option<Value>,
    #[serde(default)]
    results: Vec<TavilyResult>,
    #[serde(default)]
    images: Vec<Value>,
    #[serde(default)]
    response_time: Option<Value>,
    #[serde(default)]
    request_id: Option<Value>,
    #[serde(default)]
    usage: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct TavilyResult {
    title: String,
    url: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    score: Option<Value>,
    #[serde(default)]
    favicon: Option<Value>,
    #[serde(default)]
    images: Option<Value>,
}

impl WebSearchTool {
    pub fn new(config: WebSearchConfig) -> Result<Self> {
        if config.provider.trim() != "tavily" {
            return Err(ReshapeError::Config(format!(
                "unsupported web_search provider: {}",
                config.provider
            )));
        }
        let api_key = resolve_tavily_api_key(&config)?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.tavily.timeout_secs))
            .build()
            .map_err(|error| ReshapeError::Config(format!("build Tavily client: {error}")))?;
        Ok(Self {
            config,
            client,
            api_key,
        })
    }

    fn parse_args(args: Value) -> Result<std::result::Result<SearchArgs, ToolResult>> {
        let args = match serde_json::from_value::<SearchArgs>(args) {
            Ok(args) => args,
            Err(error) => {
                return Ok(Err(Self::error_result(
                    "invalid_args",
                    format!("invalid arguments: {error}"),
                    None,
                )?));
            }
        };
        if args.query.trim().is_empty() {
            return Ok(Err(Self::error_result(
                "invalid_args",
                "`query` cannot be empty",
                None,
            )?));
        }
        if args.max_results.is_some_and(|value| value == 0) {
            return Ok(Err(Self::error_result(
                "invalid_args",
                "`max_results` must be greater than 0",
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
            "tool": "web_search",
            "provider": "tavily",
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
}

fn resolve_tavily_api_key(config: &WebSearchConfig) -> Result<String> {
    let configured = config.tavily.api_key.trim();
    if !configured.is_empty() {
        return Ok(configured.to_string());
    }

    let env_key = config.tavily.env_key.trim();
    if !env_key.is_empty()
        && let Ok(value) = std::env::var(env_key)
        && !value.trim().is_empty()
    {
        return Ok(value);
    }

    Err(ReshapeError::Config(
        "Tavily web_search requires tools.web_search.tavily.api_key or a configured env_key"
            .to_string(),
    ))
}

#[async_trait]
impl Tool for WebSearchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "web_search".to_string(),
            description: "Search the public web through Tavily for current external information and return structured source metadata.".to_string(),
            parameters: json!({
                "type": "object",
                "description": "Search Tavily for web results. Use focused queries with entities, versions, dates, or domains when possible.",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural-language search query.",
                        "minLength": 1,
                        "examples": ["Rust reqwest timeout docs", "Tavily Search API max_results"]
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum results to return. Defaults to configured Tavily max_results and is clamped to 20.",
                        "minimum": 1,
                        "maximum": 20,
                        "default": self.config.tavily.max_results
                    },
                    "search_depth": {
                        "type": "string",
                        "enum": ["basic", "advanced"],
                        "description": "Tavily search depth. Defaults to configured value."
                    },
                    "topic": {
                        "type": "string",
                        "description": "Tavily topic such as general or news. Defaults to configured value."
                    },
                    "time_range": {
                        "type": "string",
                        "description": "Optional Tavily time range for recency filtering."
                    },
                    "include_domains": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Optional domains to restrict results to."
                    },
                    "exclude_domains": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Optional domains to exclude."
                    },
                    "include_answer": {
                        "type": "boolean",
                        "description": "Whether Tavily should include an answer summary."
                    },
                    "include_images": {
                        "type": "boolean",
                        "description": "Whether Tavily should include image results."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value, _context: &ToolContext) -> Result<ToolResult> {
        let args = match Self::parse_args(args)? {
            Ok(args) => args,
            Err(result) => return Ok(result),
        };
        let max_results = args
            .max_results
            .unwrap_or(self.config.tavily.max_results)
            .min(MAX_RESULTS_LIMIT);
        let query = args.query.trim().to_string();

        let mut body = json!({
            "query": query,
            "max_results": max_results,
            "search_depth": args.search_depth.unwrap_or_else(|| self.config.tavily.search_depth.clone()),
            "topic": args.topic.unwrap_or_else(|| self.config.tavily.topic.clone()),
            "include_answer": args.include_answer.unwrap_or(self.config.tavily.include_answer),
            "include_images": args.include_images.unwrap_or(self.config.tavily.include_images),
            "include_favicon": self.config.tavily.include_favicon,
        });
        if let Some(time_range) = args.time_range {
            body["time_range"] = json!(time_range);
        }
        if let Some(include_domains) = args.include_domains {
            body["include_domains"] = json!(include_domains);
        }
        if let Some(exclude_domains) = args.exclude_domains {
            body["exclude_domains"] = json!(exclude_domains);
        }

        let url = format!(
            "{}/search",
            self.config.tavily.base_url.trim_end_matches('/')
        );
        let response = match self
            .client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                return Self::error_result(
                    "request_failed",
                    format!("Tavily request failed: {error}"),
                    None,
                );
            }
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Self::error_result(
                "http_error",
                format!("Tavily request failed with status {status}: {body}"),
                Some(status.as_u16()),
            );
        }

        let payload = match response.json::<TavilyResponse>().await {
            Ok(payload) => payload,
            Err(error) => {
                return Self::error_result(
                    "invalid_response",
                    format!("invalid Tavily response: {error}"),
                    None,
                );
            }
        };
        let results = payload
            .results
            .into_iter()
            .map(|item| {
                json!({
                    "title": item.title,
                    "url": item.url,
                    "content": item.content.unwrap_or_default(),
                    "score": item.score,
                    "favicon": item.favicon,
                    "images": item.images,
                })
            })
            .collect::<Vec<_>>();

        ToolResult::json(
            json!({
                "tool": "web_search",
                "provider": "tavily",
                "success": true,
                "query": query,
                "answer": payload.answer,
                "results": results,
                "images": payload.images,
                "response_time": payload.response_time,
                "request_id": payload.request_id,
                "usage": payload.usage,
                "truncated": false,
            }),
            Some(format!(
                "Searched Tavily for `{}`",
                body["query"].as_str().unwrap_or("")
            )),
        )
    }
}
