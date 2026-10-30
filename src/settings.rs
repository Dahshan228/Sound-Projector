use serde::{Deserialize, Serialize};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

pub type PadId = u32;

/// mods: MOD_ALT=1, MOD_CONTROL=2, MOD_SHIFT=4, MOD_WIN=8; vk: Windows virtual-key code.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Hotkey {
    pub mods: u32,
    pub vk: u32,
}

/// A file in the library folder.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Source {
    pub id: u32,
    pub file: String,
    pub name: String,
    pub rate: u32,
    pub frames: u64,
    pub channels: u16,
}

/// A region of a source; start/end in source frames.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pad {
    pub id: PadId,
    pub source: u32,
    pub name: String,
    pub start: u64,
    pub end: u64,
    pub boost_db: f32,
    pub hotkey: Option<Hotkey>,
    pub group: Option<u32>,
}

/// A set of pads the user named. A pad is in one group at most.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Group {
    pub id: u32,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub sources: Vec<Source>,
    pub pads: Vec<Pad>,
    pub groups: Vec<Group>,
    pub next_id: u32,
    pub friends_volume: f32,
    pub you_volume: f32,
    pub boost_db: f32, // every sound, both outputs, on top of each pad's boost
    pub mic_device: Option<String>,
    pub friends_device: Option<String>,
    pub you_device: Option<String>,
    pub pause_key: Option<Hotkey>,
    pub skip_key: Option<Hotkey>,
    pub stop_key: Option<Hotkey>,
    pub start_with_windows: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sources: Vec::new(),
            pads: Vec::new(),
            groups: Vec::new(),
            next_id: 1,
            friends_volume: 0.8,
            you_volume: 0.5,
            boost_db: 0.0,
            mic_device: None,
            friends_device: None,
            you_device: None,
            pause_key: None,
            skip_key: None,
            stop_key: None,
            start_with_windows: false,
        }
    }
}

impl Settings {
    /// Missing file → defaults. Damaged file → its bytes are kept as `settings.json.bad-<unix>`.
    /// Err when the file is there but can't be read (e.g. locked), so the caller never saves over it.
    pub fn load(path: &Path) -> Result<(Settings, Option<String>), String> {
        let mut read = std::fs::read(path);
        for _ in 0..5 {
            // an antivirus scan can hold the file for a moment
            match &read {
                Err(e) if e.kind() != ErrorKind::NotFound => {}
                _ => break,
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
            read = std::fs::read(path);
        }
        let bytes = match read {
            Ok(b) => b,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok((Settings::default(), None)),
            Err(e) => return Err(e.to_string()),
        };
        match serde_json::from_slice(&bytes) {
            Ok(s) => Ok((s, None)),
            Err(_) => {
                let secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let name = format!("{}.bad-{secs}", path.file_name().unwrap_or_default().to_string_lossy());
                std::fs::write(path.with_file_name(&name), &bytes).map_err(|e| e.to_string())?;
                let notice = format!("Your settings file was damaged, so it was set aside as {name} and a fresh one was started.");
                Ok((Settings::default(), Some(notice)))
            }
        }
    }

    /// Atomic and durable: write and flush `<path>.tmp`, then rename over `path`.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let tmp = path.with_file_name(format!("{}.tmp", path.file_name().unwrap_or_default().to_string_lossy()));
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    }

    pub fn new_id(&mut self) -> u32 {
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        id
    }

    /// Its pads stay in the library, just no longer in a group.
    pub fn delete_group(&mut self, id: u32) {
        self.groups.retain(|g| g.id != id);
        for p in self.pads.iter_mut().filter(|p| p.group == Some(id)) {
            p.group = None;
        }
    }
}

pub fn app_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let d = base.join("SoundProjector");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn sounds_dir() -> PathBuf {
    let d = app_dir().join("sounds");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn settings_path() -> PathBuf {
    app_dir().join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    #[test]
    fn missing_file_gives_defaults() {
        let d = temp_dir("missing");
        let (s, notice) = Settings::load(&d.join("settings.json")).unwrap();
        assert!(s.pads.is_empty() && notice.is_none());
        assert_eq!((s.friends_volume, s.you_volume), (0.8, 0.5));
    }

    #[test]
    fn save_then_load_round_trips() {
        let d = temp_dir("roundtrip");
        let p = d.join("settings.json");
        let mut s = Settings::default();
        s.groups.push(Group { id: 7, name: "Memes".into() });
        s.pads.push(Pad { id: 1, source: 1, name: "Drop".into(), start: 10, end: 20, boost_db: 6.0, hotkey: Some(Hotkey { mods: 2, vk: 0x61 }), group: Some(7) });
        s.save(&p).unwrap();
        assert_eq!(Settings::load(&p).unwrap().0, s);
        assert!(!d.join("settings.json.tmp").exists());
    }

    #[test]
    fn corrupt_file_is_kept_aside() {
        let d = temp_dir("corrupt");
        let p = d.join("settings.json");
        std::fs::write(&p, "{ not json").unwrap();
        let (s, notice) = Settings::load(&p).unwrap();
        assert!(s.pads.is_empty() && notice.is_some());
        let bad: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("settings.json.bad"))
            .collect();
        assert_eq!(bad.len(), 1);
        assert_eq!(std::fs::read_to_string(bad[0].path()).unwrap(), "{ not json");
    }

    #[test]
    fn non_utf8_file_is_kept_aside_too() {
        let d = temp_dir("ansi");
        let p = d.join("settings.json");
        let bytes = b"{\"pads\":[{\"name\":\"Caf\xe9\"}]}";
        std::fs::write(&p, bytes).unwrap();
        let (_, notice) = Settings::load(&p).unwrap();
        assert!(notice.is_some());
        let bad = std::fs::read_dir(&d).unwrap().flatten().find(|e| e.file_name().to_string_lossy().starts_with("settings.json.bad")).unwrap();
        assert_eq!(std::fs::read(bad.path()).unwrap(), bytes);
    }

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let d = temp_dir("fields");
        let p = d.join("settings.json");
        std::fs::write(&p, r#"{"friends_volume":0.3,"future_field":1}"#).unwrap();
        let (s, notice) = Settings::load(&p).unwrap();
        assert_eq!((s.friends_volume, s.you_volume, s.boost_db, notice), (0.3, 0.5, 0.0, None));
    }

    #[test]
    fn a_library_from_before_groups_keeps_its_pads() {
        let d = temp_dir("old-library");
        let p = d.join("settings.json");
        std::fs::write(&p, r#"{"pads":[{"id":1,"source":1,"name":"Drop","start":10,"end":20,"boost_db":6.0,"hotkey":null}]}"#).unwrap();
        let (s, notice) = Settings::load(&p).unwrap();
        assert_eq!((s.pads.len(), s.pads[0].group, s.groups.len(), notice), (1, None, 0, None));
    }

    #[test]
    fn deleting_a_group_keeps_its_pads() {
        let pad = |id, group| Pad { id, source: 1, name: "Drop".into(), start: 0, end: 1, boost_db: 0.0, hotkey: None, group };
        let mut s = Settings { pads: vec![pad(1, Some(7)), pad(2, Some(8)), pad(3, None)], ..Settings::default() };
        s.groups = vec![Group { id: 7, name: "Memes".into() }, Group { id: 8, name: "Songs".into() }];
        s.delete_group(7);
        assert_eq!(s.groups, vec![Group { id: 8, name: "Songs".into() }]);
        assert_eq!(s.pads.iter().map(|p| p.group).collect::<Vec<_>>(), vec![None, Some(8), None]);
    }
}
