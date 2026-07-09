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

#[derive(Debug)]
pub struct UpstreamStream {
    pub response: Response,
    pub first: Vec<u8>,
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

fn retry_delay(attempt: usize) {
    std::thread::sleep(Duration::from_millis(800 * attempt as u64));
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
    let mut last_error = None;
    for attempt in 1..=4 {
        let mut resp = match post(cfg, body.clone()) {
            Ok(resp) => resp,
            Err(e) => {
                last_error = Some(e);
                if attempt < 4 {
                    retry_delay(attempt);
                    continue;
                }
                break;
            }
        };
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
        match resp.read_to_end(&mut body) {
            Ok(_) => {
                return Ok(UpstreamBody {
                    status,
                    content_type,
                    body,
                });
            }
            Err(e) => {
                last_error = Some(UpstreamError {
                    status: 502,
                    detail: e.to_string(),
                });
                if attempt < 4 {
                    retry_delay(attempt);
                    continue;
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| UpstreamError {
        status: 502,
        detail: "upstream request failed".to_string(),
    }))
}

fn read_first_line(resp: &mut Response) -> Result<Vec<u8>, UpstreamError> {
    let mut first = Vec::new();
    let mut byte = [0_u8; 1];
    while first.len() < 65_536 {
        match resp.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                first.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            Err(e) => {
                return Err(UpstreamError {
                    status: 502,
                    detail: e.to_string(),
                });
            }
        }
    }
    if first.is_empty() {
        return Err(UpstreamError {
            status: 502,
            detail: "upstream 200 but empty body".to_string(),
        });
    }
    Ok(first)
}

pub fn open_stream(cfg: &GatewayConfig, body: Vec<u8>) -> Result<UpstreamStream, UpstreamError> {
    let mut last_error = None;
    for attempt in 1..=4 {
        let mut resp = match post(cfg, body.clone()) {
            Ok(resp) => resp,
            Err(e) => {
                last_error = Some(e);
                if attempt < 4 {
                    retry_delay(attempt);
                    continue;
                }
                break;
            }
        };
        if !resp.status().is_success() {
            return Err(map_http_error(resp));
        }
        match read_first_line(&mut resp) {
            Ok(first) => {
                return Ok(UpstreamStream {
                    response: resp,
                    first,
                });
            }
            Err(e) => {
                last_error = Some(e);
                if attempt < 4 {
                    retry_delay(attempt);
                    continue;
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| UpstreamError {
        status: 502,
        detail: "upstream stream failed".to_string(),
    }))
}
