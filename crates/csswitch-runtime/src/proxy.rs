/// Proxy action classification for diagnostics.
#[derive(Clone, Copy, PartialEq)]
pub enum ProxyAction {
    Reused,
    Restarted,
}

impl ProxyAction {
    pub fn as_str(self) -> &'static str {
        match self {
            ProxyAction::Reused => "reused",
            ProxyAction::Restarted => "restarted",
        }
    }
}

pub fn should_write_back(gen_captured: u64, gen_now: u64, st_secret: &str, my_secret: &str) -> bool {
    gen_captured == gen_now && st_secret == my_secret
}

pub fn health_timeout_reason(port: u16, tail: &str) -> String {
    let occupied = tail.contains("Address already in use")
        || tail.contains("EADDRINUSE")
        || tail.contains("Errno 48")
        || tail.contains("Errno 98");
    if occupied {
        format!("端口 {port} 已被占用，换个端口或先停掉占用进程后重试。")
    } else {
        format!("代理起后探活超时（端口 {port}）：多为 gateway sidecar 缺失或启动异常，请查看代理日志。")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_write_back_requires_both_gen_and_secret() {
        assert!(should_write_back(5, 5, "sekret", "sekret"));
        assert!(!should_write_back(5, 5, "other", "sekret"));
        assert!(!should_write_back(5, 6, "sekret", "sekret"));
        assert!(!should_write_back(5, 6, "other", "sekret"));
    }

    #[test]
    fn health_timeout_reason_flags_port_conflict() {
        let occ = health_timeout_reason(18991, "OSError: [Errno 48] Address already in use");
        assert!(occ.contains("18991"));
        assert!(occ.contains("占用"));
        assert!(!occ.contains("key"));

        let generic = health_timeout_reason(18991, "failed to execute sidecar");
        assert!(!generic.contains("key 无效"));
    }
}
