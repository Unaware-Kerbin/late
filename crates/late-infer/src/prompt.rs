use crate::openai::{ChatMessage, FunctionCall, ToolCall};

const DEFAULT_SYSTEM: &str = "You are Qwen, created by Alibaba Cloud. You are a helpful assistant.";

const TOOLS_PREAMBLE: &str = "\n\n# Tools\n\nYou may call one or more functions to assist with the user query.\n\nYou are provided with function signatures within <tools></tools> XML tags:\n<tools>";

const TOOLS_EPILOGUE: &str = "\n</tools>\n\nFor each function call, return a json object with function name and arguments within <tool_call></tool_call> XML tags:\n<tool_call>\n{\"name\": <function-name>, \"arguments\": <args-json-object>}\n</tool_call>";

/// Qwen2.5 Instruct chat template (no Jinja). Hermes `<tool_call>` when tools are on.
pub fn apply_qwen_chat_template(messages: &[ChatMessage]) -> String {
    apply_qwen_chat_template_with_tools(messages, None, None)
}

pub fn apply_qwen_chat_template_with_tools(
    messages: &[ChatMessage],
    tools: Option<&[serde_json::Value]>,
    required_name: Option<&str>,
) -> String {
    let tools_on = tools.is_some_and(|t| !t.is_empty());
    let mut out = String::new();
    let skip_first_system = tools_on && messages.first().is_some_and(|m| m.role == "system");

    if tools_on {
        let tools = tools.unwrap();
        out.push_str("<|im_start|>system\n");
        let sys = if skip_first_system {
            let t = messages[0].text();
            if t.trim().is_empty() {
                DEFAULT_SYSTEM.to_string()
            } else {
                t
            }
        } else {
            DEFAULT_SYSTEM.to_string()
        };
        out.push_str(&sys);
        out.push_str(TOOLS_PREAMBLE);
        for tool in tools {
            out.push('\n');
            match serde_json::to_string(tool) {
                Ok(s) => out.push_str(&s),
                Err(_) => out.push_str("{}"),
            }
        }
        out.push_str(TOOLS_EPILOGUE);
        if let Some(name) = required_name {
            if name.is_empty() {
                out.push_str("\nYou must call a function.");
            } else {
                out.push_str("\nYou must call the function named ");
                out.push_str(name);
                out.push('.');
            }
        }
        out.push_str("<|im_end|>\n");
    }

    let mut i = 0;
    while i < messages.len() {
        if i == 0 && skip_first_system {
            i += 1;
            continue;
        }
        let m = &messages[i];
        if m.role == "tool" {
            out.push_str("<|im_start|>user\n");
            while i < messages.len() && messages[i].role == "tool" {
                out.push_str("<tool_response>\n");
                out.push_str(&messages[i].text());
                out.push_str("\n</tool_response>\n");
                i += 1;
            }
            out.push_str("<|im_end|>\n");
            continue;
        }
        let calls = m
            .tool_calls
            .as_ref()
            .map(|c| c.as_slice())
            .unwrap_or(&[]);
        if m.role == "assistant" && !calls.is_empty() {
            out.push_str("<|im_start|>assistant");
            let text = m.text();
            if !text.is_empty() {
                out.push('\n');
                out.push_str(&text);
            }
            for call in calls {
                if let Some((name, args)) = call.name_and_args() {
                    out.push_str("\n<tool_call>\n");
                    out.push_str(&hermes_call_json(&name, &args));
                    out.push_str("\n</tool_call>");
                }
            }
            out.push_str("<|im_end|>\n");
            i += 1;
            continue;
        }
        let role = match m.role.as_str() {
            "system" | "user" | "assistant" => m.role.as_str(),
            other => other,
        };
        out.push_str("<|im_start|>");
        out.push_str(role);
        out.push('\n');
        out.push_str(&m.text());
        out.push_str("<|im_end|>\n");
        i += 1;
    }
    if messages.last().map(|m| m.role.as_str()) != Some("assistant") {
        out.push_str("<|im_start|>assistant\n");
    }
    out
}

