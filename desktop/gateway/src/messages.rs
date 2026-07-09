use std::io::Read;
use std::time::Duration;

use reqwest::blocking::{Client, Response};

use crate::config::{GatewayConfig, UPSTREAM_UA};

#[derive(Debug)]
pub struct UpstreamBody {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub struct UpstreamError {
    pub status: u16,
    pub detail: String,
}

fn client() -> Result<Client, UpstreamError> {
    Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| UpstreamError {
            status: 502,
            detail: e.to_string(),
        })
}

fn post(cfg: &GatewayConfig, body: Vec<u8>) -> Result<Response, UpstreamError> {
    client()?
        .post(&cfg.upstream_url)
        .header("content-type", "application/json")
        .header("anthropic-version", "2023-06-01")
        .header("user-agent", UPSTREAM_UA)
        .header("x-api-key", &cfg.api_key)
        .body(body)
        .send()
        .map_err(|e| UpstreamError {
            status: 502,
            detail: e.to_string(),
        })
}

fn map_http_error(resp: Response) -> UpstreamError {
    let status = resp.status().as_u16();
    let body = resp.text().unwrap_or_default();
    let mapped = if matches!(status, 401 | 403 | 429) {
        status
    } else {
        502
    };
    let detail = if body.is_empty() {
        format!("upstream {status}")
    } else {
        format!("upstream {status}: {body}")
    };
    UpstreamError {
        status: mapped,
        detail,
    }
}

pub fn post_nonstream(cfg: &GatewayConfig, body: Vec<u8>) -> Result<UpstreamBody, UpstreamError> {
    let mut resp = post(cfg, body)?;
    if !resp.status().is_success() {
        return Err(map_http_error(resp));
    }
    let status = resp.status().as_u16();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    let mut body = Vec::new();
    resp.read_to_end(&mut body).map_err(|e| UpstreamError {
        status: 502,
        detail: e.to_string(),
    })?;
    Ok(UpstreamBody {
        status,
        content_type,
        body,
    })
}

pub fn open_stream(cfg: &GatewayConfig, body: Vec<u8>) -> Result<Response, UpstreamError> {
    let resp = post(cfg, body)?;
    if !resp.status().is_success() {
        return Err(map_http_error(resp));
    }
    Ok(resp)
}
