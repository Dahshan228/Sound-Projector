use crate::library::import;
use crate::net::{download, hidden, run, system_tool};
use crate::settings::{PadId, Source, app_dir, sounds_dir};
use eframe::egui;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};

const YT_DLP: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
const FFMPEG: &str = "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-win64-gpl-shared.zip";
const DENO: &str = "https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip";
const CONNECTED: &str = "connected"; // written to the sign-in folder once a sign-in is confirmed
const SIGN_IN: &str = "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F";

pub struct Job {
    pub url: String,
    pub source_id: u32,
    pub pad_id: PadId,
    pub replaces: Option<u32>, // a song whose audio this upgrades, instead of a new pad
}

pub enum Work {
    Download(Job),
    Connect,
    Disconnect,
}

pub enum Event {
    Status(String),
    Account(bool, String), // connected or not, and what to tell the user
    Added(PadId, Source),
    Replaced(u32, Source),
    Failed(String),
}

/// The canonical link for a direct video link. Only this reaches yt-dlp, so nothing pasted can act as an option or another site.
pub fn video_url(input: &str) -> Result<String, String> {
    let bad = || "Paste a link to a single YouTube video.".to_string();
    let input = input.trim();
    let rest = input.strip_prefix("https://").or_else(|| input.strip_prefix("http://")).ok_or_else(bad)?;
    let rest = rest.split('#').next().unwrap_or_default();
    let (address, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (host, path) = address.split_once('/').unwrap_or((address, ""));
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let id = match (host.to_ascii_lowercase().as_str(), segments.as_slice()) {
        ("youtu.be", [id]) => *id,
        ("youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com", ["watch"]) => {
            query.split('&').find_map(|kv| kv.strip_prefix("v=")).ok_or_else(bad)?
        }
        ("youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com", ["shorts" | "embed" | "live", id]) => *id,
        _ => return Err(bad()),
    };
    if id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        Ok(format!("https://www.youtube.com/watch?v={id}"))
    } else {
        Err(bad())
    }
}

pub fn sign_in_dir() -> PathBuf {
    app_dir().join("youtube-signin")
}

pub fn connected(dir: &Path) -> bool {
    dir.join(CONNECTED).exists()
}

/// A sign-in that was never confirmed, such as quitting mid-way, shouldn't leave a session on disk.
fn clear_unfinished(dir: &Path) {
    if !connected(dir) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The marker goes first, so downloads stop using the account even if Edge still holds some files.
fn disconnect(dir: &Path) -> std::io::Result<()> {
    let _ = std::fs::remove_file(dir.join(CONNECTED));
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        r => r,
    }
}

/// Reads yt-dlp's answer for a page that needs an account. Only its login gate or unreadable cookies mean no sign-in.
fn sign_in_error(check: Result<String, String>) -> Result<(), String> {
    match check {
        Err(e) if e.contains("Login details") => Err("you didn't finish signing in.".into()),
        Err(e) if e.to_lowercase().contains("cookie") => Err("the sign-in couldn't be read. Close the Edge window and try again.".into()),
        _ => Ok(()),
    }
}

/// Premium-only songs live on YouTube Music.
// ponytail: Premium's 256 kbps audio still doesn't come through, because yt-dlp skips web_music's streams without a PO token. A token provider plugin would be the upgrade.
fn account_url(url: &str) -> String {
    url.replacen("https://www.youtube.com", "https://music.youtube.com", 1)
}

fn cookies_arg(dir: &Path) -> String {
    format!("edge:{}", dir.join("Default").display())
}

fn explain(error: String, connected: bool) -> String {
    let lower = error.to_lowercase();
    if connected && lower.contains("cookie") {
        "YouTube no longer accepts your sign-in. Disconnect and connect again in the Add from YouTube window.".into()
    } else if !connected && lower.contains("premium") {
        format!("{error}. If you have Premium, connect your YouTube account in the Add from YouTube window.")
    } else {
        error
    }
}

