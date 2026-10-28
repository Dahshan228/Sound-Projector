use std::fs::File;
use std::path::Path;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Timestamp;

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

pub const RING_SECONDS: f32 = 1.0;
const CHUNK: usize = 1024;

/// decode → trim to [start, end) → stereo → resample to `out_rate`.
/// Fixed 1024-frame resampler chunks make the output deterministic, so a preroll and a stream that
/// skips the preroll's length join sample-exactly.
struct Pipeline {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    rate: u32,
    out_rate: u32,
    pos: Option<u64>, // source frame of the next decoded frame; None until anchored after a seek
    start: u64,
    end: u64,
    src_done: bool,
    taken: u64, // source frames passed downstream
    resampler: Option<Async<f32>>,
    pending: Vec<f32>, // stereo frames waiting to fill a resampler chunk
    out_tmp: Vec<f32>,
    drop_out: usize, // resampler delay still to discard
    emitted: u64,
    decoded: Vec<f32>,
}

impl Pipeline {
    fn new(path: &Path, start: u64, end: u64, out_rate: u32) -> Result<Pipeline, String> {
        let mut format = open(path)?;
        let track = format.default_track(TrackType::Audio).ok_or("no audio track")?;
        let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or("no audio track")?.clone();
        let rate = params.sample_rate.ok_or("unknown sample rate")?;
        let track_id = track.id;
        // Timestamps are frames only when the time base is 1/rate; otherwise decode from the top and count.
        let ts_is_frames = track.time_base.is_none_or(|tb| tb.numer.get() == 1 && tb.denom.get() == rate);
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|e| e.to_string())?;
        let mut pos = Some(0);
        if start > 0 && ts_is_frames {
            let to = SeekTo::Timestamp { ts: Timestamp::new(start as i64), track_id };
            if format.seek(SeekMode::Accurate, to).is_ok() {
                pos = None;
            }
        }
        let resampler = if rate == out_rate {
            None
        } else {
            let params = SincInterpolationParameters::new(128, WindowFunction::Blackman2)
                .oversampling_factor(256)
                .interpolation(SincInterpolationType::Quadratic);
            let r = Async::<f32>::new_sinc(out_rate as f64 / rate as f64, 1.0, &params, CHUNK, 2, FixedAsync::Input)
                .map_err(|e| e.to_string())?;
            Some(r)
        };
        let drop_out = resampler.as_ref().map(|r| r.output_delay()).unwrap_or(0);
        let out_max = resampler.as_ref().map(|r| r.output_frames_max()).unwrap_or(0);
        Ok(Pipeline {
            format,
            decoder,
            track_id,
            rate,
            out_rate,
            pos,
            start,
            end,
            src_done: false,
            taken: 0,
            resampler,
            pending: Vec::with_capacity(CHUNK * 4),
            out_tmp: vec![0.0; out_max * 2],
            drop_out,
            emitted: 0,
            decoded: Vec::new(),
        })
    }

    fn total_out(&self) -> u64 {
        crate::library::out_frames(self.taken, self.rate, self.out_rate)
    }

    /// Decode one packet's worth of source frames inside [start, end) into `pending`.
    fn pull_source(&mut self) {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                _ => {
                    self.src_done = true;
                    return;
                }
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let Ok(buf) = self.decoder.decode(&packet) else { continue };
            let frames = buf.frames();
            let ch = buf.spec().channels().count();
            if frames == 0 || ch == 0 {
                continue;
            }
            self.decoded.resize(frames * ch, 0.0);
            buf.copy_to_slice_interleaved(&mut self.decoded[..]);
            let first = *self.pos.get_or_insert_with(|| (packet.pts.get() + packet.trim_start.get() as i64).max(0) as u64);
            self.pos = Some(first + frames as u64);
            for (i, f) in self.decoded.chunks_exact(ch).enumerate() {
                let at = first + i as u64;
                if at < self.start {
                    continue;
                }
                if at >= self.end {
                    self.src_done = true;
                    return;
                }
                let (l, r) = downmix(f);
                self.pending.push(l);
                self.pending.push(r);
                self.taken += 1;
            }
            return;
        }
    }

    /// Append the next run of output frames to `out`. Returns false once everything is out.
    fn next_chunk(&mut self, out: &mut Vec<f32>) -> bool {
        if self.resampler.is_none() {
            if self.pending.is_empty() {
                if self.src_done {
                    return false;
                }
                self.pull_source();
            }
            out.extend_from_slice(&self.pending);
            self.emitted += (self.pending.len() / 2) as u64;
            self.pending.clear();
            return !self.src_done || self.emitted < self.taken;
        }
        while self.pending.len() < CHUNK * 2 && !self.src_done {
            self.pull_source();
        }
        let total = if self.src_done { Some(self.total_out()) } else { None };
        if total.is_some_and(|t| self.emitted >= t) {
            return false;
        }
        // Past the end of the source, flush the resampler with silence.
        let n = self.pending.len().min(CHUNK * 2);
        let mut chunk = [0.0f32; CHUNK * 2];
        chunk[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        let input = InterleavedSlice::new(&chunk[..], 2, CHUNK).unwrap();
        let out_frames = self.out_tmp.len() / 2;
        let mut output = InterleavedSlice::new_mut(&mut self.out_tmp[..], 2, out_frames).unwrap();
        let rs = self.resampler.as_mut().unwrap();
        let Ok((_, produced)) = rs.process_into_buffer(&input, &mut output, None) else { return false };
        let skip = self.drop_out.min(produced);
        self.drop_out -= skip;
        let mut keep = produced - skip;
        if let Some(t) = total {
            keep = keep.min((t - self.emitted) as usize);
        }
        out.extend_from_slice(&self.out_tmp[skip * 2..(skip + keep) * 2]);
        self.emitted += keep as u64;
        true
    }
}

