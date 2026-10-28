use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

pub struct SourceInfo {
    pub rate: u32,
    pub frames: u64,
    pub channels: u16,
}

fn open(path: &Path) -> Result<Box<dyn FormatReader>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|_| "not a supported audio file".to_string())
}

pub fn probe(path: &Path) -> Result<SourceInfo, String> {
    let mut format = open(path)?;
    let track = format.default_track(TrackType::Audio).ok_or("no audio track")?;
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or("no audio track")?.clone();
    let rate = params.sample_rate.ok_or("unknown sample rate")?;
    let channels = params.channels.as_ref().map(|c| c.count()).unwrap_or(0) as u16;
    let track_id = track.id;
    if let (Some(frames), true) = (track.num_frames, channels > 0) {
        return Ok(SourceInfo { rate, frames, channels });
    }
    // Length or layout unknown up front (some MP3s): decode once to count.
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| e.to_string())?;
    let (mut frames, mut ch) = (0u64, channels);
    while let Ok(Some(packet)) = format.next_packet() {
        if packet.track_id != track_id {
            continue;
        }
        if let Ok(buf) = decoder.decode(&packet) {
            frames += buf.frames() as u64;
            ch = buf.spec().channels().count() as u16;
        }
    }
    if frames == 0 || ch == 0 {
        return Err("not a supported audio file".into());
    }
    Ok(SourceInfo { rate, frames, channels: ch })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    #[test]
    fn probe_reads_wav_info() {
        let p = temp_dir("probe").join("a.wav");
        write_wav(&p, 48_000, 1, &[0i16; 10_000]);
        let i = probe(&p).unwrap();
        assert_eq!((i.rate, i.frames, i.channels), (48_000, 10_000, 1));
    }

    #[test]
    fn probe_rejects_non_audio() {
        let p = temp_dir("junk").join("x.mp3");
        std::fs::write(&p, "hello").unwrap();
        assert!(probe(&p).is_err());
    }
}
