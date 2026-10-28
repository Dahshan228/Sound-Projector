use crate::settings::{Pad, PadId, Source};
use std::path::Path;

/// Probe first, then copy into `sounds_dir`. An error never leaves a file behind.
pub fn import(path: &Path, sounds_dir: &Path, id: u32) -> Result<Source, String> {
    let info = crate::decode::probe(path)?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Sound".into());
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let mut file = format!("{stem}{ext}");
    let mut n = 2;
    while sounds_dir.join(&file).exists() {
        file = format!("{stem} ({n}){ext}");
        n += 1;
    }
    std::fs::copy(path, sounds_dir.join(&file)).map_err(|e| e.to_string())?;
    Ok(Source { id, file, name: stem, rate: info.rate, frames: info.frames, channels: info.channels })
}

pub fn whole_pad(src: &Source, id: PadId) -> Pad {
    Pad { id, source: src.id, name: src.name.clone(), start: 0, end: src.frames, boost_db: 0.0, hotkey: None }
}

pub fn secs_to_frames(secs: f64, rate: u32) -> u64 {
    (secs.max(0.0) * rate as f64).round() as u64
}

pub fn frames_to_secs(frames: u64, rate: u32) -> f64 {
    frames as f64 / rate as f64
}

/// Output frames a region of `src_frames` becomes at `out_rate` (rounded up).
pub fn out_frames(src_frames: u64, src_rate: u32, out_rate: u32) -> u64 {
    (src_frames * out_rate as u64).div_ceil(src_rate as u64)
}

/// Order the ends, keep them inside `0..=total`, and make the region at least `min_len` long.
pub fn clamp_region(start: u64, end: u64, total: u64, min_len: u64) -> (u64, u64) {
    let (mut s, mut e) = (start.min(end).min(total), start.max(end).min(total));
    if e - s < min_len {
        e = (s + min_len).min(total);
        s = e.saturating_sub(min_len);
    }
    (s, e)
}

/// A section of a song: source longer than 60 s and the pad isn't (nearly) all of it.
pub fn is_clip(pad: &Pad, src: &Source) -> bool {
    let src_secs = frames_to_secs(src.frames, src.rate);
    let pad_secs = frames_to_secs(pad.end.saturating_sub(pad.start), src.rate);
    src_secs > 60.0 && pad_secs < src_secs - 1.0
}

pub fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    #[test]
    fn region_math() {
        assert_eq!(secs_to_frames(1.5, 44_100), 66_150);
        assert_eq!(secs_to_frames(-1.0, 44_100), 0);
        assert_eq!(frames_to_secs(66_150, 44_100), 1.5);
        assert_eq!(out_frames(44_100, 44_100, 48_000), 48_000);
        assert_eq!(out_frames(1, 44_100, 48_000), 2);
        assert_eq!(clamp_region(100, 50, 1000, 10), (50, 100));
        assert_eq!(clamp_region(0, 5000, 1000, 10), (0, 1000));
        assert_eq!(clamp_region(995, 1000, 1000, 10), (990, 1000));
        assert_eq!(clamp_region(500, 503, 1000, 10), (500, 510));
        assert_eq!(fmt_time(83.4), "1:23");
    }

    #[test]
    fn import_copies_and_dedupes_names() {
        let (src_dir, lib) = (temp_dir("imp-src"), temp_dir("imp-lib"));
        let f = src_dir.join("Bruh.wav");
        write_wav(&f, 48_000, 1, &[0i16; 4800]);
        let a = import(&f, &lib, 1).unwrap();
        let b = import(&f, &lib, 2).unwrap();
        assert_eq!((a.rate, a.frames, a.channels, a.name.as_str()), (48_000, 4800, 1, "Bruh"));
        assert_eq!((a.file.as_str(), b.file.as_str()), ("Bruh.wav", "Bruh (2).wav"));
        assert!(lib.join("Bruh (2).wav").exists() && f.exists());
    }

    #[test]
    fn import_rejects_non_audio() {
        let (src_dir, lib) = (temp_dir("bad-src"), temp_dir("bad-lib"));
        let f = src_dir.join("notes.mp3");
        std::fs::write(&f, "hello").unwrap();
        assert!(import(&f, &lib, 1).is_err());
        assert_eq!(std::fs::read_dir(&lib).unwrap().count(), 0);
    }
}