fn hermes_call_json(name: &str, arguments: &str) -> String {
    let args_val: serde_json::Value = serde_json::from_str(arguments)
        .unwrap_or_else(|_| serde_json::Value::String(arguments.to_string()));
    serde_json::json!({ "name": name, "arguments": args_val }).to_string()
}

/// Parse Hermes `<tool_call>` JSON into OpenAI `tool_calls` (`arguments` is a string).
/// Finds every `<tool_call>…</tool_call>` (newlines allowed). Invalid bodies are skipped.
pub fn parse_hermes_tool_calls(text: &str) -> Vec<ToolCall> {
    let mut out = Vec::new();
    let mut search_from = 0;
    let mut saw_tag = false;
    while let Some(rel) = text[search_from..].find("<tool_call>") {
        saw_tag = true;
        let start = search_from + rel + "<tool_call>".len();
        let Some(end_rel) = text[start..].find("</tool_call>") else {
            break;
        };
        let body = text[start..start + end_rel].trim();
        if let Some(tc) = parse_one_tool_json(body, out.len()) {
            out.push(tc);
        }
        search_from = start + end_rel + "</tool_call>".len();
    }
    if out.is_empty() && !saw_tag {
        if let Some(tc) = parse_bare_tool_json(text) {
            out.push(tc);
        }
    }
    out
}

fn parse_bare_tool_json(text: &str) -> Option<ToolCall> {
    let t = text.trim();
    if !t.starts_with('{') {
        return None;
    }
    parse_one_tool_json(t, 0)
}

fn parse_one_tool_json(body: &str, i: usize) -> Option<ToolCall> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let obj = v.as_object()?;
    let name = obj.get("name")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }
    let arguments = match obj.get("arguments") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Null) | None => "{}".into(),
        Some(other) => serde_json::to_string(other).unwrap_or_else(|_| "{}".into()),
    };
    Some(ToolCall {
        id: format!("call_{i}"),
        kind: "function",
        function: FunctionCall {
            name: name.to_string(),
            arguments,
        },
    })
}

const GEMMA_BOS: &str = "<bos>";
const GEMMA_DEFAULT_SYSTEM: &str = "You are a helpful assistant.";

/// Gemma 4 Instruct chat template (canonical turns, not Qwen ChatML).
/// Tool declarations use `<|tool>declaration:name{…}<tool|>`. Calls use
/// `<|tool_call>call:name{key:value}<tool_call|>` (not Hermes XML).
pub fn apply_gemma_chat_template(messages: &[ChatMessage]) -> String {
    apply_gemma_chat_template_with_tools(messages, None, None)
}

pub fn apply_gemma_chat_template_with_tools(
    messages: &[ChatMessage],
    tools: Option<&[serde_json::Value]>,
    required_name: Option<&str>,
) -> String {
    let tools_on = tools.is_some_and(|t| !t.is_empty());
    let mut out = String::from(GEMMA_BOS);
    let skip_first_system = messages.first().is_some_and(|m| m.role == "system");
    if tools_on || skip_first_system {
        out.push_str("<|turn>system\n");
        if skip_first_system {
            let t = messages[0].text();
            if t.trim().is_empty() {
                out.push_str(GEMMA_DEFAULT_SYSTEM);
            } else {
                out.push_str(t.trim());
            }
        }
        if tools_on {
            for tool in tools.unwrap() {
                out.push_str("<|tool>");
                out.push_str(&format_gemma_tool_declaration(tool));
                out.push_str("<tool|>");
            }
            if let Some(name) = required_name {
                if name.is_empty() {
                    out.push_str("\nYou must call a function.");
                } else {
                    out.push_str("\nYou must call the function named ");
                    out.push_str(name);
                    out.push('.');
                }
            }
        }
        out.push_str("<turn|>\n");
    }

    let mut i = if skip_first_system { 1 } else { 0 };
    while i < messages.len() {
        let m = &messages[i];
        if m.role == "tool" {
            i += 1;
            continue;
        }
        let role = if m.role == "assistant" { "model" } else { m.role.as_str() };
        out.push_str("<|turn>");
        out.push_str(role);
        out.push('\n');
        let calls = m
            .tool_calls
            .as_ref()
            .map(|c| c.as_slice())
            .unwrap_or(&[]);
        if !calls.is_empty() {
            for call in calls {
                if let Some((name, args)) = call.name_and_args() {
                    out.push_str("<|tool_call>call:");
                    out.push_str(&name);
                    out.push_str(&format_gemma_call_args(&args));
                    out.push_str("<tool_call|>");
                }
            }
            let mut j = i + 1;
            while j < messages.len() && messages[j].role == "tool" {
                let tname = tool_name_for_response(calls, &messages[j]);
                out.push_str("<|tool_response>response:");
                out.push_str(&tname);
                out.push_str("{value:");
                out.push_str(&gemma_quote_string(&messages[j].text()));
                out.push_str("}<tool_response|>");
                j += 1;
            }
            let text = m.text();
            if !text.is_empty() {
                out.push_str(text.trim());
            }
            out.push_str("<turn|>\n");
            i = j;
            continue;
        }
        out.push_str(m.text().trim());
        out.push_str("<turn|>\n");
        i += 1;
    }
    if messages.last().map(|m| m.role.as_str()) != Some("assistant") {
        out.push_str("<|turn>model\n");
    }
    out
}

