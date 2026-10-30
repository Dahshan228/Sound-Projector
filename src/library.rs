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
    Pad { id, source: src.id, name: src.name.clone(), start: 0, end: src.frames, boost_db: 0.0, hotkey: None, group: None }
}

/// How far apart two copies of a song can be in length and still be the same recording.
const SAME_SONG_SECS: f64 = 2.0;

/// Swaps a song's audio for a better copy. Its pads keep their times, hotkeys and boosts.
/// Returns the old file's name. Refuses a different length, because the clips wouldn't line up.
pub fn replace_source(sources: &mut [Source], pads: &mut [Pad], old_id: u32, new: Source) -> Result<String, String> {
    let old = sources.iter_mut().find(|s| s.id == old_id).ok_or("that song isn't in your library anymore")?;
    let (old_secs, new_secs) = (frames_to_secs(old.frames, old.rate), frames_to_secs(new.frames, new.rate));
    if (old_secs - new_secs).abs() > SAME_SONG_SECS {
        return Err(format!("that video is {} long but this song is {}, so its clips wouldn't line up. Nothing changed.", fmt_time(new_secs), fmt_time(old_secs)));
    }
    for pad in pads.iter_mut().filter(|p| p.source == old_id) {
        let at_end = pad.end >= old.frames;
        let start = secs_to_frames(frames_to_secs(pad.start, old.rate), new.rate);
        let end = if at_end { new.frames } else { secs_to_frames(frames_to_secs(pad.end, old.rate), new.rate) };
        (pad.start, pad.end) = clamp_region(start, end, new.frames, 1);
    }
    let old_file = std::mem::replace(&mut old.file, new.file);
    (old.rate, old.frames, old.channels) = (new.rate, new.frames, new.channels);
    Ok(old_file)
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

    fn song(file: &str, rate: u32, secs: f64) -> Source {
        Source { id: 1, file: file.into(), name: file.into(), rate, frames: secs_to_frames(secs, rate), channels: 2 }
    }

    fn pad(id: PadId, source: u32, start: u64, end: u64) -> Pad {
        Pad { id, source, name: format!("pad {id}"), start, end, boost_db: 3.0, hotkey: None, group: None }
    }

    #[test]
    fn replacing_audio_keeps_clips_at_the_same_times() {
        let mut sources = vec![song("old.mp3", 44_100, 200.0), Source { id: 2, ..song("other.mp3", 44_100, 60.0) }];
        let hotkey = crate::settings::Hotkey { mods: 0, vk: 0x70 };
        let mut pads = vec![pad(1, 1, 0, 8_820_000), pad(2, 1, 1_984_500, 2_866_500), pad(3, 2, 10, 20)];
        pads[1].hotkey = Some(hotkey);
        let old = replace_source(&mut sources, &mut pads, 1, Source { id: 9, name: "New Title".into(), ..song("new.flac", 48_000, 200.5) }).unwrap();
        assert_eq!(old, "old.mp3");
        assert_eq!((sources[0].id, sources[0].name.as_str(), sources[0].file.as_str()), (1, "old.mp3", "new.flac"));
        assert_eq!((sources[0].rate, sources[0].frames), (48_000, 9_624_000));
        assert_eq!((pads[0].start, pads[0].end), (0, 9_624_000)); // a whole-song pad still reaches the end
        assert_eq!((pads[1].start, pads[1].end), (2_160_000, 3_120_000)); // 45 s to 65 s, as before
        assert_eq!((pads[1].hotkey, pads[1].boost_db), (Some(hotkey), 3.0));
        assert_eq!((pads[2].start, pads[2].end), (10, 20)); // another song's pad isn't touched
    }

    #[test]
    fn replacing_audio_keeps_a_clip_inside_a_slightly_shorter_copy() {
        let mut sources = vec![song("old.mp3", 48_000, 100.0)];
        let mut pads = vec![pad(1, 1, 4_776_000, 4_795_200)]; // 99.5 s to 99.9 s
        replace_source(&mut sources, &mut pads, 1, song("new.flac", 48_000, 99.0)).unwrap();
        assert!(pads[0].start < pads[0].end && pads[0].end <= sources[0].frames);
    }

    #[test]
    fn replacing_audio_refuses_a_different_length_and_changes_nothing() {
        let mut sources = vec![song("old.mp3", 44_100, 200.0)];
        let mut pads = vec![pad(1, 1, 44_100, 88_200)];
        let (before_sources, before_pads) = (sources.clone(), pads.clone());
        let err = replace_source(&mut sources, &mut pads, 1, song("new.flac", 48_000, 215.0)).unwrap_err();
        assert!(err.contains("3:35") && err.contains("3:20"), "{err}");
        assert_eq!((sources, pads), (before_sources, before_pads));
        assert!(replace_source(&mut [], &mut [], 1, song("new.flac", 48_000, 200.0)).is_err()); // deleted meanwhile
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
