# Provider Capability Matrix

本文只记录公开代码层面的 provider 能力边界，供后续开发和 PR 摘要引用。它不记录测试环境、密钥、账号、路径或私有实现细节。

| Family | Templates | Adapter | Upstream API | Endpoint handling | Model handling | Tool handling | Streaming behavior |
|---|---|---|---|---|---|---|---|
| Anthropic-compatible native | DeepSeek and compatible relay presets | `deepseek` / `relay` | Anthropic Messages | Uses provider base URL and `/v1/messages` / `/v1/models` conventions | Native mapping or selected relay model | Preserves Anthropic tool blocks, with provider-specific policy filters where configured | Proxies upstream SSE, with local keepalive/error handling |
| OpenAI Chat compatible | Qwen and custom OpenAI Chat | `qwen` / `openai-custom` | Chat Completions | Base root derives `/chat/completions` and `/models` | Forces selected model when configured | Converts Anthropic tools to Chat Completions tools and maps tool calls back | Uses non-streaming upstream response with local Anthropic SSE replay |
| OpenAI Responses compatible | Custom OpenAI Responses | `openai-responses` | Responses | Base root derives `/responses` and `/models` | Forces selected model when configured; clamps `max_output_tokens` to 65536 | Converts Anthropic tools to Responses function tools; forced `tool` / `any` choices degrade to `auto` | Uses non-streaming upstream response with local Anthropic SSE replay |

## Current Boundary

- Responses support is a first compatibility slice, not a claim of full native Responses parity.
- Native Responses SSE conversion remains future work.
- Provider-specific behavior should continue moving toward explicit capability flags instead of broad assumptions shared by all providers.
