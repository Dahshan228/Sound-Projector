use crate::library::import;
use crate::net::{download, hidden, run, system_tool};
use crate::settings::{PadId, Source, app_dir, sounds_dir};
use eframe::egui;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::{Receiver, Sender, channel};

const YT_DLP: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
const FFMPEG: &str = "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-win64-gpl-shared.zip";
const DENO: &str = "https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip";

pub struct Job {
    pub url: String,
    pub source_id: u32,
    pub pad_id: PadId,
}

pub enum Event {
    Status(String),
    Added(PadId, Source),
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
fn fetch_audio(url: &str, tools: &Path, job_dir: &Path, status: &mut dyn FnMut(String)) -> Result<PathBuf, String> {
    let errors = job_dir.join("errors.txt");
    let log = File::create(&errors).map_err(|e| e.to_string())?;
    let mut child = hidden(tools.join("yt-dlp.exe"))
        .args(["--ignore-config", "--no-playlist", "--windows-filenames", "--color", "never", "--newline", "--progress"])
        .args(["--progress-template", "download:%(progress._percent_str)s"])
        .arg("--js-runtimes")
        .arg(format!("deno:{}", tools.join("deno").join("deno.exe").display()))
        .arg("--ffmpeg-location")
        .arg(tools.join("ffmpeg"))
        .args(["--format", "bestaudio/best", "--extract-audio", "--audio-format", "flac"])
        .arg("--paths")
        .arg(job_dir)
        .args(["--output", "%(title).120s.%(ext)s", "--", url])
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
            Err(reason.unwrap_or("the download didn't finish").to_string())
        }
    }
}

fn add(job: &Job, tools: &Path, status: &mut dyn FnMut(String)) -> Result<Source, String> {
    let job_dir = std::env::temp_dir().join("SoundProjector-youtube");
    let _ = std::fs::remove_dir_all(&job_dir);
    std::fs::create_dir_all(&job_dir).map_err(|e| e.to_string())?;
    let r = fetch_audio(&job.url, tools, &job_dir, status).and_then(|file| import(&file, &sounds_dir(), job.source_id));
    let _ = std::fs::remove_dir_all(&job_dir);
    r
}

/// One worker, so several links wait their turn instead of competing.
// ponytail: quitting mid-download leaves yt-dlp to finish on its own into the temp folder, which the next import clears. Add a job object if that ever matters.
pub fn start(ctx: egui::Context) -> (Sender<Job>, Receiver<Event>) {
    let (job_tx, job_rx) = channel::<Job>();
    let (event_tx, event_rx) = channel();
    std::thread::spawn(move || {
        let tools = app_dir().join("tools");
        let mut updated = false;
        for job in job_rx {
            let mut status = |s: String| {
                let _ = event_tx.send(Event::Status(s));
                ctx.request_repaint();
            };
            status("Getting ready to download from YouTube…".into());
            let r = ensure_tools(&tools, &mut status).and_then(|()| {
                // YouTube changes often and old yt-dlp versions stop working. A failed update can still try the download.
                if !std::mem::replace(&mut updated, true) {
                    let _ = run(hidden(tools.join("yt-dlp.exe")).arg("--update"));
                }
                add(&job, &tools, &mut status)
            });
            let _ = event_tx.send(match r {
                Ok(src) => Event::Added(job.pad_id, src),
                Err(e) => Event::Failed(e),
            });
            ctx.request_repaint();
        }
    });
    (job_tx, event_rx)
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
    #[ignore] // downloads the tools and a video: cargo test youtube_end_to_end -- --ignored --nocapture
    fn youtube_end_to_end() {
        let tools = std::env::temp_dir().join("sp-test-yt-tools");
        let job_dir = crate::testutil::temp_dir("yt-job");
        let mut status = |s: String| println!("{s}");
        ensure_tools(&tools, &mut status).unwrap();
        let file = fetch_audio("https://www.youtube.com/watch?v=jNQXAC9IVRw", &tools, &job_dir, &mut status).unwrap();
        let lib = crate::testutil::temp_dir("yt-lib");
        let src = import(&file, &lib, 1).unwrap();
        println!("{src:?}\nkept at {}", file.display());
        assert!(src.file.ends_with(".flac") && src.frames > 0 && src.rate >= 44_100);
        let audio = crate::decode::read_preroll(&lib.join(&src.file), 0, src.frames, 48_000, 5000).unwrap();
        assert!(audio.iter().any(|s| s.abs() > 0.01), "the decoded audio is silent");
    }
}
