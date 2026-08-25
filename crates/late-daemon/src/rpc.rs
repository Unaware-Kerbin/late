use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use late_core::http_api::ApiRequest;
use late_core::{
    sftp, App, AppSettings, AuthProfile, CommandCollection, Device, LateError, OpenSession,
    SessionKind, Vendor,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::path::PathBuf;

#[derive(Deserialize)]
struct RpcReq {
    #[serde(default)]
    id: Value,
    #[serde(default)]
    method: String,
    #[serde(default)]
    params: Value,
}

pub async fn handle(app: &App, raw: &str) -> String {
    let parsed: Result<RpcReq, _> = serde_json::from_str(raw);
    let req = match parsed {
        Ok(r) => r,
        Err(e) => {
            return json!({"id": Value::Null, "error": {"code": -32700, "message": e.to_string()}})
                .to_string();
        }
    };
    if req.method.is_empty() {
        return json!({"id": req.id, "error": {"code": -32600, "message": "missing method"}})
            .to_string();
    }
    match dispatch(app, &req.method, to_snake(req.params)).await {
        Ok(result) => {
            let _ = late_core::audit::append(&app.paths, &req.method, true);
            json!({"id": req.id, "result": to_camel(result)}).to_string()
        }
        Err(e) => {
            let _ = late_core::audit::append(&app.paths, &req.method, false);
            let mut err = json!({"code": e.rpc_code(), "message": e.to_string()});
            if let Some(data) = e.rpc_data() {
                err["data"] = data;
            }
            json!({"id": req.id, "error": err}).to_string()
        }
    }
}

async fn dispatch(app: &App, method: &str, params: Value) -> Result<Value, LateError> {
    match method {
        "inventory.list" => Ok(serde_json::to_value(app.inventory.load()?)?),
        "inventory.upsert" => {
            let device: Device = serde_json::from_value(unwrap_obj(&params, "device"))?;
            Ok(serde_json::to_value(app.inventory.upsert_device(device)?)?)
        }
        "inventory.delete" => {
            let id = req_str(&params, &["id", "device_id"])?;
            app.inventory.delete_device(&id)?;
            Ok(json!({"ok": true}))
        }
        "inventory.folder.upsert" => {
            let path = req_str(&params, &["path", "folder"])?;
            Ok(serde_json::to_value(app.inventory.upsert_folder(&path)?)?)
        }
        "inventory.folder.rename" => {
            let from = req_str(&params, &["from", "old", "path"])?;
            let to = req_str(&params, &["to", "new", "name"])?;
            Ok(serde_json::to_value(
                app.inventory.rename_folder(&from, &to)?,
            )?)
        }
        "inventory.folder.delete" => {
            let path = req_str(&params, &["path", "folder"])?;
            Ok(serde_json::to_value(app.inventory.delete_folder(&path)?)?)
        }
        "auth.list" => Ok(serde_json::to_value(app.inventory.load_auth()?)?),
        "auth.upsert" => auth_upsert(app, params),
        "auth.delete" => {
            let id = req_str(&params, &["id", "profile_id"])?;
            let _ = app.secrets.delete(&id);
            app.inventory.delete_auth(&id)?;
            Ok(json!({"ok": true}))
        }
        "settings.get" => Ok(serde_json::to_value(app.settings())?),
        "settings.set" => {
            let settings: AppSettings = serde_json::from_value(unwrap_obj(&params, "settings"))?;
            app.set_settings(settings.clone())?;
            Ok(serde_json::to_value(settings)?)
        }
        "providers.status" => app.provider_status(),
        "providers.set" => {
            let name = req_str(&params, &["name", "provider"])?;
            let key = req_str(&params, &["key", "token"])?;
            app.set_provider_key(&name, &key)?;
            Ok(json!({"ok": true}))
        }
        "providers.delete" => {
            app.delete_provider_key(&req_str(&params, &["name", "provider"])?)?;
            Ok(json!({"ok": true}))
        }
        "inference.status" => {
            let engine = late_core::weights::parse_engine(
                &pstr(&params, &["engine", "backend"]).unwrap_or_default(),
            )?;
            Ok(serde_json::to_value(late_core::weights::status_for(
                engine,
                &app.settings(),
            ))?)
        }
        "inference.start" => {
            let engine = late_core::weights::parse_engine(
                &pstr(&params, &["engine", "backend"]).unwrap_or_default(),
            )?;
            let settings = app.settings();
            let model = pstr(&params, &["model", "serve_model", "id"]).unwrap_or_else(|| {
                match engine {
                    late_core::weights::Engine::Vllm => settings.vllm_model.clone(),
                    late_core::weights::Engine::LlamaCpp => settings.llama_cpp_model.clone(),
                    late_core::weights::Engine::Ollama => settings.ollama_model.clone(),
                }
            });
            Ok(serde_json::to_value(late_core::weights::start(
                engine, &model, &settings,
            )?)?)
        }
        "inference.stop" => {
            let engine = late_core::weights::parse_engine(
                &pstr(&params, &["engine", "backend"]).unwrap_or_default(),
            )?;
            Ok(serde_json::to_value(late_core::weights::stop(engine)?)?)
        }
        "inference.download" => {
            let engine = late_core::weights::parse_engine(
                &pstr(&params, &["engine", "backend"]).unwrap_or_default(),
            )?;
            let model = req_str(&params, &["model", "id"])?;
            Ok(serde_json::to_value(late_core::weights::download(
                engine,
                &model,
                &app.settings(),
            )?)?)
        }
        "session.open" => {
            let info = app.open_session(parse_open(&params)?)?;
            Ok(serde_json::to_value(info)?)
        }
        "known_hosts.accept" | "knownHosts.accept" => {
            let host = req_str(&params, &["host"])?;
            let fp = req_str(&params, &["fingerprint", "presented"])?;
            app.accept_host_key(&host, &fp)?;
            Ok(json!({"ok": true}))
        }
        "session.close" => {
            app.close_session(&req_str(&params, &["id", "session_id"])?)?;
            Ok(json!({"ok": true}))
        }
        "session.input" | "session.write" => {
            let id = req_str(&params, &["id", "session_id"])?;
            let data = req_str(&params, &["data"])?;
            let bytes = STANDARD
                .decode(data.trim())
                .map_err(|e| LateError::Message(format!("base64: {e}")))?;
            app.write(&id, &bytes)?;
            Ok(json!({"ok": true}))
        }
        "session.resize" => {
            let id = req_str(&params, &["id", "session_id"])?;
            let cols = params.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u32;
            let rows = params.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u32;
            app.resize(&id, cols, rows)?;
            Ok(json!({"ok": true}))
        }
        "session.reconnect" => {
            let info = app.reconnect(&req_str(&params, &["id", "session_id"])?)?;
            Ok(serde_json::to_value(info)?)
        }
        "session.list" => Ok(serde_json::to_value(app.list_sessions())?),
        "session.break" | "session.interrupt" => {
            app.send_break(&req_str(&params, &["id", "session_id"])?)?;
            Ok(json!({"ok": true}))
        }
        "session.setLogging" | "session.set_logging" => {
            let id = req_str(&params, &["id", "session_id"])?;
            let enabled = params
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let path = if enabled {
                let stem =
                    late_core::confine::safe_export_stem(&id).unwrap_or_else(|_| "session".into());
                Some(app.paths.data.join("logs").join(format!("{stem}.log")))
            } else {
                None
            };
            if let Some(p) = &path {
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            app.set_logging(&id, path)?;
            Ok(json!({"ok": true}))
        }
        "session.export" => {
            let id = req_str(&params, &["id", "session_id"])?;
            let name = pstr(&params, &["name"]).unwrap_or_else(|| "session".into());
            let pass = pstr(&params, &["encrypt_passphrase", "passphrase", "password"]);
            let path = app.export_capture(&id, &name, pass.as_deref(), true)?;
            Ok(json!({"path": path}))
        }
        "session.scrollback" => {
            let id = req_str(&params, &["id", "session_id"])?;
            let redacted = params
                .get("redacted")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let text = if redacted {
                app.redacted_scrollback(&id)?
            } else {
                app.raw_scrollback(&id)?
            };
            Ok(json!({"id": id, "text": text}))
        }
        "policy.check" => {
            if let Some(sid) = pstr(&params, &["session_id", "id"]) {
                let cmd = req_str(&params, &["command", "cmd"])?;
                Ok(serde_json::to_value(app.check_command(&sid, &cmd)?)?)
            } else {
                let vendor = Vendor::parse(&req_str(&params, &["vendor"])?);
                let cmd = req_str(&params, &["command", "cmd"])?;
                Ok(serde_json::to_value(app.check_policy(vendor, &cmd))?)
            }
        }
        "sftp.list" | "sftp.ls" => {
            let id = req_str(&params, &["id", "session_id"])?;
            let path = pstr(&params, &["path"]).unwrap_or_else(|| "/".into());
            let app = app.clone();
            blocking(move || Ok(serde_json::to_value(app.sftp_list(&id, &path)?)?)).await
        }
        "sftp.local" | "sftp.local_list" | "fs.list" => {
            let path = pstr(&params, &["path"]).unwrap_or_default();
            blocking(move || Ok(serde_json::to_value(sftp::list_local(&path)?)?)).await
        }
        "sftp.get" | "sftp.download" => {
            let id = req_str(&params, &["id", "session_id"])?;
            let remote = req_str(&params, &["remote", "src"])?;
            let local = req_str(&params, &["local", "dst"])?;
            let app = app.clone();
            blocking(move || {
                app.sftp_download(&id, &remote, &local)?;
                Ok(json!({"ok": true}))
            })
            .await
        }
        "sftp.put" | "sftp.upload" => {
            let id = req_str(&params, &["id", "session_id"])?;
            let local = req_str(&params, &["local", "src"])?;
            let remote = req_str(&params, &["remote", "dst"])?;
            let app = app.clone();
            blocking(move || {
                app.sftp_upload(&id, &local, &remote)?;
                Ok(json!({"ok": true}))
            })
            .await
        }
        "sftp.mkdir" => {
            let id = req_str(&params, &["id", "session_id"])?;
            let path = req_str(&params, &["path"])?;
            let app = app.clone();
            blocking(move || {
                app.sftp_mkdir(&id, &path)?;
                Ok(json!({"ok": true}))
            })
            .await
        }
        "sftp.rm" => {
            let dir = params.get("dir").and_then(|v| v.as_bool()).unwrap_or(false);
            let id = req_str(&params, &["id", "session_id"])?;
            let path = req_str(&params, &["path"])?;
            let app = app.clone();
            blocking(move || {
                app.sftp_remove(&id, &path, dir)?;
                Ok(json!({"ok": true}))
            })
            .await
        }
        "pcap.interfaces" | "pcap.ifs" | "pcap.nics" => Ok(json!(app.pcap_interfaces())),
        "serial.list" | "serial.ports" | "serial.enumerate" => Ok(json!(app.serial_ports())),
        "pcap.start" => {
            let iface = req_str(&params, &["iface", "interface"])?;
            let bpf = pstr(&params, &["bpf", "filter"]);
            let count = params
                .get("count")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32);
            let app = app.clone();
            blocking(move || Ok(serde_json::to_value(app.start_pcap(&iface, bpf, count)?)?)).await
        }
        "pcap.stop" => {
            let id = req_str(&params, &["id", "session_id", "capture_id"])?;
            let app = app.clone();
            blocking(move || app.stop_pcap(&id)).await
        }
        "pcap.open" => {
            let path = PathBuf::from(req_str(&params, &["path"])?);
            let app = app.clone();
            blocking(move || app.open_pcap(path)).await
        }
        "pcap.list" | "pcap.files" | "pcap.saved" => {
            let app = app.clone();
            blocking(move || app.list_pcaps()).await
        }
        "pcap.remote.start" | "pcap.ssh.start" => {
            let device_id = pstr(&params, &["device_id"]);
            let session_id = pstr(&params, &["session_id"]);
            let auth_profile_id = pstr(&params, &["auth_profile_id"]);
            let iface = pstr(&params, &["iface", "interface"]).unwrap_or_else(|| "any".into());
            let bpf = pstr(&params, &["bpf", "filter"]);
            let app = app.clone();
            blocking(move || {
                app.start_ssh_pcap(
                    device_id.as_deref(),
                    session_id.as_deref(),
                    &iface,
                    bpf.as_deref(),
                    auth_profile_id.as_deref(),
                )
            })
            .await
        }
        "pcap.remote" | "pcap.ssh" => {
            let device_id = pstr(&params, &["device_id"]);
            let session_id = pstr(&params, &["session_id"]);
            let auth_profile_id = pstr(&params, &["auth_profile_id"]);
            let iface = pstr(&params, &["iface", "interface"]).unwrap_or_else(|| "any".into());
            let count = params.get("count").and_then(|v| v.as_u64()).unwrap_or(200) as u32;
            let bpf = pstr(&params, &["bpf", "filter"]);
            let app = app.clone();
            blocking(move || {
                app.capture_ssh_pcap(
                    device_id.as_deref(),
                    session_id.as_deref(),
                    &iface,
                    count,
                    bpf.as_deref(),
                    auth_profile_id.as_deref(),
                )
            })
            .await
        }
        "pcap.wireshark" | "pcap.open_wireshark" | "pcap.openWireshark" => {
            let session_id = pstr(&params, &["session_id", "id"]);
            let path = pstr(&params, &["path"]);
            let app = app.clone();
            blocking(move || {
                app.open_pcap_in_wireshark(session_id.as_deref(), path.as_deref())?;
                Ok(json!({"ok": true}))
            })
            .await
        }
        "pcap.edgeshark.status" | "edgeshark.status" => {
            Ok(serde_json::to_value(late_core::edgeshark::status())?)
        }
        "pcap.edgeshark.start" | "edgeshark.start" => {
            blocking(|| Ok(serde_json::to_value(late_core::edgeshark::start()?)?)).await
        }
        "pcap.edgeshark.stop" | "edgeshark.stop" => {
            blocking(|| Ok(serde_json::to_value(late_core::edgeshark::stop()?)?)).await
        }
        "pcap.packets" | "pcap.filter" => {
            let id = req_str(&params, &["id", "session_id"])?;
            if let Some(expr) = pstr(&params, &["filter", "expr"]) {
                Ok(serde_json::to_value(app.pcap_filter(&id, &expr)?)?)
            } else {
                Ok(serde_json::to_value(app.pcap_packets(&id)?)?)
            }
        }
        "pcap.findings" => {
            let id = req_str(&params, &["id", "session_id"])?;
            Ok(serde_json::to_value(app.pcap_findings(&id)?)?)
        }
        "pcap.query" => {
            let id = req_str(&params, &["id", "session_id"])?;
            let q = req_str(&params, &["q", "query"])?;
            app.pcap_query(&id, &q)
        }
        "api.request" => {
            let device_id = if let Some(d) = pstr(&params, &["device_id"]) {
                d
            } else {
                app.session_device_id(&req_str(&params, &["session_id", "id"])?)?
            };
            let req = parse_api(&params)?;
            let agent = params
                .get("agent")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(serde_json::to_value(
                app.api_send(&device_id, req, agent).await?,
            )?)
        }
        "import.file" | "import.path" => {
            let path = PathBuf::from(req_str(&params, &["path"])?);
            let commit = params
                .get("commit")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(serde_json::to_value(app.import_file(&path, commit)?)?)
        }
        "collections.list" => Ok(serde_json::to_value(app.list_collections()?.collections)?),
        "collections.upsert" => {
            let col: CommandCollection = serde_json::from_value(unwrap_obj(&params, "collection"))?;
            Ok(serde_json::to_value(app.upsert_collection(col)?)?)
        }
        "collections.delete" => {
            app.delete_collection(&req_str(&params, &["id"])?)?;
            Ok(json!({"ok": true}))
        }
        "capture.list" => Ok(serde_json::to_value(app.list_captures()?)?),
        "capture.save" => {
            let id = req_str(&params, &["id", "session_id"])?;
            let name = pstr(&params, &["name"]).unwrap_or_else(|| "capture".into());
            Ok(serde_json::to_value(app.save_capture(&id, &name)?)?)
        }
        "capture.diff" => {
            let a = req_str(&params, &["a", "left"])?;
            let b = req_str(&params, &["b", "right"])?;
            Ok(serde_json::to_value(app.diff_captures(&a, &b)?)?)
        }
        "capture.export" => {
            let id = req_str(&params, &["id", "session_id"])?;
            let name = pstr(&params, &["name"]).unwrap_or_else(|| "session".into());
            let pass = pstr(&params, &["passphrase", "password"]);
            let redacted = params
                .get("redacted")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let path = app.export_capture(&id, &name, pass.as_deref(), redacted)?;
            Ok(json!({"path": path}))
        }
        "stage.render" => {
            let art = app.stage_render(
                &req_str(&params, &["format"])?,
                &pstr(&params, &["intent"]).unwrap_or_default(),
                pstr(&params, &["body"]).as_deref(),
                pstr(&params, &["device_id", "deviceId"]).as_deref(),
                pstr(&params, &["session_id", "sessionId"]).as_deref(),
            )?;
            Ok(serde_json::to_value(art)?)
        }
        "stage.save" => {
            let art = app.stage_save(
                &req_str(&params, &["format"])?,
                &pstr(&params, &["intent"]).unwrap_or_default(),
                pstr(&params, &["body"]).as_deref(),
                pstr(&params, &["device_id", "deviceId"]).as_deref(),
                pstr(&params, &["session_id", "sessionId"]).as_deref(),
                pstr(&params, &["id"]).as_deref(),
            )?;
            Ok(serde_json::to_value(art)?)
        }
        "stage.get" => Ok(serde_json::to_value(
            app.stage_get(&req_str(&params, &["id"])?)?,
        )?),
        "stage.list" => Ok(serde_json::to_value(app.stage_list()?)?),
        "stage.plan" => Ok(serde_json::to_value(app.stage_plan(
            pstr(&params, &["id"]).as_deref(),
            &pstr(&params, &["format"]).unwrap_or_else(|| "cli".into()),
            &pstr(&params, &["intent"]).unwrap_or_default(),
            pstr(&params, &["body"]).as_deref(),
            pstr(&params, &["device_id", "deviceId"]).as_deref(),
            pstr(&params, &["session_id", "sessionId"]).as_deref(),
        )?)?),
        "stage.push" => {
            let app = app.clone();
            let id = pstr(&params, &["id"]);
            let format = pstr(&params, &["format"]).unwrap_or_else(|| "cli".into());
            let intent = pstr(&params, &["intent"]).unwrap_or_default();
            let body = pstr(&params, &["body"]);
            let device_id = pstr(&params, &["device_id", "deviceId"]);
            let session_id = pstr(&params, &["session_id", "sessionId"]);
            blocking(move || {
                Ok(serde_json::to_value(app.stage_push(
                    id.as_deref(),
                    &format,
                    &intent,
                    body.as_deref(),
                    device_id.as_deref(),
                    session_id.as_deref(),
                )?)?)
            })
            .await
        }
        other => Err(LateError::Message(format!("unknown method: {other}"))),
    }
}

fn auth_upsert(app: &App, mut params: Value) -> Result<Value, LateError> {
    let password = params
        .get("password")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if let Some(obj) = params.as_object_mut() {
        obj.remove("password");
    }
    let mut profile: AuthProfile = serde_json::from_value(unwrap_obj(&params, "profile"))?;
    if let Some(pw) = password {
        if !pw.is_empty() {
            if profile.id.is_empty() {
                profile.id = uuid::Uuid::new_v4().to_string();
            }
            app.secrets.set(&profile.id, &pw)?;
            profile.has_password = true;
        }
    }
    let saved = app.inventory.upsert_auth(profile)?;
    Ok(serde_json::to_value(saved)?)
}

fn parse_open(params: &Value) -> Result<OpenSession, LateError> {
    let kind = parse_kind(&req_str(params, &["kind"])?)?;
    Ok(OpenSession {
        device_id: pstr(params, &["device_id"]),
        kind,
        accept_unknown_host: json_flag(params, &["accept_unknown_host", "acceptUnknownHost"]),
        replace_host_key: json_flag(params, &["replace_host_key", "replaceHostKey"]),
        cols: params.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u32,
        rows: params.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u32,
        shell: pstr(params, &["shell"]),
        path: pstr(params, &["path"]).map(PathBuf::from),
        iface: pstr(params, &["iface", "interface"]),
        bpf: pstr(params, &["bpf", "filter"]),
    })
}

fn parse_kind(s: &str) -> Result<SessionKind, LateError> {
    match s.to_ascii_lowercase().as_str() {
        "ssh" => Ok(SessionKind::Ssh),
        "serial" => Ok(SessionKind::Serial),
        "local" => Ok(SessionKind::Local),
        "sftp" => Ok(SessionKind::Sftp),
        "pcap" => Ok(SessionKind::Pcap),
        "api" => Ok(SessionKind::Api),
        other => Err(LateError::Message(format!("unknown session kind: {other}"))),
    }
}

fn parse_api(params: &Value) -> Result<ApiRequest, LateError> {
    let src = if params.get("method").is_some() && params.get("url").is_some() {
        params.clone()
    } else {
        unwrap_obj(params, "request")
    };
    serde_json::from_value(src).map_err(|e| LateError::Message(e.to_string()))
}

fn unwrap_obj(params: &Value, key: &str) -> Value {
    params.get(key).cloned().unwrap_or_else(|| params.clone())
}

fn json_flag(v: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .any(|k| v.get(*k).and_then(|x| x.as_bool()) == Some(true))
}

fn pstr(v: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(|x| x.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn req_str(v: &Value, keys: &[&str]) -> Result<String, LateError> {
    pstr(v, keys).ok_or_else(|| LateError::Message(format!("missing {}", keys.join("/"))))
}

fn to_snake(v: Value) -> Value {
    map_keys(v, camel_to_snake, false)
}

fn to_camel(v: Value) -> Value {
    map_keys(v, snake_to_camel, false)
}

fn map_keys(v: Value, f: fn(&str) -> String, skip: bool) -> Value {
    match v {
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, val) in map {
                let skip_children = skip
                    || k.eq_ignore_ascii_case("headers")
                    || k.eq_ignore_ascii_case("body")
                    || k.eq_ignore_ascii_case("redacted_body")
                    || k.eq_ignore_ascii_case("redactedBody")
                    || k.eq_ignore_ascii_case("query");
                let nk = if skip { k } else { f(&k) };
                out.insert(nk, map_keys(val, f, skip_children));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(|x| map_keys(x, f, skip)).collect()),
        other => other,
    }
}

async fn blocking<T, F>(f: F) -> Result<T, LateError>
where
    F: FnOnce() -> Result<T, LateError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| LateError::Message(format!("background task: {e}")))?
}