/// Mono → both; 5.1 (FL FR FC LFE BL BR) → ITU-style fold-down; anything else → front L/R.
fn downmix(f: &[f32]) -> (f32, f32) {
    // ponytail: only 5.1 gets a real downmix; other surround layouts take front L/R.
    match f.len() {
        1 => (f[0], f[0]),
        6 => (f[0] + 0.707 * f[2] + 0.707 * f[4], f[1] + 0.707 * f[2] + 0.707 * f[5]),
        _ => (f[0], f[1]),
    }
}

/// The first `frames` output frames of a region, exactly as `spawn_stream` would produce them.
pub fn read_preroll(path: &Path, start: u64, end: u64, out_rate: u32, frames: usize) -> Result<Vec<f32>, String> {
    let mut p = Pipeline::new(path, start, end, out_rate)?;
    let mut v = Vec::with_capacity(frames * 2 + CHUNK * 4);
    while v.len() < frames * 2 && p.next_chunk(&mut v) {}
    v.truncate(frames * 2);
    Ok(v)
}

/// Stream a region on its own thread into a ring of interleaved stereo at `out_rate`.
/// Errors only if the file can't be opened; later errors just end the stream.
pub fn spawn_stream(path: &Path, start: u64, end: u64, out_rate: u32, skip_out_frames: u64) -> Result<rtrb::Consumer<f32>, String> {
    let mut p = Pipeline::new(path, start, end, out_rate)?;
    let (mut tx, rx) = rtrb::RingBuffer::new((out_rate as f32 * RING_SECONDS) as usize * 2);
    std::thread::Builder::new()
        .name("sp-stream".into())
        .spawn(move || {
            let mut skip = (skip_out_frames * 2) as usize;
            let mut buf = Vec::with_capacity(CHUNK * 4);
            loop {
                buf.clear();
                let more = p.next_chunk(&mut buf);
                let s = skip.min(buf.len());
                skip -= s;
                let mut rest = &buf[s..];
                while !rest.is_empty() {
                    if tx.is_abandoned() {
                        return;
                    }
                    let n = tx.slots().min(rest.len());
                    if n == 0 {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                        continue;
                    }
                    if let Ok(chunk) = tx.write_chunk_uninit(n) {
                        chunk.fill_from_iter(rest[..n].iter().copied());
                    }
                    rest = &rest[n..];
                }
                if !more {
                    return;
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(rx)
}

/// Mono (min, max) per `bucket` source frames, for drawing the waveform.
pub fn compute_peaks(path: &Path, bucket: u64) -> Result<Vec<(f32, f32)>, String> {
    let info = probe(path)?;
    let mut p = Pipeline::new(path, 0, u64::MAX, info.rate)?;
    let mut peaks = Vec::new();
    let (mut lo, mut hi, mut n) = (f32::MAX, f32::MIN, 0u64);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let more = p.next_chunk(&mut buf);
        for f in buf.chunks_exact(2) {
            let m = 0.5 * (f[0] + f[1]);
            lo = lo.min(m);
            hi = hi.max(m);
            n += 1;
            if n == bucket {
                peaks.push((lo.max(-1.0), hi.min(1.0)));
                (lo, hi, n) = (f32::MAX, f32::MIN, 0);
            }
        }
        if !more {
            break;
        }
    }
    if n > 0 {
        peaks.push((lo.max(-1.0), hi.min(1.0)));
    }
    Ok(peaks)
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

    fn ramp_wav(name: &str, rate: u32, frames: usize) -> (std::path::PathBuf, Vec<i16>) {
        let s: Vec<i16> = (0..frames).map(|i| ((i % 1000) as i16) * 30).collect();
        let p = temp_dir(name).join("ramp.wav");
        write_wav(&p, rate, 1, &s);
        (p, s)
    }

    fn drain(mut c: rtrb::Consumer<f32>) -> Vec<f32> {
        let t = std::time::Instant::now();
        let mut v = Vec::new();
        loop {
            while let Ok(s) = c.pop() {
                v.push(s);
            }
            if c.is_abandoned() && c.is_empty() {
                return v;
            }
            assert!(t.elapsed().as_secs() < 5, "stream hung");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn preroll_matches_source_region() {
        let (p, s) = ramp_wav("pre", 48_000, 10_000);
        let v = read_preroll(&p, 1000, 3000, 48_000, 500).unwrap();
        assert_eq!(v.len(), 1000);
        for k in 0..500 {
            let e = s[1000 + k] as f32 / 32768.0;
            assert!((v[2 * k] - e).abs() < 1e-4 && (v[2 * k + 1] - e).abs() < 1e-4);
        }
    }

    #[test]
    fn stream_continues_exactly_after_preroll() {
        let (p, s) = ramp_wav("cont", 48_000, 10_000);
        let v = drain(spawn_stream(&p, 1000, 3000, 48_000, 500).unwrap());
        assert_eq!(v.len(), 3000);
        for k in 0..1500 {
            assert!((v[2 * k] - s[1500 + k] as f32 / 32768.0).abs() < 1e-4);
        }
    }

    #[test]
    fn preroll_plus_stream_is_seamless_when_resampling() {
        let (p, _) = ramp_wav("seam", 44_100, 44_100);
        let whole = drain(spawn_stream(&p, 0, 44_100, 48_000, 0).unwrap());
        let mut joined = read_preroll(&p, 0, 44_100, 48_000, 4800).unwrap();
        joined.extend(drain(spawn_stream(&p, 0, 44_100, 48_000, 4800).unwrap()));
        assert_eq!(whole, joined);
    }

    #[test]
    fn resampled_stream_has_right_length_and_pitch() {
        let s: Vec<i16> = (0..44_100)
            .map(|i| (16_000.0 * (std::f32::consts::TAU * 1000.0 * i as f32 / 44_100.0).sin()) as i16)
            .collect();
        let p = temp_dir("pitch").join("sine.wav");
        write_wav(&p, 44_100, 1, &s);
        let v = drain(spawn_stream(&p, 0, 44_100, 48_000, 0).unwrap());
        assert!((v.len() as i64 / 2 - 48_000).abs() <= 2);
        let l: Vec<f32> = v.iter().step_by(2).copied().collect();
        let x = l.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count() as i64;
        assert!((x - 2000).abs() <= 4);
    }

    #[test]
    fn surround_and_mono_become_stereo() {
        let p = temp_dir("51").join("s.wav");
        let frame = [16384i16, -16384, 8192, 0, 8192, 8192]; // FL FR FC LFE BL BR
        write_wav(&p, 48_000, 6, &frame.repeat(100));
        let v = read_preroll(&p, 0, 100, 48_000, 10).unwrap();
        assert!((v[0] - (0.5 + 0.707 * 0.25 + 0.707 * 0.25)).abs() < 1e-3);
        assert!((v[1] - (-0.5 + 0.707 * 0.25 + 0.707 * 0.25)).abs() < 1e-3);
    }

    #[test]
    fn missing_file_errors() {
        assert!(spawn_stream(Path::new("C:/nope/x.wav"), 0, 10, 48_000, 0).is_err());
    }

    #[test]
    fn region_past_end_still_finishes() {
        let (p, _) = ramp_wav("past", 48_000, 10_000);
        let v = drain(spawn_stream(&p, 9000, 15_000, 48_000, 0).unwrap());
        assert!(v.len() <= 2 * 6000);
    }

    #[test]
    fn peaks_cover_whole_file() {
        let (p, _) = ramp_wav("peaks", 48_000, 10_000);
        let pk = compute_peaks(&p, 256).unwrap();
        assert_eq!(pk.len(), 40);
        assert!(pk.iter().all(|&(lo, hi)| lo <= hi && hi <= 1.0 && lo >= -1.0));
    }
}