fn edge_exe() -> Result<PathBuf, String> {
    ["ProgramFiles(x86)", "ProgramFiles"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(|root| PathBuf::from(root).join(r"Microsoft\Edge\Application\msedge.exe"))
        .find(|p| p.exists())
        .ok_or_else(|| "signing in needs Microsoft Edge, which isn't installed.".to_string())
}

fn edge_args(dir: &Path) -> Vec<String> {
    // Without the last two, Edge signs this profile in to the Windows work or school account and offers to sync it.
    let flags = ["--no-first-run", "--no-default-browser-check", "--disable-sync", "--disable-features=msImplicitSignin"];
    flags.into_iter().map(String::from).chain([format!("--user-data-dir={}", dir.display()), SIGN_IN.into()]).collect()
}

fn yt_dlp(tools: &Path) -> Command {
    let mut c = hidden(tools.join("yt-dlp.exe"));
    c.arg("--ignore-config").arg("--js-runtimes").arg(format!("deno:{}", tools.join("deno").join("deno.exe").display()));
    c
}

/// Signs in with a real browser, because Google refuses embedded ones, on a profile of the app's own, because yt-dlp often can't read the everyday one.
fn connect(dir: &Path, tools: &Path, status: &mut dyn FnMut(String)) -> Result<(), String> {
    let edge = edge_exe()?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    status("Sign in to YouTube in the Edge window, then close that window.".into());
    Command::new(edge).args(edge_args(dir)).status().map_err(|e| e.to_string())?;
    status("Checking your sign-in…".into());
    // ":ytsubs" is a page yt-dlp refuses without an account.
    let check = run(yt_dlp(tools).arg("--cookies-from-browser").arg(cookies_arg(dir)).args(["--flat-playlist", "--playlist-items", "1", "--simulate", "--print", "id", "--", ":ytsubs"]));
    sign_in_error(check)?;
    File::create(dir.join(CONNECTED)).map(drop).map_err(|e| e.to_string())
}

/// Unpacks only once complete, so an interrupted download starts over instead of leaving half a tool.
fn fetch_zip(tools: &Path, name: &str, url: &str, tar_args: &[&str]) -> Result<(), String> {
    let (dest, zip, part) = (tools.join(name), tools.join(format!("{name}.zip")), tools.join(format!("{name}.unpacking")));
    if dest.exists() {
        return Ok(());
    }
    download(url, &zip)?;
    let _ = std::fs::remove_dir_all(&part);
    std::fs::create_dir_all(&part).map_err(|e| e.to_string())?;
    run(system_tool("tar.exe").arg("-xf").arg(&zip).arg("-C").arg(&part).args(tar_args))?;
    std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&zip);
    Ok(())
}

// ponytail: tools come over HTTPS from each project's own latest release with no pinned checksum, because yt-dlp must stay current to keep working. Pin them if that trust stops being enough.
fn ensure_tools(tools: &Path, status: &mut dyn FnMut(String)) -> Result<(), String> {
    let yt_dlp = tools.join("yt-dlp.exe");
    if yt_dlp.exists() && tools.join("ffmpeg").exists() && tools.join("deno").exists() {
        return Ok(());
    }
    status("Setting up YouTube import, first time only (about 160 MB)…".into());
    std::fs::create_dir_all(tools).map_err(|e| e.to_string())?;
    if !yt_dlp.exists() {
        download(YT_DLP, &yt_dlp)?;
    }
    fetch_zip(tools, "ffmpeg", FFMPEG, &["--strip-components=2", "*/bin/*"])?;
    fetch_zip(tools, "deno", DENO, &[])
}

/// Downloads YouTube's best audio into `job_dir` as FLAC, which adds no second round of lossy compression.
fn fetch_audio(url: &str, tools: &Path, job_dir: &Path, account: Option<&Path>, status: &mut dyn FnMut(String)) -> Result<PathBuf, String> {
    let errors = job_dir.join("errors.txt");
    let log = File::create(&errors).map_err(|e| e.to_string())?;
    let mut cmd = yt_dlp(tools);
    let url = match account {
        Some(dir) => {
            cmd.arg("--cookies-from-browser").arg(cookies_arg(dir));
            account_url(url)
        }
        None => url.to_string(),
    };
    let mut child = cmd
        .args(["--no-playlist", "--windows-filenames", "--color", "never", "--newline", "--progress"])
        .args(["--progress-template", "download:%(progress._percent_str)s"])
        .arg("--ffmpeg-location")
        .arg(tools.join("ffmpeg"))
        .args(["--format", "bestaudio/best", "--extract-audio", "--audio-format", "flac"])
        .arg("--paths")
        .arg(job_dir)
        .args(["--output", "%(title).120s.%(ext)s", "--", &url])
        .stdout(Stdio::piped())
        .stderr(Stdio::from(log))
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let line = line.trim();
            if line.starts_with("100") && line.ends_with('%') {
                status("Saving from YouTube in lossless quality…".into());
            } else if line.ends_with('%') {
                status(format!("Downloading from YouTube: {line}"));
            }
        }
    }
    let ok = child.wait().map_err(|e| e.to_string())?.success();
    let flac = std::fs::read_dir(job_dir).map_err(|e| e.to_string())?.flatten().map(|f| f.path()).find(|p| p.extension().is_some_and(|e| e == "flac"));
    match flac {
        Some(path) if ok => Ok(path),
        _ => {
            let text = std::fs::read_to_string(&errors).unwrap_or_default();
            let reason = text.lines().rev().find_map(|l| l.strip_prefix("ERROR: "));
            Err(explain(reason.unwrap_or("the download didn't finish").to_string(), account.is_some()))
        }
    }
}

