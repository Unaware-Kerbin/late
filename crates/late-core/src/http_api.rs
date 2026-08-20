use crate::error::{LateError, Result};
use crate::redact::Redactor;
use crate::types::ApiController;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRequest {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub query: HashMap<String, String>,
    #[serde(default)]
    pub body: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Value,
    pub redacted_body: Value,
    pub elapsed_ms: u128,
}

pub fn method_allowed_for_agent(method: &str, controller: ApiController) -> bool {
    if controller == ApiController::Fortimanager {
        return false;
    }
    method.eq_ignore_ascii_case("GET")
}

pub fn host_pinned(request_url: &str, base: &str) -> bool {
    fn host_of(u: &str) -> Option<String> {
        let u = reqwest::Url::parse(u).ok()?;
        Some(format!("{}:{}", u.host_str()?, u.port_or_known_default()?))
    }
    match (host_of(request_url), host_of(base)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

pub async fn send_request(
    req: ApiRequest,
    extra_headers: HashMap<String, String>,
    insecure_tls: bool,
) -> Result<ApiResponse> {
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(insecure_tls)
        .build()
        .map_err(|e| LateError::Http(e.to_string()))?;
    let method = reqwest::Method::from_bytes(req.method.as_bytes())
        .map_err(|e| LateError::Http(e.to_string()))?;
    let mut b = client.request(method, &req.url);
    for (k, v) in extra_headers.iter().chain(req.headers.iter()) {
        b = b.header(k, v);
    }
    if !req.query.is_empty() {
        b = b.query(&req.query);
    }
    if let Some(body) = &req.body {
        b = b.json(body);
    }
    let start = std::time::Instant::now();
    let resp = b.send().await.map_err(|e| LateError::Http(e.to_string()))?;
    let status = resp.status().as_u16();
    let mut headers = HashMap::new();
    for (k, v) in resp.headers() {
        headers.insert(k.to_string(), v.to_str().unwrap_or("").to_string());
    }
    let text = resp
        .text()
        .await
        .map_err(|e| LateError::Http(e.to_string()))?;
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::String(text.clone()));
    let mut redactor = Redactor::new();
    let redacted_text = redactor.redact(&text);
    let redacted_body: Value =
        serde_json::from_str(&redacted_text).unwrap_or(Value::String(redacted_text));
    Ok(ApiResponse {
        status,
        headers,
        body,
        redacted_body,
        elapsed_ms: start.elapsed().as_millis(),
    })
}

#[derive(Debug, Deserialize)]
struct PostmanCollection {
    #[serde(default)]
    item: Vec<PostmanItem>,
}

#[derive(Debug, Deserialize)]
struct PostmanItem {
    #[serde(default)]
    name: String,
    #[serde(default)]
    item: Vec<PostmanItem>,
    request: Option<PostmanRequest>,
}

#[derive(Debug, Deserialize)]
struct PostmanRequest {
    #[serde(default)]
    method: String,
    url: Option<PostmanUrl>,
    #[serde(default)]
    header: Vec<PostmanHeader>,
    body: Option<PostmanBody>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PostmanUrl {
    Raw(String),
    Obj { raw: Option<String> },
}

#[derive(Debug, Deserialize)]
struct PostmanHeader {
    key: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct PostmanBody {
    #[serde(default)]
    raw: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportedRequest {
    pub name: String,
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Value>,
}

pub fn import_postman(json: &str) -> Result<Vec<ImportedRequest>> {
    let col: PostmanCollection = serde_json::from_str(json)?;
    let mut out = Vec::new();
    walk_items(&col.item, &mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ApiController;

    #[test]
    fn agent_may_only_get() {
        assert!(method_allowed_for_agent("GET", ApiController::Generic));
        assert!(method_allowed_for_agent("get", ApiController::Unifi));
        assert!(!method_allowed_for_agent("POST", ApiController::Generic));
        assert!(!method_allowed_for_agent(
            "GET",
            ApiController::Fortimanager
        ));
    }

    #[test]
    fn host_pin_matches_port() {
        assert!(host_pinned(
            "https://ctrl.example/api/v1/status",
            "https://ctrl.example"
        ));
        assert!(!host_pinned(
            "https://evil.example/api/v1/status",
            "https://ctrl.example"
        ));
        assert!(!host_pinned("not-a-url", "https://ctrl.example"));
    }
}

fn walk_items(items: &[PostmanItem], out: &mut Vec<ImportedRequest>) {
    for it in items {
        if !it.item.is_empty() {
            walk_items(&it.item, out);
        }
        if let Some(req) = &it.request {
            let url = match &req.url {
                Some(PostmanUrl::Raw(s)) => s.clone(),
                Some(PostmanUrl::Obj { raw }) => raw.clone().unwrap_or_default(),
                None => String::new(),
            };
            let mut headers = HashMap::new();
            for h in &req.header {
                let k = h.key.to_ascii_lowercase();
                if k == "authorization" || k == "x-api-key" || k.ends_with("-token") {
                    continue;
                }
                headers.insert(h.key.clone(), h.value.clone());
            }
            let body = req
                .body
                .as_ref()
                .and_then(|b| b.raw.as_ref().and_then(|r| serde_json::from_str(r).ok()));
            out.push(ImportedRequest {
                name: it.name.clone(),
                method: req.method.clone(),
                url,
                headers,
                body,
            });
        }
    }
}
