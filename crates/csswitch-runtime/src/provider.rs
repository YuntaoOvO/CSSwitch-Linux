use csswitch_config::Profile;
use csswitch_templates;

/// Non-cryptographic key fingerprint (SipHash) for configuration change detection.
pub fn key_fingerprint(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

pub fn key_env_for_adapter(adapter: &str) -> &'static str {
    match adapter {
        "deepseek" => "DEEPSEEK_API_KEY",
        "qwen" => "DASHSCOPE_API_KEY",
        "openai-custom" | "openai-responses" => "CSSWITCH_OPENAI_KEY",
        _ => "CSSWITCH_RELAY_KEY",
    }
}

pub struct ProxyLaunch {
    pub adapter: String,
    pub base_url: String,
    pub model: String,
    pub key: String,
    pub key_env: &'static str,
    pub thinking_policy: &'static str,
}

pub fn adapter_for_profile(p: &Profile) -> &'static str {
    if p.template_id == "custom" {
        match p.api_format.as_str() {
            "openai_chat" => "openai-custom",
            "openai_responses" => "openai-responses",
            _ => csswitch_templates::adapter_for(&p.template_id),
        }
    } else {
        csswitch_templates::adapter_for(&p.template_id)
    }
}

pub fn proxy_args_for(p: &Profile) -> ProxyLaunch {
    let adapter = adapter_for_profile(p).to_string();
    let key_env = key_env_for_adapter(&adapter);
    ProxyLaunch {
        adapter,
        base_url: p.base_url.clone(),
        model: p.model.clone(),
        key: p.api_key.clone(),
        key_env,
        thinking_policy: csswitch_templates::thinking_policy_for(&p.template_id),
    }
}

pub fn proxy_fingerprint_with_runtime(
    p: &Profile,
    launch: &ProxyLaunch,
    gateway_kind: &str,
    shim_mode: &str,
) -> u64 {
    let shim_mode = normalize_shim_mode(&launch.adapter, Some(shim_mode));
    key_fingerprint(&format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        p.template_id,
        p.api_format,
        launch.adapter,
        launch.base_url,
        launch.model,
        launch.thinking_policy,
        launch.key,
        gateway_kind,
        shim_mode
    ))
}

pub fn assert_format_supported(p: &Profile) -> Result<(), String> {
    match p.api_format.as_str() {
        "anthropic" | "openai_chat" | "openai_responses" => Ok(()),
        other => Err(format!(
            "api_format `{other}` 暂不支持，请选 anthropic、openai_chat 或 openai_responses。"
        )),
    }
}

pub fn is_native_adapter(adapter: &str) -> bool {
    adapter == "deepseek" || adapter == "qwen"
}

pub fn is_openai_adapter(adapter: &str) -> bool {
    matches!(adapter, "openai-custom" | "openai-responses")
}

pub fn gateway_kind_for_adapter(_adapter: &str) -> &'static str {
    "rust"
}

pub fn current_shim_mode_for_adapter(adapter: &str) -> &'static str {
    if adapter != "deepseek" {
        return "off";
    }
    std::env::var("CSSWITCH_DSML_SHIM")
        .ok()
        .as_deref()
        .map(normalize_shim_mode_raw)
        .unwrap_or("detect")
}

fn normalize_shim_mode_raw(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "detect" => "detect",
        "rewrite" => "rewrite",
        _ => "off",
    }
}

pub fn normalize_shim_mode(adapter: &str, raw: Option<&str>) -> &'static str {
    if adapter != "deepseek" {
        return "off";
    }
    match raw.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "detect" => "detect",
        "rewrite" => "rewrite",
        _ => "off",
    }
}

pub fn reject_openai_custom_anthropic_base(adapter: &str, base_url: &str) -> Result<(), String> {
    if is_openai_adapter(adapter) {
        let u = base_url.trim().trim_end_matches('/').to_ascii_lowercase();
        if u.contains("/anthropic") {
            return Err("这个地址看起来是 Anthropic 兼容端点。请改选「自定义 Anthropic」，或使用 OpenAI 兼容 base root。".to_string());
        }
    }
    Ok(())
}

pub fn relay_missing_base_url(adapter: &str, base_url: &str) -> bool {
    !is_native_adapter(adapter) && base_url.trim().is_empty()
}

pub fn relay_missing_model(adapter: &str, model: &str) -> bool {
    !is_native_adapter(adapter) && model.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_fingerprint_stable_and_distinct() {
        assert_eq!(key_fingerprint("sk-aaaa"), key_fingerprint("sk-aaaa"));
        assert_ne!(key_fingerprint("sk-aaaa"), key_fingerprint("sk-bbbb"));
    }

    #[test]
    fn native_adapter_detection() {
        assert!(is_native_adapter("deepseek"));
        assert!(is_native_adapter("qwen"));
        assert!(!is_native_adapter("relay"));
        assert!(!is_native_adapter("openai-custom"));
    }

    #[test]
    fn shim_mode_only_for_deepseek() {
        assert_eq!(normalize_shim_mode("deepseek", Some("detect")), "detect");
        assert_eq!(normalize_shim_mode("deepseek", Some("rewrite")), "rewrite");
        assert_eq!(normalize_shim_mode("deepseek", Some("off")), "off");
        assert_eq!(normalize_shim_mode("qwen", Some("rewrite")), "off");
        assert_eq!(normalize_shim_mode("relay", Some("detect")), "off");
    }

    #[test]
    fn key_env_for_adapter_returns_correct_vars() {
        assert_eq!(key_env_for_adapter("deepseek"), "DEEPSEEK_API_KEY");
        assert_eq!(key_env_for_adapter("qwen"), "DASHSCOPE_API_KEY");
        assert_eq!(key_env_for_adapter("openai-custom"), "CSSWITCH_OPENAI_KEY");
        assert_eq!(key_env_for_adapter("relay"), "CSSWITCH_RELAY_KEY");
    }

    #[test]
    fn relay_missing_base_url_detection() {
        assert!(relay_missing_base_url("relay", ""));
        assert!(relay_missing_base_url("relay", "   "));
        assert!(relay_missing_base_url("custom", ""));
        assert!(!relay_missing_base_url("relay", "https://r"));
        assert!(!relay_missing_base_url("deepseek", ""));
        assert!(!relay_missing_base_url("qwen", ""));
    }
}
