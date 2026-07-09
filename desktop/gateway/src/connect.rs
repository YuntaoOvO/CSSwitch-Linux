use std::io::Write;
use std::net::TcpStream;
use std::thread;

fn write_status(mut stream: TcpStream, code: u16, reason: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {code} {reason}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
    );
    let _ = stream.flush();
}

pub fn is_blocked_host(host: &str) -> bool {
    let host = host.trim_matches('.').to_ascii_lowercase();
    host == "anthropic.com"
        || host.ends_with(".anthropic.com")
        || host == "claude.ai"
        || host.ends_with(".claude.ai")
        || host == "claude.com"
        || host.ends_with(".claude.com")
}

fn parse_target(target: &str) -> Result<(String, u16), ()> {
    if let Some(rest) = target.strip_prefix('[') {
        let (host, suffix) = rest.split_once(']').ok_or(())?;
        let port = suffix
            .strip_prefix(':')
            .ok_or(())?
            .parse()
            .map_err(|_| ())?;
        return Ok((host.to_string(), port));
    }
    let (host, port) = target.rsplit_once(':').ok_or(())?;
    if host.is_empty() {
        return Err(());
    }
    Ok((host.to_string(), port.parse().map_err(|_| ())?))
}

pub fn handle_connect(target: &str, mut client: TcpStream) {
    let Ok((host, port)) = parse_target(target) else {
        write_status(client, 400, "Bad Request");
        return;
    };
    if is_blocked_host(&host) {
        write_status(client, 401, "Unauthorized");
        return;
    }
    let upstream = match TcpStream::connect((host.as_str(), port)) {
        Ok(stream) => stream,
        Err(_) => {
            write_status(client, 502, "Bad Gateway");
            return;
        }
    };
    let _ = client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n");
    let _ = client.flush();

    let mut client_r = match client.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut upstream_w = match upstream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut upstream_r = upstream;
    let mut client_w = client;

    let to_upstream = thread::spawn(move || {
        let _ = std::io::copy(&mut client_r, &mut upstream_w);
        let _ = upstream_w.shutdown(std::net::Shutdown::Write);
    });
    let to_client = thread::spawn(move || {
        let _ = std::io::copy(&mut upstream_r, &mut client_w);
        let _ = client_w.shutdown(std::net::Shutdown::Write);
    });
    let _ = to_upstream.join();
    let _ = to_client.join();
}

#[cfg(test)]
mod tests {
    use super::is_blocked_host;

    #[test]
    fn blocks_anthropic_claude_hosts_only() {
        assert!(is_blocked_host("api.anthropic.com"));
        assert!(is_blocked_host("claude.ai"));
        assert!(is_blocked_host("foo.claude.com"));
        assert!(!is_blocked_host("example.com"));
    }
}
