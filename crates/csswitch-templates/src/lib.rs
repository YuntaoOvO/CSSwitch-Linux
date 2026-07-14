//! 模板注册表：单一来源（spec §5）。template_id 稳定持久于 Profile，据它派生
//! 运行 adapter（模型策略/鉴权/上限）与 UI 能力（是否必选模型 / URL 可编辑 / 内置模型）。
//! 前端 list_templates 取一次铺 UI，不在前端复制常量。

#[derive(Clone)]
pub struct Template {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,   // official | cn_official | custom
    pub api_format: &'static str, // anthropic | openai_chat | openai_responses | gemini_native
    pub adapter: &'static str, // Rust gateway provider：deepseek | qwen | relay | openai-custom | openai-responses
    pub base_url: &'static str, // 默认；空=用户填
    pub base_url_editable: bool,
    pub requires_model_override: bool,
    pub builtin_models: &'static [&'static str],
    pub website_url: &'static str,
    pub icon: &'static str,
    pub icon_color: &'static str,
    pub thinking_policy: &'static str, // relay thinking 策略：adaptive（默认）/ enabled（Kimi）/ ""（native）
}

pub fn all() -> &'static [Template] {
    TEMPLATES
}

pub fn by_id(id: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.id == id)
}

/// 未命中 → "relay"（通用 anthropic 兼容透传，双鉴权）。
pub fn adapter_for(template_id: &str) -> &'static str {
    by_id(template_id).map(|t| t.adapter).unwrap_or("relay")
}

/// 模板的 relay thinking 策略；未命中 → ""（native/未知不注入，代理走默认 auto→adaptive）。
pub fn thinking_policy_for(template_id: &str) -> &'static str {
    by_id(template_id).map(|t| t.thinking_policy).unwrap_or("")
}

/// 旧固定槽 id → 新 template_id（迁移用）。未知/遗留裸 relay → custom。
pub fn template_id_for_legacy_slot(slot: &str) -> &'static str {
    match slot {
        "deepseek" => "deepseek",
        "qwen" => "qwen",
        "relay-glm" => "glm",
        "relay-xiaomi" => "xiaomi",
        "relay-siliconflow" => "siliconflow",
        "relay-openrouter" => "openrouter",
        _ => "custom",
    }
}