fn add(job: &Job, tools: &Path, status: &mut dyn FnMut(String)) -> Result<Source, String> {
    let job_dir = std::env::temp_dir().join("SoundProjector-youtube");
    let _ = std::fs::remove_dir_all(&job_dir);
    std::fs::create_dir_all(&job_dir).map_err(|e| e.to_string())?;
    let sign_in = sign_in_dir();
    let account = connected(&sign_in).then_some(sign_in.as_path());
    let r = fetch_audio(&job.url, tools, &job_dir, account, status).and_then(|file| import(&file, &sounds_dir(), job.source_id));
    let _ = std::fs::remove_dir_all(&job_dir);
    r
}

/// One worker, so several links wait their turn instead of competing.
// ponytail: quitting mid-download leaves yt-dlp to finish on its own into the temp folder, which the next import clears. Add a job object if that ever matters.
pub fn start(ctx: egui::Context) -> (Sender<Work>, Receiver<Event>) {
    let (job_tx, job_rx) = channel::<Work>();
    let (event_tx, event_rx) = channel();
    std::thread::spawn(move || {
        let tools = app_dir().join("tools");
        let sign_in = sign_in_dir();
        clear_unfinished(&sign_in);
        let mut updated = false;
        for work in job_rx {
            let mut status = |s: String| {
                let _ = event_tx.send(Event::Status(s));
                ctx.request_repaint();
            };
            let event = match work {
                Work::Disconnect => Event::Account(
                    false,
                    match disconnect(&sign_in) {
                        Ok(()) => "Disconnected. Your YouTube sign-in was removed from this PC.".into(),
                        Err(_) => "Disconnected. Some sign-in files were in use, so they'll be removed the next time the app starts.".into(),
                    },
                ),
                Work::Connect => match ready_tools(&tools, &mut updated, &mut status).and_then(|()| connect(&sign_in, &tools, &mut status)) {
                    Ok(()) => Event::Account(true, "Connected to your YouTube account.".into()),
                    Err(e) => {
                        clear_unfinished(&sign_in);
                        Event::Account(false, format!("Couldn't connect: {e}"))
                    }
                },
                Work::Download(job) => {
                    status("Getting ready to download from YouTube…".into());
                    match ready_tools(&tools, &mut updated, &mut status).and_then(|()| add(&job, &tools, &mut status)) {
                        Ok(src) => match job.replaces {
                            Some(old) => Event::Replaced(old, src),
                            None => Event::Added(job.pad_id, src),
                        },
                        Err(e) => Event::Failed(e),
                    }
                }
            };
            let _ = event_tx.send(event);
            ctx.request_repaint();
        }
    });
    (job_tx, event_rx)
}

