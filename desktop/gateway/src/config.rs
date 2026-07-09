#[derive(Clone, Debug)]
pub struct GatewayConfig {
    pub provider: String,
    pub port: u16,
    pub auth_secret: Option<String>,
    pub api_key: String,
    pub upstream_url: String,
}

pub const UPSTREAM_UA: &str = "CSSwitch/0.2 (+https://github.com/SuperJJ007/CSSwitch)";
pub const DEFAULT_UPSTREAM_URL: &str = "https://api.deepseek.com/anthropic/v1/messages";

pub const DEEPSEEK_MODELS: &[(&str, &str)] = &[
    ("claude-opus-4-8", "DeepSeek V4 Pro"),
    ("claude-haiku-4-5", "DeepSeek V4 Flash"),
];

pub fn shim_mode(raw: Option<&str>) -> &'static str {
    match raw.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "detect" => "detect",
        "rewrite" => "rewrite",
        _ => "off",
    }
}

pub fn rust_eligible(provider: &str, shim: &str) -> bool {
    provider == "deepseek" && shim == "off"
}

impl GatewayConfig {
    pub fn from_env_args(args: Vec<String>) -> Result<Self, String> {
        let mut provider = "deepseek".to_string();
        let mut port: Option<u16> = None;
        let mut auth_token_arg: Option<String> = None;

        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--provider" => {
                    i += 1;
                    provider = args.get(i).ok_or("--provider 缺少值")?.clone();
                }
                "--port" => {
                    i += 1;
                    let raw = args.get(i).ok_or("--port 缺少值")?;
                    port = Some(raw.parse().map_err(|_| format!("非法端口：{raw}"))?);
                }
                "--auth-token" => {
                    i += 1;
                    auth_token_arg = Some(args.get(i).ok_or("--auth-token 缺少值")?.clone());
                }
                other => return Err(format!("未知参数：{other}")),
            }
            i += 1;
        }

        let shim = shim_mode(std::env::var("CSSWITCH_TOOLUSE_SHIM").ok().as_deref());
        if !rust_eligible(&provider, shim) {
            return Err(format!(
                "只支持 deepseek + shim off（provider={provider}, shim={shim}）"
            ));
        }

        let api_key = std::env::var("DEEPSEEK_API_KEY")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or("缺少 DEEPSEEK_API_KEY")?;
        let auth_secret = std::env::var("CSSWITCH_AUTH_TOKEN")
            .ok()
            .filter(|v| !v.is_empty())
            .or(auth_token_arg)
            .filter(|v| !v.is_empty());
        let upstream_url = std::env::var("CSSWITCH_UPSTREAM_URL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_UPSTREAM_URL.to_string());
        Ok(Self {
            provider,
            port: port.ok_or("--port 必填")?,
            auth_secret,
            api_key,
            upstream_url,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{rust_eligible, shim_mode};

    #[test]
    fn shim_mode_parses_deepseek_off_contract() {
        assert_eq!(shim_mode(None), "off");
        assert_eq!(shim_mode(Some("Detect")), "detect");
        assert_eq!(shim_mode(Some("rewrite")), "rewrite");
        assert_eq!(shim_mode(Some("bad")), "off");
    }

    #[test]
    fn eligibility_is_deepseek_off_only() {
        assert!(rust_eligible("deepseek", "off"));
        assert!(!rust_eligible("deepseek", "detect"));
        assert!(!rust_eligible("qwen", "off"));
        assert!(!rust_eligible("relay", "off"));
    }
}
