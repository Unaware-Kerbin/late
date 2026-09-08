//! Hub `tokenizer_config.json` `chat_template` (Jinja) when present.
//! Falls back to per-family Qwen Hermes / Gemma 4 templates if render fails.
//!
//! The compiled blob never receives a filesystem. Tools stay OpenAI `tools`
//! in the prompt; writes stay Late Approve + Orchestrator allowlists.

use crate::openai::ChatMessage;
use minijinja::{Environment, Error as MjError, ErrorKind, Value};
use serde_json::json;

pub fn load_chat_template(tokenizer_config: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(tokenizer_config).ok()?;
    v.get("chat_template")?.as_str().map(|s| s.to_string())
}

/// Render Hub Jinja. Returns None if the template uses filters we do not implement.
pub fn apply_hub_chat_template(
    template: &str,
    messages: &[ChatMessage],
    tools: Option<&[serde_json::Value]>,
    add_generation_prompt: bool,
) -> Option<String> {
    let mut env = Environment::new();
    env.add_filter("tojson", tojson);
    env.add_function("raise_exception", raise_exception);
    env.add_filter("trim", |s: String| -> String { s.trim().to_string() });
    if env.add_template("chat", template).is_err() {
        return None;
    }
    let tmpl = env.get_template("chat").ok()?;
    let msgs: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            json!({
                "role": m.role,
                "content": m.text(),
            })
        })
        .collect();
    let ctx = json!({
        "messages": msgs,
        "tools": tools.unwrap_or(&[]),
        "add_generation_prompt": add_generation_prompt,
        "bos_token": "",
        "eos_token": "",
    });
    tmpl.render(ctx).ok()
}

fn tojson(v: Value) -> Result<String, MjError> {
    let dumped = serde_json::to_string(&v).map_err(|e| {
        MjError::new(ErrorKind::InvalidOperation, e.to_string())
    })?;
    Ok(dumped)
}

fn raise_exception(msg: String) -> Result<Value, MjError> {
    Err(MjError::new(ErrorKind::InvalidOperation, msg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::{ChatMessage, Content};

    fn msg(role: &str, text: &str) -> ChatMessage {
        ChatMessage {
            role: role.into(),
            content: Content::Text(text.into()),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }
    }

    #[test]
    fn simple_jinja_chat_template() {
        let t = "{% for message in messages %}{{ message.role }}:{{ message.content }}\n{% endfor %}{% if add_generation_prompt %}assistant:\n{% endif %}";
        let s = apply_hub_chat_template(t, &[msg("user", "hi")], None, true).unwrap();
        assert_eq!(s, "user:hi\nassistant:\n");
    }

    #[test]
    fn load_template_from_tokenizer_config() {
        let raw = br#"{"chat_template":"{{ messages[0].content }}","unk_token":"<unk>"}"#;
        assert_eq!(load_chat_template(raw).as_deref(), Some("{{ messages[0].content }}"));
    }

    #[test]
    fn broken_template_returns_none() {
        assert!(apply_hub_chat_template("{{ unknown_filter(x) }}", &[], None, true).is_none());
    }
}
