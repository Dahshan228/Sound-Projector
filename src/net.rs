use std::ffi::OsStr;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Console tools would otherwise flash a window: the app has no console for them to share.
pub fn hidden(exe: impl AsRef<OsStr>) -> Command {
    let mut c = Command::new(exe);
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

/// A tool built into Windows 10 and later, by full path so nothing beside the app can stand in for it.
pub fn system_tool(exe: &str) -> Command {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| r"C:\Windows".into());
    hidden(root.join("System32").join(exe))
}

/// Runs a tool and returns its output, or its error message.
pub fn run(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(if err.is_empty() { format!("{} failed", cmd.get_program().to_string_lossy()) } else { err })
}

fn curl() -> Command {
    let mut c = system_tool("curl.exe");
    c.args(["-sSfL", "--connect-timeout", "20", "--speed-time", "60", "--speed-limit", "1024"]);
    c
}

pub fn fetch_text(url: &str) -> Result<String, String> {
    run(curl().args(["--max-time", "30", url]))
}

/// A failed download never leaves a file at `to`.
pub fn download(url: &str, to: &Path) -> Result<(), String> {
    let part = to.with_extension("part");
    run(curl().args(["--retry", "2", "-o"]).arg(&part).arg(url))?;
    std::fs::rename(&part, to).map_err(|e| e.to_string())
}

pub fn sha256(path: &Path) -> Result<String, String> {
    let out = run(system_tool("certutil.exe").arg("-hashfile").arg(path).arg("SHA256"))?;
    let hash = out.lines().nth(1).ok_or("no hash in certutil's output")?;
    Ok(hash.replace(' ', "").to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_a_file() {
        let p = crate::testutil::temp_dir("sha").join("abc.txt");
        std::fs::write(&p, "abc").unwrap();
        assert_eq!(sha256(&p).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
