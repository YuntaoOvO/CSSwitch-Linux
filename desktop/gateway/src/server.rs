use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use serde_json::{json, Value};

use crate::auth::{strip_path_secret, AuthResult};
use crate::config::GatewayConfig;
use crate::{connect, messages, models, policy};

struct RequestHead {
    method: String,
    target: String,
    headers: HashMap<String, String>,
}

fn read_head(stream: &mut TcpStream) -> Result<RequestHead, String> {
    let mut buf = Vec::with_capacity(4096);
    let mut byte = [0_u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        let n = stream.read(&mut byte).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("empty request".to_string());
        }
        buf.push(byte[0]);
        if buf.len() > 64 * 1024 {
            return Err("request headers too large".to_string());
        }
    }
    let text = std::str::from_utf8(&buf).map_err(|_| "invalid request headers".to_string())?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().ok_or("missing request line")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or("missing method")?.to_string();
    let target = parts.next().ok_or("missing target")?.to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    Ok(RequestHead {
        method,
        target,
        headers,
    })
}

fn content_length(headers: &HashMap<String, String>) -> Result<usize, String> {
    headers
        .get("content-length")
        .ok_or("missing content-length".to_string())?
        .parse::<usize>()
        .map_err(|_| "invalid content-length".to_string())
}

fn read_body(stream: &mut TcpStream, len: usize) -> Result<Vec<u8>, String> {
    let mut body = vec![0_u8; len];
    stream.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok(body)
}

fn json_bytes(value: Value) -> Vec<u8> {
    serde_json::to_vec(&value).unwrap_or_else(|_| b"{\"error\":\"internal\"}".to_vec())
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn write_json(stream: &mut TcpStream, status: u16, reason: &str, value: Value) {
    let body = json_bytes(value);
    write_response(stream, status, reason, "application/json", &body);
}

fn error_json(stream: &mut TcpStream, status: u16, reason: &str, detail: &str) {
    write_json(
        stream,
        status,
        reason,
        json!({"error": {"type": "csswitch_gateway_error", "message": detail}}),
    );
}

fn forbidden_json(stream: &mut TcpStream) {
    write_json(
        stream,
        403,
        "Forbidden",
        json!({
            "type": "error",
            "error": {
                "type": "permission_error",
                "message": "forbidden",
            },
        }),
    );
}

fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        502 => "Bad Gateway",
        _ => "Error",
    }
}

fn dequery(path: &str) -> &str {
    path.split_once('?').map(|(p, _)| p).unwrap_or(path)
}

fn handle_get(stream: &mut TcpStream, cfg: &GatewayConfig, target: &str) {
    let path = match strip_path_secret(dequery(target), cfg.auth_secret.as_deref()) {
        AuthResult::Ok(path) => path,
        AuthResult::Forbidden => {
            forbidden_json(stream);
            return;
        }
    };
    match path.as_str() {
        "/health" => write_json(
            stream,
            200,
            "OK",
            json!({"status": "ok", "provider": cfg.provider}),
        ),
        "/v1/models" => write_json(stream, 200, "OK", models::deepseek_models_response()),
        _ => error_json(stream, 404, "Not Found", "not found"),
    }
}

fn write_chunk(stream: &mut TcpStream, chunk: &[u8]) -> std::io::Result<()> {
    write!(stream, "{:x}\r\n", chunk.len())?;
    stream.write_all(chunk)?;
    stream.write_all(b"\r\n")?;
    stream.flush()
}

fn stream_error_event(detail: &str) -> Vec<u8> {
    format!(
        "event: error\ndata: {}\n\n",
        json!({"error": {"message": detail}})
    )
    .into_bytes()
}

fn handle_stream(stream: &mut TcpStream, cfg: &GatewayConfig, body: Vec<u8>) {
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n"
    );
    match messages::open_stream(cfg, body) {
        Ok(mut upstream) => {
            let mut buf = [0_u8; 8192];
            loop {
                match upstream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if write_chunk(stream, &buf[..n]).is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = write_chunk(stream, &stream_error_event(&e.to_string()));
                        break;
                    }
                }
            }
        }
        Err(e) => {
            let _ = write_chunk(stream, &stream_error_event(&e.detail));
        }
    }
    let _ = stream.write_all(b"0\r\n\r\n");
    let _ = stream.flush();
}

fn handle_messages(stream: &mut TcpStream, cfg: &GatewayConfig, body: Vec<u8>) {
    let raw: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            error_json(stream, 400, "Bad Request", &e.to_string());
            return;
        }
    };
    let is_stream = raw.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let transformed = match policy::transform_request(raw) {
        Ok(body) => body,
        Err(e) => {
            error_json(stream, 400, "Bad Request", &e);
            return;
        }
    };
    if is_stream {
        handle_stream(stream, cfg, transformed);
        return;
    }
    match messages::post_nonstream(cfg, transformed) {
        Ok(resp) => write_response(
            stream,
            resp.status,
            status_reason(resp.status),
            &resp.content_type,
            &resp.body,
        ),
        Err(e) => error_json(stream, e.status, status_reason(e.status), &e.detail),
    }
}

fn handle_post(stream: &mut TcpStream, cfg: &GatewayConfig, target: &str, head: &RequestHead) {
    let path = match strip_path_secret(dequery(target), cfg.auth_secret.as_deref()) {
        AuthResult::Ok(path) => path,
        AuthResult::Forbidden => {
            forbidden_json(stream);
            return;
        }
    };
    if path != "/v1/messages" {
        error_json(stream, 404, "Not Found", "not found");
        return;
    }
    let len = match content_length(&head.headers) {
        Ok(len) => len,
        Err(e) => {
            error_json(stream, 400, "Bad Request", &e);
            return;
        }
    };
    let body = match read_body(stream, len) {
        Ok(body) => body,
        Err(e) => {
            error_json(stream, 400, "Bad Request", &e);
            return;
        }
    };
    handle_messages(stream, cfg, body);
}

fn handle_one(cfg: GatewayConfig, mut stream: TcpStream) {
    let head = match read_head(&mut stream) {
        Ok(head) => head,
        Err(e) => {
            error_json(&mut stream, 400, "Bad Request", &e);
            return;
        }
    };
    match head.method.as_str() {
        "CONNECT" => connect::handle_connect(&head.target, stream),
        "GET" => handle_get(&mut stream, &cfg, &head.target),
        "POST" => {
            let target = head.target.clone();
            handle_post(&mut stream, &cfg, &target, &head)
        }
        _ => error_json(&mut stream, 404, "Not Found", "not found"),
    }
}

pub fn serve(cfg: GatewayConfig) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", cfg.port)).map_err(|e| e.to_string())?;
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let cfg = cfg.clone();
                thread::spawn(move || handle_one(cfg, stream));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}
