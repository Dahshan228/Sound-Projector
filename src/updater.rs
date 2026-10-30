use crate::net::{download, fetch_text, sha256, system_tool};
use eframe::egui;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::time::Duration;

const LATEST: &str = "https://api.github.com/repos/Dahshan228/Sound-Projector/releases/latest";
const ASSET: &str = "SoundProjectorSetup.exe";
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60); // the app can sit in the tray for days

#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    pub version: String,
    url: String,
    sha256: String,
}

fn version_parts(v: &str) -> Option<Vec<u32>> {
    v.trim_start_matches('v').split('.').map(|p| p.parse().ok()).collect()
}

fn is_newer(latest: &str, current: &str) -> bool {
    matches!((version_parts(latest), version_parts(current)), (Some(l), Some(c)) if l > c)
}

/// The update in GitHub's latest-release JSON, if it's newer. A release without a checksum is never offered.
fn parse_latest(json: &str, current: &str) -> Option<Update> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let tag = v["tag_name"].as_str()?;
    if !is_newer(tag, current) {
        return None;
    }
    let asset = v["assets"].as_array()?.iter().find(|a| a["name"] == ASSET)?;
    Some(Update {
        version: tag.trim_start_matches('v').to_string(),
        url: asset["browser_download_url"].as_str()?.to_string(),
        sha256: asset["digest"].as_str()?.strip_prefix("sha256:")?.to_lowercase(),
    })
}

/// Only an installed copy updates itself: a build run from anywhere else would gain a second copy.
fn is_installed() -> bool {
    std::env::current_exe().is_ok_and(|e| e.with_file_name("unins000.exe").exists())
}

pub fn watch(ctx: egui::Context, tx: Sender<Update>) {
    if !is_installed() {
        return;
    }
    std::thread::spawn(move || {
        loop {
            if let Some(u) = fetch_text(LATEST).ok().and_then(|j| parse_latest(&j, env!("CARGO_PKG_VERSION"))) {
                let _ = tx.send(u);
                ctx.request_repaint();
            }
            std::thread::sleep(CHECK_EVERY);
        }
    });
}

/// Outlives the app: Setup can't replace a running exe, and only something still running can reopen the app if Setup fails.
/// Setup reopens the app itself on success. Reading Handle is what makes PowerShell 5.1 keep the exit code.
const RUN_SETUP: &str = "Wait-Process -Id $env:SP_APP_PID -ErrorAction SilentlyContinue; \
    $p = Start-Process $env:SP_SETUP -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART' -PassThru; \
    $null = $p.Handle; $p.WaitForExit(); \
    if ($p.ExitCode -ne 0) { Start-Process $env:SP_APP -ArgumentList '--update-failed' }";

fn start_setup(setup: &Path, app: &Path, app_pid: u32) -> Result<(), String> {
    system_tool(r"WindowsPowerShell\v1.0\powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", RUN_SETUP])
        .env("SP_SETUP", setup)
        .env("SP_APP", app)
        .env("SP_APP_PID", app_pid.to_string())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Downloads and checks the installer, hands over to it, and quits. Any error here leaves the app running as it was.
pub fn install(u: &Update) -> Result<(), String> {
    let setup = std::env::temp_dir().join("SoundProjectorUpdate.exe");
    download(&u.url, &setup)?;
    let actual = sha256(&setup)?;
    if actual != u.sha256 {
        let _ = std::fs::remove_file(&setup);
        return Err("the download was damaged".into());
    }
    let app = std::env::current_exe().map_err(|e| e.to_string())?;
    start_setup(&setup, &app, std::process::id())?;
    // ponytail: settings are saved on every change, so a hard exit loses nothing but eframe's last window-geometry autosave
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "b26c811ee9ace5eab5c81044c8620e303ca2442278a9f4132d715925814396f1";

    fn release(tag: &str, name: &str, digest: &str) -> String {
        serde_json::json!({"tag_name": tag, "assets": [
            {"name": "notes.txt", "browser_download_url": "https://x/notes.txt", "digest": "sha256:00"},
            {"name": name, "browser_download_url": "https://x/setup.exe", "digest": digest},
        ]})
        .to_string()
    }

    #[test]
    fn compares_versions_as_numbers() {
        assert!(is_newer("v0.1.10", "0.1.9"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("v0.1.1", "0.1.1"));
        assert!(!is_newer("v0.1.0", "0.1.1"));
        assert!(!is_newer("nightly", "0.1.1"));
    }

    #[test]
    fn offers_only_a_newer_release_with_a_checksum() {
        let digest = format!("sha256:{HASH}");
        let u = parse_latest(&release("v0.2.0", ASSET, &digest), "0.1.1").unwrap();
        assert_eq!((u.version.as_str(), u.url.as_str(), u.sha256.as_str()), ("0.2.0", "https://x/setup.exe", HASH));
        assert_eq!(parse_latest(&release("v0.1.1", ASSET, &digest), "0.1.1"), None);
        assert_eq!(parse_latest(&release("v0.2.0", "Other.exe", &digest), "0.1.1"), None);
        assert_eq!(parse_latest(&release("v0.2.0", ASSET, "md5:abc"), "0.1.1"), None);
        assert_eq!(parse_latest("not json", "0.1.1"), None);
    }

    /// Runs the real script with a stand-in installer. Returns the arguments the app was reopened with, if it was.
    fn run_with_setup_exit_code(code: u32) -> Option<String> {
        let dir = crate::testutil::temp_dir("setup");
        let (setup, app, marker) = (dir.join("setup.bat"), dir.join("app.bat"), dir.join("reopened.txt"));
        std::fs::write(&setup, format!("@exit /b {code}\r\n")).unwrap();
        std::fs::write(&app, format!("@echo %* > \"{}\"\r\n", marker.display())).unwrap();
        let mut running_app = std::process::Command::new("ping").args(["-n", "2", "127.0.0.1"]).stdout(std::process::Stdio::null()).spawn().unwrap();
        start_setup(&setup, &app, running_app.id()).unwrap();
        let _ = running_app.wait();
        for _ in 0..40 {
            if let Ok(args) = std::fs::read_to_string(&marker) {
                return Some(args.trim().to_string());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        None
    }

    #[test]
    #[ignore] // opens console windows for a moment: cargo test setup_outcomes -- --ignored
    fn setup_outcomes() {
        assert_eq!(run_with_setup_exit_code(3).as_deref(), Some("--update-failed"));
        assert_eq!(run_with_setup_exit_code(0), None); // Setup reopens the app itself
    }
}
