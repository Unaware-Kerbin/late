use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct ChatRequest {
    #[serde(default)]
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub tool_choice: Option<ToolChoice>,
}

impl ChatRequest {
    /// Inject tools unless the list is empty or `tool_choice` is `none`.
    pub fn tools_active(&self) -> bool {
        let Some(tools) = self.tools.as_ref() else {
            return false;
        };
        if tools.is_empty() {
            return false;
        }
        !self.tool_choice.as_ref().is_some_and(ToolChoice::is_none)
    }

    /// Stream and empty messages are 400. Tools never are.
    pub fn reject_reason(&self) -> Option<(&'static str, &'static str)> {
        if self.stream.unwrap_or(false) {
            return Some((
                "late-infer does not stream yet; omit stream or set stream=false",
                "invalid_request_error",
            ));
        }
        if self.messages.is_empty() {
            return Some(("messages must not be empty", "invalid_request_error"));
        }
        None
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    Mode(String),
    Named(serde_json::Value),
}

impl ToolChoice {
    pub fn is_none(&self) -> bool {
        match self {
            ToolChoice::Mode(s) => s.eq_ignore_ascii_case("none"),
            ToolChoice::Named(_) => false,
        }
    }

    pub fn required_name(&self) -> Option<String> {
        match self {
            ToolChoice::Mode(s) if s.eq_ignore_ascii_case("required") => {
                Some(String::new())
            }
            ToolChoice::Named(v) => v
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(|s| s.to_string()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: Content,
    #[serde(default)]
    #[allow(dead_code)]
    pub name: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<IncomingToolCall>>,
}

impl ChatMessage {
    pub fn text(&self) -> String {
        self.content.as_text()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct IncomingToolCall {
    #[serde(default)]
    #[allow(dead_code)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<IncomingFunction>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<serde_json::Value>,
}

impl IncomingToolCall {
    pub fn name_and_args(&self) -> Option<(String, String)> {
        if let Some(f) = &self.function {
            let name = f.name.as_deref().unwrap_or("").trim();
            if name.is_empty() {
                return None;
            }
            return Some((name.to_string(), args_to_string(&f.arguments)));
        }
        let name = self.name.as_deref().unwrap_or("").trim();
        if name.is_empty() {
            return None;
        }
        Some((name.to_string(), args_to_string(&self.arguments)))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct IncomingFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<serde_json::Value>,
}

fn args_to_string(v: &Option<serde_json::Value>) -> String {
    match v {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Null) | None => "{}".into(),
        Some(other) => serde_json::to_string(other).unwrap_or_else(|_| "{}".into()),
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Parts(Vec<ContentPart>),
    None,
}

impl Default for Content {
    fn default() -> Self {
        Content::None
    }
}

impl Content {
    pub fn as_text(&self) -> String {
        match self {
            Content::Text(s) => s.clone(),
            Content::Parts(parts) => parts
                .iter()
                .filter_map(|p| p.text.as_deref())
                .collect::<Vec<_>>()
                .join(""),
            Content::None => String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContentPart {
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatResponse {
    pub id: String,
    pub object: &'static str,
    pub created: u64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Usage,
}

#[derive(Debug, Serialize)]
pub struct Choice {
    pub index: u32,
    pub message: ResponseMessage,
    pub finish_reason: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ResponseMessage {
    pub role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Serialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Serialize)]
pub struct ModelsResponse {
    pub object: &'static str,
    pub data: Vec<ModelCard>,
}

#[derive(Debug, Serialize)]
pub struct ModelCard {
    pub id: String,
    pub object: &'static str,
    pub owned_by: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
}

pub fn error_json(message: impl Into<String>, kind: &'static str) -> ErrorBody {
    ErrorBody {
        error: ErrorDetail {
            message: message.into(),
            kind,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{ChatRequest, ToolChoice};

    fn req_with(tools: Option<Vec<serde_json::Value>>, tool_choice: Option<ToolChoice>) -> ChatRequest {
        ChatRequest {
            model: String::new(),
            messages: vec![],
            max_tokens: None,
            temperature: None,
            top_p: None,
            stream: None,
            tools,
            tool_choice,
        }
    }

    #[test]
    fn tools_present_are_active_for_auto() {
        let tools = vec![serde_json::json!({"type":"function","function":{"name":"x"}})];
        let req = req_with(Some(tools.clone()), Some(ToolChoice::Mode("auto".into())));
        assert!(req.tools_active());
        let req = req_with(Some(tools), None);
        assert!(req.tools_active());
    }

    #[test]
    fn empty_or_none_choice_is_text() {
        let tools = vec![serde_json::json!({"type":"function","function":{"name":"x"}})];
        assert!(!req_with(Some(tools), Some(ToolChoice::Mode("none".into()))).tools_active());
        assert!(!req_with(Some(vec![]), Some(ToolChoice::Mode("auto".into()))).tools_active());
        assert!(!req_with(None, Some(ToolChoice::Mode("auto".into()))).tools_active());
    }

    #[test]
    fn tool_call_arguments_wire_as_json_string() {
        let tc = super::ToolCall {
            id: "call_0".into(),
            kind: "function",
            function: super::FunctionCall {
                name: "propose_command".into(),
                arguments: r#"{"command":"show vlan"}"#.into(),
            },
        };
        let v = serde_json::to_value(&tc).expect("serialize");
        assert_eq!(v["id"], "call_0");
        assert!(v["function"]["arguments"].is_string());
        assert_eq!(
            v["function"]["arguments"].as_str().unwrap(),
            r#"{"command":"show vlan"}"#
        );
    }

    #[test]
    fn sidecar_shaped_tools_auto_is_not_400() {
        let raw = r#"{
            "model": "Qwen/Qwen2.5-0.5B-Instruct",
            "messages": [{"role":"system","content":"You are Late."},{"role":"user","content":"show vlan"}],
            "tools": [{"type":"function","function":{"name":"propose_command","description":"Suggest a CLI command","parameters":{"type":"object","properties":{"command":{"type":"string"}}}}}],
            "tool_choice": "auto",
            "temperature": 0.2,
            "stream": false
        }"#;
        let req: ChatRequest = serde_json::from_str(raw).expect("sidecar body");
        assert!(req.tools_active());
        assert!(req.reject_reason().is_none());
    }

    #[test]
    fn stream_true_is_400_tools_are_not() {
        let tools = vec![serde_json::json!({"type":"function","function":{"name":"x"}})];
        let mut req = req_with(Some(tools), Some(ToolChoice::Mode("auto".into())));
        req.messages = vec![super::ChatMessage {
            role: "user".into(),
            content: super::Content::Text("hi".into()),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }];
        assert!(req.reject_reason().is_none());
        req.stream = Some(true);
        assert!(req.reject_reason().is_some());
    }
}