fn tool_name_for_response(calls: &[crate::openai::IncomingToolCall], tool_msg: &ChatMessage) -> String {
    if let Some(id) = tool_msg.tool_call_id.as_deref() {
        for c in calls {
            if c.id.as_deref() == Some(id) {
                if let Some((name, _)) = c.name_and_args() {
                    return name;
                }
            }
        }
    }
    tool_msg.name.clone().unwrap_or_else(|| "unknown".into())
}

fn format_gemma_tool_declaration(tool: &serde_json::Value) -> String {
    let func = tool.get("function").unwrap_or(tool);
    let name = func
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("unknown");
    let desc = func.get("description").and_then(|d| d.as_str()).unwrap_or("");
    let mut s = format!("declaration:{name}{{description:{}", gemma_quote_string(desc));
    if let Some(params) = func.get("parameters") {
        s.push_str(",parameters:");
        s.push_str(&format_gemma_json_like(params, false));
    }
    s.push('}');
    s
}

fn format_gemma_call_args(arguments: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(arguments)
        .unwrap_or_else(|_| serde_json::Value::String(arguments.to_string()));
    match v {
        serde_json::Value::Object(_) | serde_json::Value::Null => format_gemma_json_like(&v, false),
        other => {
            let mut m = serde_json::Map::new();
            m.insert("value".into(), other);
            format_gemma_json_like(&serde_json::Value::Object(m), false)
        }
    }
}

fn format_gemma_json_like(v: &serde_json::Value, escape_keys: bool) -> String {
    match v {
        serde_json::Value::Null => "null".into(),
        serde_json::Value::Bool(b) => {
            if *b {
                "true".into()
            } else {
                "false".into()
            }
        }
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => gemma_quote_string(s),
        serde_json::Value::Array(a) => {
            let inner = a
                .iter()
                .map(|x| format_gemma_json_like(x, escape_keys))
                .collect::<Vec<_>>()
                .join(",");
            format!("[{inner}]")
        }
        serde_json::Value::Object(m) => {
            let mut keys: Vec<_> = m.keys().collect();
            keys.sort();
            let mut inner = String::new();
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    inner.push(',');
                }
                if escape_keys {
                    inner.push_str(&gemma_quote_string(k));
                } else {
                    inner.push_str(k);
                }
                inner.push(':');
                inner.push_str(&format_gemma_json_like(&m[*k], escape_keys));
            }
            format!("{{{inner}}}")
        }
    }
}

fn gemma_quote_string(s: &str) -> String {
    format!("<|\"|>{s}<|\"|>")
}

