use serde_json::Value;

const DEFAULT_MODEL: &str = "deepseek-v4-flash";

pub fn resolve_model(name: Option<&str>) -> &'static str {
    let name = name.unwrap_or("");
    match name {
        "" => DEFAULT_MODEL,
        "claude-opus-4-8" => "deepseek-v4-pro",
        "claude-sonnet-5" | "claude-sonnet-4-6" | "claude-haiku-4-5" => "deepseek-v4-flash",
        "deepseek-v4-pro" => "deepseek-v4-pro",
        "deepseek-v4-flash" => "deepseek-v4-flash",
        _ if name.starts_with("claude-opus-4-8") => "deepseek-v4-pro",
        _ if name.starts_with("claude-sonnet-5")
            || name.starts_with("claude-sonnet-4-6")
            || name.starts_with("claude-haiku-4-5") =>
        {
            "deepseek-v4-flash"
        }
        _ => DEFAULT_MODEL,
    }
}

pub fn clamp_max_tokens(value: Option<u64>, model: &str) -> Option<u64> {
    let cap = match model {
        "deepseek-v4-pro" => 65_536,
        "deepseek-v4-flash" => 32_768,
        _ => 8_192,
    };
    value.map(|v| v.min(cap))
}

pub fn normalize_thinking(body: &mut Value) {
    let forcing = body
        .get("tool_choice")
        .and_then(Value::as_object)
        .and_then(|tc| tc.get("type"))
        .and_then(Value::as_str)
        .map(|t| t == "any" || t == "tool")
        .unwrap_or(false);
    if forcing {
        body["thinking"] = serde_json::json!({"type": "disabled"});
        return;
    }
    if body
        .get("thinking")
        .and_then(Value::as_object)
        .and_then(|th| th.get("type"))
        .and_then(Value::as_str)
        == Some("auto")
    {
        if let Some(thinking) = body.get_mut("thinking").and_then(Value::as_object_mut) {
            thinking.insert("type".to_string(), Value::String("adaptive".to_string()));
        }
    }
}

pub fn transform_request(mut body: Value) -> Result<Vec<u8>, String> {
    let obj = body
        .as_object_mut()
        .ok_or("request body must be a JSON object with a 'messages' array")?;
    if !obj.get("messages").map(Value::is_array).unwrap_or(false) {
        return Err("request body must be a JSON object with a 'messages' array".to_string());
    }
    let model = resolve_model(obj.get("model").and_then(Value::as_str));
    obj.insert("model".to_string(), Value::String(model.to_string()));
    if let Some(max_tokens) = obj.get("max_tokens").and_then(Value::as_u64) {
        obj.insert(
            "max_tokens".to_string(),
            Value::Number(serde_json::Number::from(
                clamp_max_tokens(Some(max_tokens), model).unwrap_or(max_tokens),
            )),
        );
    }
    normalize_thinking(&mut body);
    serde_json::to_vec(&body).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{clamp_max_tokens, resolve_model, transform_request};
    use serde_json::json;

    #[test]
    fn resolves_deepseek_shell_models() {
        assert_eq!(resolve_model(Some("claude-opus-4-8")), "deepseek-v4-pro");
        assert_eq!(resolve_model(Some("claude-haiku-4-5")), "deepseek-v4-flash");
        assert_eq!(resolve_model(Some("")), "deepseek-v4-flash");
    }

    #[test]
    fn clamps_deepseek_max_tokens() {
        assert_eq!(
            clamp_max_tokens(Some(100_000), "deepseek-v4-pro"),
            Some(65_536)
        );
        assert_eq!(clamp_max_tokens(Some(500), "deepseek-v4-pro"), Some(500));
    }

    #[test]
    fn transform_maps_model_and_normalizes_thinking() {
        let raw = json!({
            "model": "claude-opus-4-8",
            "max_tokens": 100000,
            "thinking": {"type": "auto"},
            "messages": [{"role": "user", "content": "hi"}]
        });
        let bytes = transform_request(raw).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["model"], "deepseek-v4-pro");
        assert_eq!(v["max_tokens"], 65536);
        assert_eq!(v["thinking"]["type"], "adaptive");
    }

    #[test]
    fn forced_tool_choice_disables_thinking() {
        let raw = json!({
            "model": "claude-opus-4-8",
            "tool_choice": {"type": "any"},
            "thinking": {"type": "auto"},
            "messages": []
        });
        let bytes = transform_request(raw).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["thinking"]["type"], "disabled");
    }
}