static TEMPLATES: &[Template] = &[
    Template {
        id: "deepseek",
        name: "DeepSeek",
        category: "cn_official",
        api_format: "anthropic",
        adapter: "deepseek",
        base_url: "https://api.deepseek.com/anthropic",
        base_url_editable: false,
        requires_model_override: false,
        builtin_models: &["claude-opus-4-8", "claude-haiku-4-5"],
        website_url: "https://platform.deepseek.com",
        icon: "deepseek",
        icon_color: "#1E88E5",
        thinking_policy: "",
    },
    Template {
        id: "glm",
        name: "智谱 GLM",
        category: "cn_official",
        api_format: "anthropic",
        adapter: "relay",
        base_url: "https://open.bigmodel.cn/api/anthropic",
        base_url_editable: true,
        requires_model_override: true, // #9：全 relay 统一 FIXED（选/填一个模型 → force）
        builtin_models: &["glm-5.2", "glm-4.7", "glm-4.6", "glm-4.5-air"], // 官方核定 2026-07-04：旗舰 glm-5.2
        website_url: "https://open.bigmodel.cn",
        icon: "glm",
        icon_color: "#2E6BE6",
        thinking_policy: "adaptive",
    },
    Template {
        id: "xiaomi",
        name: "小米 MiMo",
        category: "cn_official",
        api_format: "anthropic",
        adapter: "relay",
        base_url: "https://api.xiaomimimo.com/anthropic",
        base_url_editable: true,
        requires_model_override: true,
        builtin_models: &["mimo-v2.5-pro"],
        website_url: "https://xiaomimimo.com",
        icon: "xiaomi",
        icon_color: "#FF6900",
        thinking_policy: "adaptive",
    },
    Template {
        id: "siliconflow",
        name: "硅基流动",
        category: "cn_official",
        api_format: "anthropic",
        adapter: "relay",
        base_url: "https://api.siliconflow.cn",
        base_url_editable: true,
        requires_model_override: true,
        builtin_models: &[
            "deepseek-ai/DeepSeek-V4-Pro",
            "deepseek-ai/DeepSeek-V4-Flash",
            "deepseek-ai/DeepSeek-V3.2",
            "zai-org/GLM-5.2",
        ], // 官方核定 2026-07-04；真机证实 api.siliconflow.cn/v1/messages 返回 Anthropic 200（relay/anthropic 配置正确，无需翻译）
        website_url: "https://siliconflow.cn",
        icon: "siliconflow",
        icon_color: "#7C3AED",
        thinking_policy: "adaptive",
    },
    Template {
        id: "kimi",
        name: "Kimi（Moonshot）",
        category: "cn_official",
        api_format: "anthropic",
        adapter: "relay",
        base_url: "https://api.moonshot.cn/anthropic", // 国际站可改 api.moonshot.ai/anthropic
        base_url_editable: true,
        requires_model_override: true,
        builtin_models: &["kimi-k2.7-code", "kimi-k2.7-code-highspeed", "kimi-k2.6"], // 官方核定 2026-07-04
        website_url: "https://platform.moonshot.cn",
        icon: "kimi",
        icon_color: "#16182F",
        thinking_policy: "enabled",
    },
    Template {
        id: "minimax",
        name: "MiniMax",
        category: "cn_official",
        api_format: "anthropic",
        adapter: "relay",
        base_url: "https://api.minimaxi.com/anthropic", // 国内站（真机验证：key 有效 + /v1/models 实时发现 200）；国际站改 api.minimax.io
        base_url_editable: true,
        requires_model_override: true,
        builtin_models: &["MiniMax-M3", "MiniMax-M2.7", "MiniMax-M2.7-highspeed"], // 官方核定 2026-07-04：旗舰 M3（2026-06-01 GA）
        website_url: "https://platform.minimaxi.com",
        icon: "minimax",
        icon_color: "#E1341E",
        thinking_policy: "adaptive",
    },
    Template {
        id: "openrouter",
        name: "OpenRouter",
        category: "custom",
        api_format: "anthropic",
        adapter: "relay",
        base_url: "https://openrouter.ai/api",
        base_url_editable: true,
        requires_model_override: true, // #9：全 relay 统一 FIXED
        builtin_models: &[
            "anthropic/claude-sonnet-5",
            "anthropic/claude-opus-4.8",
            "anthropic/claude-opus-4.8-fast",
        ], // 官方核定 2026-07-04：补非 2x 价的 opus-4.8
        website_url: "https://openrouter.ai",
        icon: "openrouter",
        icon_color: "#6467F2",
        thinking_policy: "adaptive",
    },
    Template {
        id: "qwen",
        name: "通义千问",
        category: "cn_official",
        api_format: "openai_chat",
        adapter: "qwen",
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        base_url_editable: false,
        requires_model_override: false,
        builtin_models: &["qwen3.7-max", "qwen-plus-latest", "qwen-turbo"],
        website_url: "https://dashscope.aliyun.com",
        icon: "qwen",
        icon_color: "#615CED",
        thinking_policy: "",
    },
    Template {
        id: "custom-openai",
        name: "自定义 OpenAI",
        category: "custom",
        api_format: "openai_chat",
        adapter: "openai-custom",
        base_url: "",
        base_url_editable: true,
        requires_model_override: true,
        builtin_models: &[],
        website_url: "",
        icon: "custom",
        icon_color: "#2563EB",
        thinking_policy: "",
    },
    Template {
        id: "custom-openai-responses",
        name: "自定义 OpenAI Responses",
        category: "custom",
        api_format: "openai_responses",
        adapter: "openai-responses",
        base_url: "",
        base_url_editable: true,
        requires_model_override: true,
        builtin_models: &[],
        website_url: "",
        icon: "custom",
        icon_color: "#0F766E",
        thinking_policy: "",
    },
    Template {
        id: "custom",
        name: "自定义 Anthropic",
        category: "custom",
        api_format: "anthropic",
        adapter: "relay",
        base_url: "",
        base_url_editable: true,
        requires_model_override: true,
        builtin_models: &[],
        website_url: "",
        icon: "custom",
        icon_color: "#6B7280",
        thinking_policy: "adaptive",
    },
];


#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_templates_have_required_fields() {
        for t in all() {
            assert!(!t.id.is_empty());
            assert!(!t.name.is_empty());
            assert!(!t.adapter.is_empty());
            assert!(!t.api_format.is_empty());
        }
    }
}