/// Parse Gemma 4 `<|tool_call>call:name{…}<tool_call|>` into OpenAI `tool_calls`.
pub fn parse_gemma_tool_calls(text: &str) -> Vec<ToolCall> {
    let mut out = Vec::new();
    let mut search_from = 0;
    const OPEN: &str = "<|tool_call>call:";
    const CLOSE: &str = "<tool_call|>";
    while let Some(rel) = text[search_from..].find(OPEN) {
        let start = search_from + rel + OPEN.len();
        let rest = &text[start..];
        let Some(brace) = rest.find('{') else {
            break;
        };
        let name = rest[..brace].trim();
        if name.is_empty() {
            search_from = start + brace + 1;
            continue;
        }
        let Some((args_json, consumed)) = parse_gemma_object_to_json(&rest[brace..]) else {
            search_from = start + brace + 1;
            continue;
        };
        let after = &rest[brace + consumed..];
        let close_at = after.find(CLOSE).unwrap_or(0);
        if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            out.push(ToolCall {
                id: format!("call_{}", out.len()),
                kind: "function",
                function: FunctionCall {
                    name: name.to_string(),
                    arguments: args_json,
                },
            });
        }
        search_from = start + brace + consumed + close_at + CLOSE.len();
    }
    out
}

fn parse_gemma_object_to_json(s: &str) -> Option<(String, usize)> {
    let (v, n) = parse_gemma_value(s)?;
    Some((v.to_string(), n))
}

fn parse_gemma_value(s: &str) -> Option<(serde_json::Value, usize)> {
    let t = s.trim_start();
    let pad = s.len() - t.len();
    if t.starts_with("<|\"|>") {
        let inner = &t["<|\"|>".len()..];
        let end = inner.find("<|\"|>")?;
        let v = serde_json::Value::String(inner[..end].to_string());
        return Some((v, pad + "<|\"|>".len() + end + "<|\"|>".len()));
    }
    if t.starts_with('{') {
        let mut i = 1;
        let mut map = serde_json::Map::new();
        loop {
            let rest = t[i..].trim_start();
            i = t.len() - rest.len();
            if rest.starts_with('}') {
                return Some((serde_json::Value::Object(map), pad + i + 1));
            }
            if rest.is_empty() {
                return None;
            }
            let (key, klen) = parse_gemma_key(rest)?;
            i += klen;
            let rest = t[i..].trim_start();
            if !rest.starts_with(':') {
                return None;
            }
            i = t.len() - rest.len() + 1;
            let (val, vlen) = parse_gemma_value(&t[i..])?;
            i += vlen;
            map.insert(key, val);
            let rest = t[i..].trim_start();
            i = t.len() - rest.len();
            if rest.starts_with(',') {
                i += 1;
            } else if rest.starts_with('}') {
                return Some((serde_json::Value::Object(map), pad + i + 1));
            } else {
                return None;
            }
        }
    }
    if t.starts_with('[') {
        let mut i = 1;
        let mut arr = Vec::new();
        loop {
            let rest = t[i..].trim_start();
            i = t.len() - rest.len();
            if rest.starts_with(']') {
                return Some((serde_json::Value::Array(arr), pad + i + 1));
            }
            let (val, vlen) = parse_gemma_value(&t[i..])?;
            i += vlen;
            arr.push(val);
            let rest = t[i..].trim_start();
            i = t.len() - rest.len();
            if rest.starts_with(',') {
                i += 1;
            } else if rest.starts_with(']') {
                return Some((serde_json::Value::Array(arr), pad + i + 1));
            } else {
                return None;
            }
        }
    }
    if t.starts_with("true") {
        return Some((serde_json::Value::Bool(true), pad + 4));
    }
    if t.starts_with("false") {
        return Some((serde_json::Value::Bool(false), pad + 5));
    }
    if t.starts_with("null") {
        return Some((serde_json::Value::Null, pad + 4));
    }
    let mut nend = 0;
    for (idx, c) in t.char_indices() {
        if c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e' || c == 'E' {
            nend = idx + c.len_utf8();
        } else {
            break;
        }
    }
    if nend > 0 {
        let num: serde_json::Number = t[..nend].parse().ok()?;
        return Some((serde_json::Value::Number(num), pad + nend));
    }
    None
}