fn camel_to_snake(s: &str) -> String {
    if s.contains('-') || s.contains('/') {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        if c.is_uppercase() {
            if !out.is_empty() {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn snake_to_camel(s: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in s.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn folder_rename_reads_to_not_new() {
        // UI store.ts sends { from, to, old: from }. `new` is a reserved-ish JS key
        // so the client prefers `to`; req_str must read that first.
        let params = to_snake(json!({
            "from": "Sites/NYC",
            "to": "Sites/Boston",
            "old": "Sites/NYC"
        }));
        assert_eq!(
            req_str(&params, &["from", "old", "path"]).unwrap(),
            "Sites/NYC"
        );
        assert_eq!(
            req_str(&params, &["to", "new", "name"]).unwrap(),
            "Sites/Boston"
        );
        assert_eq!(
            params.get("to").and_then(|v| v.as_str()),
            Some("Sites/Boston")
        );
        assert!(params.get("new").is_none());
    }

    #[test]
    fn folder_rename_aliases_old_new_path_name() {
        let params = to_snake(json!({
            "old": "Sites/NYC",
            "new": "Sites/Boston"
        }));
        assert_eq!(
            req_str(&params, &["from", "old", "path"]).unwrap(),
            "Sites/NYC"
        );
        assert_eq!(
            req_str(&params, &["to", "new", "name"]).unwrap(),
            "Sites/Boston"
        );
        let params = to_snake(json!({
            "path": "Sites/NYC",
            "name": "Sites/Boston"
        }));
        assert_eq!(
            req_str(&params, &["from", "old", "path"]).unwrap(),
            "Sites/NYC"
        );
        assert_eq!(
            req_str(&params, &["to", "new", "name"]).unwrap(),
            "Sites/Boston"
        );
    }

    #[test]
    fn folder_upsert_delete_aliases() {
        let params = to_snake(json!({ "path": "Sites/NYC", "folder": "Sites/NYC" }));
        assert_eq!(req_str(&params, &["path", "folder"]).unwrap(), "Sites/NYC");
    }
}
