use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::Manager;

struct Sidecars {
    children: Mutex<Vec<Child>>,
}

fn spawn_optional(bin: &str, args: &[&str]) -> Option<Child> {
    match Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => {
            eprintln!("late: spawned {bin}");
            Some(child)
        }
        Err(err) => {
            eprintln!("late: did not spawn {bin}: {err}");
            None
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|_app| {
            let mut kids = Vec::new();
            if std::env::var("LATE_SPAWN_SIDECARS").ok().as_deref() == Some("1") {
                if let Some(c) = spawn_optional("late-daemon", &[]) {
                    kids.push(c);
                }
                let sidecar = std::env::var("LATE_SIDECAR_JS").unwrap_or_else(|_| {
                    "apps/agent-sidecar/src/index.ts".into()
                });
                if let Some(c) = spawn_optional("npx", &["tsx", &sidecar]) {
                    kids.push(c);
                }
            }
            _app.manage(Sidecars {
                children: Mutex::new(kids),
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Late");
}