fn parse_gemma_key(s: &str) -> Option<(String, usize)> {
    let t = s.trim_start();
    let pad = s.len() - t.len();
    if t.starts_with("<|\"|>") {
        let inner = &t["<|\"|>".len()..];
        let end = inner.find("<|\"|>")?;
        return Some((
            inner[..end].to_string(),
            pad + "<|\"|>".len() + end + "<|\"|>".len(),
        ));
    }
    let mut n = 0;
    for (idx, c) in t.char_indices() {
        if c.is_ascii_alphanumeric() || c == '_' {
            n = idx + c.len_utf8();
        } else {
            break;
        }
    }
    if n == 0 {
        return None;
    }
    Some((t[..n].to_string(), pad + n))
}

/// Text before the first Gemma `<|tool_call>`, if any.
pub fn text_before_gemma_tool_calls(text: &str) -> Option<String> {
    match text.find("<|tool_call>") {
        Some(0) | None => None,
        Some(i) => {
            let s = strip_gemma_thinking(&text[..i]);
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
    }
}

fn strip_gemma_thinking(text: &str) -> String {
    let mut rest = text;
    let mut out = String::new();
    while let Some(start) = rest.find("<|channel>thought") {
        out.push_str(&rest[..start]);
        let after = &rest[start + "<|channel>thought".len()..];
        if let Some(end) = after.find("<channel|>") {
            rest = &after[end + "<channel|>".len()..];
        } else {
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// Text before the first `<tool_call>`, if any.
pub fn text_before_tool_calls(text: &str) -> Option<String> {
    match text.find("<tool_call>") {
        Some(0) | None => None,
        Some(i) => {
            let s = text[..i].trim();
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_gemma_chat_template, apply_gemma_chat_template_with_tools, apply_qwen_chat_template,
        apply_qwen_chat_template_with_tools, parse_gemma_tool_calls, parse_hermes_tool_calls,
        text_before_gemma_tool_calls, text_before_tool_calls,
    };
    use crate::openai::{
        ChatMessage, Content, IncomingFunction, IncomingToolCall,
    };

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
    fn user_turn_opens_assistant() {
        let s = apply_qwen_chat_template(&[msg("user", "hi")]);
        assert_eq!(
            s,
            "<|im_start|>user\nhi<|im_end|>\n<|im_start|>assistant\n"
        );
    }

    #[test]
    fn system_and_user() {
        let s = apply_qwen_chat_template(&[msg("system", "be brief"), msg("user", "2+2")]);
        assert!(s.starts_with("<|im_start|>system\nbe brief<|im_end|>\n"));
        assert!(s.ends_with("<|im_start|>assistant\n"));
    }

    #[test]
    fn assistant_prefill_skips_extra_header() {
        let s = apply_qwen_chat_template(&[msg("user", "hi"), msg("assistant", "Hello")]);
        assert!(s.ends_with("<|im_start|>assistant\nHello<|im_end|>\n"));
        assert!(!s.ends_with("<|im_start|>assistant\n"));
    }

    fn sample_tool() -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "propose_command",
                "description": "Suggest a CLI command",
                "parameters": {
                    "type": "object",
                    "properties": { "command": { "type": "string" } }
                }
            }
        })
    }

    #[test]
    fn tools_inject_hermes_block() {
        let tools = [sample_tool()];
        let s = apply_qwen_chat_template_with_tools(
            &[msg("system", "You help."), msg("user", "show version")],
            Some(&tools),
            None,
        );
        assert!(s.contains("# Tools"));
        assert!(s.contains("You may call one or more functions"));
        assert!(s.contains("<tools>"));
        assert!(s.contains("propose_command"));
        assert!(s.contains("</tools>"));
        assert!(s.contains("<tool_call>"));
        assert!(s.contains(r#"{"name": <function-name>, "arguments": <args-json-object>}"#));
        assert!(s.contains("You help."));
        assert!(s.contains("<|im_start|>user\nshow version<|im_end|>\n"));
        assert!(s.ends_with("<|im_start|>assistant\n"));
        assert!(!s.contains("<|im_start|>system\nYou help.<|im_end|>\n"));
    }

    #[test]
    fn empty_tools_matches_plain_template() {
        let msgs = [msg("user", "hi")];
        let a = apply_qwen_chat_template(&msgs);
        let b = apply_qwen_chat_template_with_tools(&msgs, Some(&[]), None);
        assert_eq!(a, b);
    }

    #[test]
    fn parse_tool_call_arguments_are_string() {
        let raw = "<tool_call>\n{\"name\": \"propose_command\", \"arguments\": {\"command\": \"show vlan\"}}\n</tool_call>";
        let calls = parse_hermes_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_0");
        assert_eq!(calls[0].kind, "function");
        assert_eq!(calls[0].function.name, "propose_command");
        let args: serde_json::Value =
            serde_json::from_str(&calls[0].function.arguments).expect("arguments json");
        assert_eq!(args["command"], "show vlan");
        assert!(calls[0].function.arguments.starts_with('{'));
    }

    #[test]
    fn parse_string_arguments_stays_string() {
        let raw = r#"<tool_call>{"name":"ping","arguments":"{\"host\":\"1.1.1.1\"}"}</tool_call>"#;
        let calls = parse_hermes_tool_calls(raw);
        assert_eq!(calls[0].function.arguments, r#"{"host":"1.1.1.1"}"#);
    }

    #[test]
    fn parse_plain_text_is_empty() {
        assert!(parse_hermes_tool_calls("Just a normal reply.").is_empty());
        assert!(text_before_tool_calls("Just a normal reply.").is_none());
    }

    #[test]
    fn parse_two_calls() {
        let raw = "<tool_call>{\"name\":\"a\",\"arguments\":{}}</tool_call>\n<tool_call>{\"name\":\"b\",\"arguments\":{\"x\":1}}</tool_call>";
        let calls = parse_hermes_tool_calls(raw);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.name, "a");
        assert_eq!(calls[1].id, "call_1");
        assert_eq!(calls[1].function.name, "b");
    }

    #[test]
    fn parse_pretty_json_with_newlines() {
        let raw = r#"Let me look.

<tool_call>
{
  "name": "propose_command",
  "arguments": {
    "command": "show vlan"
  }
}
</tool_call>"#;
        let calls = parse_hermes_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_0");
        assert_eq!(calls[0].function.name, "propose_command");
        let args: serde_json::Value =
            serde_json::from_str(&calls[0].function.arguments).expect("arguments json");
        assert_eq!(args["command"], "show vlan");
        assert_eq!(
            text_before_tool_calls(raw).as_deref(),
            Some("Let me look.")
        );
    }

    #[test]
    fn parse_missing_arguments_becomes_empty_object() {
        let raw = r#"<tool_call>{"name": "status"}</tool_call>"#;
        let calls = parse_hermes_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.arguments, "{}");
    }

    #[test]
    fn parse_array_arguments_become_json_string() {
        let raw = r#"<tool_call>{"name":"batch","arguments":[1,2]}</tool_call>"#;
        let calls = parse_hermes_tool_calls(raw);
        assert_eq!(calls[0].function.arguments, "[1,2]");
    }

    #[test]
    fn parse_invalid_tool_call_body_is_skipped_not_error() {
        let raw = r#"<tool_call>not-json</tool_call> leftover"#;
        assert!(parse_hermes_tool_calls(raw).is_empty());
    }

    #[test]
    fn inbound_string_arguments_are_not_double_encoded() {
        let assistant = ChatMessage {
            role: "assistant".into(),
            content: Content::None,
            name: None,
            tool_call_id: None,
            tool_calls: Some(vec![IncomingToolCall {
                id: Some("call_0".into()),
                function: Some(IncomingFunction {
                    name: Some("propose_command".into()),
                    arguments: Some(serde_json::Value::String(
                        r#"{"command":"show ip"}"#.into(),
                    )),
                }),
                name: None,
                arguments: None,
            }]),
        };
        let s = apply_qwen_chat_template(&[msg("user", "go"), assistant]);
        assert!(s.contains(r#""name":"propose_command""#));
        assert!(s.contains(r#""arguments":{"command":"show ip"}"#));
        assert!(!s.contains(r#""arguments":"{\"command\""#));
        assert!(!s.contains("<|im_start|>tool"));
    }

    #[test]
    fn tool_role_is_user_tool_response() {
        let tool = ChatMessage {
            role: "tool".into(),
            content: Content::Text("ok".into()),
            name: None,
            tool_call_id: Some("call_0".into()),
            tool_calls: None,
        };
        let s = apply_qwen_chat_template(&[msg("user", "go"), tool]);
        assert!(s.contains("<|im_start|>user\n<tool_response>\nok\n</tool_response>"));
        assert!(!s.contains("<|im_start|>tool"));
    }

    #[test]
    fn assistant_history_with_tool_calls() {
        let assistant = ChatMessage {
            role: "assistant".into(),
            content: Content::None,
            name: None,
            tool_call_id: None,
            tool_calls: Some(vec![IncomingToolCall {
                id: Some("call_0".into()),
                function: Some(IncomingFunction {
                    name: Some("propose_command".into()),
                    arguments: Some(serde_json::json!({"command": "show ip"})),
                }),
                name: None,
                arguments: None,
            }]),
        };
        let tool = ChatMessage {
            role: "tool".into(),
            content: Content::Text("ok".into()),
            name: None,
            tool_call_id: Some("call_0".into()),
            tool_calls: None,
        };
        let s = apply_qwen_chat_template(&[msg("user", "go"), assistant, tool]);
        assert!(s.contains("<tool_call>"));
        assert!(s.contains("propose_command"));
        assert!(s.contains("<tool_response>\nok\n</tool_response>"));
        assert!(s.ends_with("<|im_start|>assistant\n"));
    }

    #[test]
    fn gemma_user_turn_opens_model() {
        let s = apply_gemma_chat_template(&[msg("user", "hi")]);
        assert!(s.starts_with("<bos>"));
        assert!(s.contains("<|turn>user\nhi<turn|>\n"));
        assert!(s.ends_with("<|turn>model\n"));
        assert!(!s.contains("<|im_start|>"));
    }

    #[test]
    fn gemma_tools_are_not_hermes() {
        let tools = [sample_tool()];
        let s = apply_gemma_chat_template_with_tools(
            &[msg("system", "You help."), msg("user", "show version")],
            Some(&tools),
            None,
        );
        assert!(s.contains("<|turn>system\nYou help."));
        assert!(s.contains("<|tool>declaration:propose_command"));
        assert!(s.contains("<tool|>"));
        assert!(s.contains("<|turn>user\nshow version<turn|>\n"));
        assert!(s.ends_with("<|turn>model\n"));
        assert!(!s.contains("<|im_start|>"));
        assert!(!s.contains("# Tools"));
        assert!(!s.contains("</tools>"));
    }

    #[test]
    fn parse_gemma_call_arguments_are_json_string() {
        let raw = "<|tool_call>call:propose_command{command:<|\"|>show vlan<|\"|>}<tool_call|>";
        let calls = parse_gemma_tool_calls(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "propose_command");
        let args: serde_json::Value =
            serde_json::from_str(&calls[0].function.arguments).expect("arguments json");
        assert_eq!(args["command"], "show vlan");
        assert!(parse_hermes_tool_calls(raw).is_empty());
    }

    #[test]
    fn parse_gemma_plain_text_is_empty() {
        assert!(parse_gemma_tool_calls("Just a normal reply.").is_empty());
        assert!(text_before_gemma_tool_calls("Just a normal reply.").is_none());
    }

    #[test]
    fn parse_gemma_two_calls_and_preamble() {
        let raw = "Let me look.\n<|tool_call>call:a{}<tool_call|><|tool_call>call:b{x:1}<tool_call|>";
        let calls = parse_gemma_tool_calls(raw);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.name, "a");
        assert_eq!(calls[1].function.name, "b");
        assert_eq!(
            text_before_gemma_tool_calls(raw).as_deref(),
            Some("Let me look.")
        );
    }
}