/// YouTube changes often and old yt-dlp versions stop working. A failed update can still try the work.
fn ready_tools(tools: &Path, updated: &mut bool, status: &mut dyn FnMut(String)) -> Result<(), String> {
    ensure_tools(tools, status)?;
    if !std::mem::replace(updated, true) {
        let _ = run(hidden(tools.join("yt-dlp.exe")).arg("--update"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WATCH: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

    #[test]
    fn accepts_single_video_links_and_keeps_only_the_id() {
        for link in [
            WATCH,
            "http://youtube.com/watch?v=dQw4w9WgXcQ",
            "  https://m.youtube.com/watch?list=PL123&v=dQw4w9WgXcQ&t=30s  ",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ&si=abc",
            "https://youtu.be/dQw4w9WgXcQ?si=abc",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "https://www.YouTube.com/watch?v=dQw4w9WgXcQ#t=5",
        ] {
            assert_eq!(video_url(link).as_deref(), Ok(WATCH), "{link}");
        }
    }

    #[test]
    fn rejects_anything_else() {
        for link in [
            "",
            "dQw4w9WgXcQ",
            "--exec=calc.exe",
            "https://www.youtube.com/playlist?list=PL123",
            "https://www.youtube.com/results?search_query=song",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/watch?v=dQw4w9WgXc;",
            "https://example.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com@example.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com.example.com/watch?v=dQw4w9WgXcQ",
            "file:///C:/watch?v=dQw4w9WgXcQ",
        ] {
            assert!(video_url(link).is_err(), "{link}");
        }
    }

    #[test]
    fn only_a_finished_sign_in_counts_as_connected() {
        let dir = crate::testutil::temp_dir("yt-signin").join("signin");
        assert!(!connected(&dir));
        std::fs::create_dir_all(dir.join("Default")).unwrap();
        assert!(!connected(&dir), "a profile alone can be a sign-in that never finished");
        clear_unfinished(&dir);
        assert!(!dir.exists(), "an unfinished sign-in leaves nothing behind");

        std::fs::create_dir_all(dir.join("Default")).unwrap();
        std::fs::write(dir.join(CONNECTED), "").unwrap();
        assert!(connected(&dir));
        clear_unfinished(&dir);
        assert!(connected(&dir), "a finished sign-in survives a restart");
    }

    #[test]
    fn disconnect_removes_the_whole_sign_in() {
        let dir = crate::testutil::temp_dir("yt-signout").join("signin");
        std::fs::create_dir_all(dir.join("Default")).unwrap();
        std::fs::write(dir.join("Default").join("Cookies"), "session").unwrap();
        std::fs::write(dir.join(CONNECTED), "").unwrap();
        disconnect(&dir).unwrap();
        assert!(!dir.exists());
        disconnect(&dir).unwrap(); // already gone is fine
    }

    #[test]
    fn sign_in_check_tells_signed_out_from_signed_in() {
        let signed_out = "ERROR: [youtube:subscriptions] Login details are needed to download this content. Use --cookies-from-browser or --cookies for the authentication.";
        assert!(sign_in_error(Err(signed_out.into())).unwrap_err().contains("didn't finish signing in"));
        assert!(sign_in_error(Err("ERROR: Could not copy Chrome cookie database. See ...".into())).is_err());
        assert!(sign_in_error(Ok("abc".into())).is_ok());
        // It got past the login gate, so an empty or odd feed doesn't mean signed out.
        assert!(sign_in_error(Err("ERROR: [youtube:tab] The channel has no tab".into())).is_ok());
    }

    #[test]
    fn sign_in_window_stays_out_of_the_users_other_accounts() {
        let args = edge_args(Path::new(r"C:\data\youtube-signin"));
        assert!(args.contains(&r"--user-data-dir=C:\data\youtube-signin".to_string()));
        // Otherwise Edge signs this profile in to the Windows work or school account and offers to sync it.
        assert!(args.contains(&"--disable-features=msImplicitSignin".to_string()));
        assert!(args.contains(&"--disable-sync".to_string()));
    }

    #[test]
    fn connected_downloads_go_through_youtube_music() {
        assert_eq!(account_url(WATCH), "https://music.youtube.com/watch?v=dQw4w9WgXcQ");
    }

    #[test]
    fn cookies_come_from_the_apps_own_profile() {
        assert_eq!(cookies_arg(Path::new(r"C:\data\youtube-signin")), r"edge:C:\data\youtube-signin\Default");
    }

    #[test]
    fn errors_point_at_the_account_when_it_can_help() {
        let premium = "This video is only available to Music Premium members".to_string();
        let hint = explain(premium.clone(), false);
        assert!(hint.starts_with(&premium) && hint.contains("connect your YouTube account"), "{hint}");
        assert_eq!(explain(premium.clone(), true), premium, "connecting again won't help");

        let stale = "Sign in to confirm you're not a bot. Use --cookies-from-browser or --cookies".to_string();
        assert!(explain(stale.clone(), true).contains("Disconnect and connect again"));
        assert_eq!(explain(stale.clone(), false), stale);
        assert_eq!(explain("Video unavailable".into(), true), "Video unavailable");
    }

    #[test]
    #[ignore] // downloads the tools and a video: cargo test youtube_end_to_end -- --ignored --nocapture
    fn youtube_end_to_end() {
        let tools = std::env::temp_dir().join("sp-test-yt-tools");
        let job_dir = crate::testutil::temp_dir("yt-job");
        let mut status = |s: String| println!("{s}");
        ensure_tools(&tools, &mut status).unwrap();
        let file = fetch_audio("https://www.youtube.com/watch?v=jNQXAC9IVRw", &tools, &job_dir, None, &mut status).unwrap();
        let lib = crate::testutil::temp_dir("yt-lib");
        let src = import(&file, &lib, 1).unwrap();
        println!("{src:?}\nkept at {}", file.display());
        assert!(src.file.ends_with(".flac") && src.frames > 0 && src.rate >= 44_100);
        let audio = crate::decode::read_preroll(&lib.join(&src.file), 0, src.frames, 48_000, 5000).unwrap();
        assert!(audio.iter().any(|s| s.abs() > 0.01), "the decoded audio is silent");
    }
}
