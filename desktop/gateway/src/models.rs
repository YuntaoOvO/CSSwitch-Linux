use serde_json::{json, Value};

use crate::config::DEEPSEEK_MODELS;

const CREATED_AT: &str = "2026-01-01T00:00:00Z";

pub fn deepseek_models_response() -> Value {
    let data: Vec<Value> = DEEPSEEK_MODELS
        .iter()
        .map(|(id, display)| {
            json!({
                "type": "model",
                "id": id,
                "display_name": display,
                "supports_tools": null,
                "created_at": CREATED_AT,
            })
        })
        .collect();
    json!({
        "data": data,
        "has_more": false,
        "first_id": DEEPSEEK_MODELS.first().map(|m| m.0),
        "last_id": DEEPSEEK_MODELS.last().map(|m| m.0),
    })
}

#[cfg(test)]
mod tests {
    use super::deepseek_models_response;

    #[test]
    fn models_body_matches_deepseek_shell_contract() {
        let v = deepseek_models_response();
        assert_eq!(v["data"][0]["id"], "claude-opus-4-8");
        assert_eq!(v["data"][0]["display_name"], "DeepSeek V4 Pro");
        assert_eq!(v["first_id"], "claude-opus-4-8");
        assert_eq!(v["last_id"], "claude-haiku-4-5");
    }
}
