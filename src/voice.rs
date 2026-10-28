use crate::dsp::{DriftResampler, LIMIT_THRESHOLD, Limiter};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};

const BLOCK: usize = 2048;

/// One item playing on one output: a cached preroll, then the stream that continues it.
pub struct Voice {
    entry: u64,
    frames_done: u64,
    total: u64,
    preroll: Option<Arc<[f32]>>,
    pre_pos: usize,
    stream: Option<rtrb::Consumer<f32>>,
    gain: f32,
    fade_in: u32, // frames of fade-in still to apply
}

impl Voice {
    pub fn new(
        entry: u64,
        offset_frames: u64,
        total_frames: u64,
        preroll: Option<Arc<[f32]>>,
        stream: Option<rtrb::Consumer<f32>>,
        gain: f32,
        fade_in: bool,
    ) -> Voice {
        Voice {
            entry,
            frames_done: offset_frames,
            total: total_frames,
            preroll,
            pre_pos: 0,
            stream,
            gain,
            fade_in: if fade_in { FADE_FRAMES } else { 0 },
        }
    }

    fn preroll_left(&self) -> usize {
        self.preroll.as_ref().map_or(0, |p| p.len() - self.pre_pos)
    }

    fn finished(&self) -> bool {
        self.frames_done >= self.total
            || (self.preroll_left() == 0 && self.stream.as_ref().is_none_or(|s| s.is_abandoned() && s.is_empty()))
    }

    /// Write up to `dst.len()/2` stereo frames (gain applied), zero the rest. Underruns don't advance.
    fn read(&mut self, dst: &mut [f32]) -> usize {
        let want = (dst.len() / 2).min(self.total.saturating_sub(self.frames_done) as usize);
        let mut got = 0;
        if let Some(p) = &self.preroll {
            let n = want.min((p.len() - self.pre_pos) / 2);
            dst[..n * 2].copy_from_slice(&p[self.pre_pos..self.pre_pos + n * 2]);
            self.pre_pos += n * 2;
            got = n;
        }
        if got < want && self.preroll_left() == 0
            && let Some(s) = &mut self.stream {
                let n = (want - got).min(s.slots() / 2);
                if let Ok(chunk) = s.read_chunk(n * 2) {
                    let (a, b) = chunk.as_slices();
                    dst[got * 2..got * 2 + a.len()].copy_from_slice(a);
                    dst[got * 2 + a.len()..(got + n) * 2].copy_from_slice(b);
                    chunk.commit_all();
                    got += n;
                }
            }
        dst[got * 2..].fill(0.0);
        for (k, f) in dst[..got * 2].as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let mut g = self.gain;
            if self.fade_in > 0 {
                g *= 1.0 - self.fade_in as f32 / FADE_FRAMES as f32;
                self.fade_in -= 1;
            }
            // The last 5 ms fade out, so a trimmed end or a section doesn't click.
            let left = self.total - self.frames_done - k as u64;
            if left < FADE_FRAMES as u64 {
                g *= left as f32 / FADE_FRAMES as f32;
            }
            f[0] *= g;
            f[1] *= g;
        }
        self.frames_done += got as u64;
        got
    }
}

const FADE_FRAMES: u32 = 240; // 5 ms at 48 kHz

pub struct MicIn {
    pub ring: rtrb::Consumer<f32>,
    pub resampler: DriftResampler,
}

pub enum OutCmd {
    SetMain(Option<Box<Voice>>),
    SetPreview(Option<Box<Voice>>),
}

pub struct OutputControl {
    pub cmds: rtrb::Producer<OutCmd>,
    pub trash: rtrb::Consumer<Box<Voice>>,
}

/// What the controller and UI can see of an output, as atomics only.
pub struct OutputShared {
    gain_target: AtomicU32,
    paused: AtomicBool,
    main_entry: AtomicU64,
    main_frames: AtomicU64,
    finished: AtomicU64,
    preview_frames: AtomicU64,
    preview_active: AtomicBool,
    error: AtomicBool,
}

impl OutputShared {
    pub fn set_gain(&self, g: f32) {
        self.gain_target.store(g.to_bits(), Relaxed);
    }
    pub fn set_paused(&self, p: bool) {
        self.paused.store(p, Relaxed);
    }
    pub fn main_entry(&self) -> u64 {
        self.main_entry.load(Relaxed)
    }
    pub fn main_frames(&self) -> u64 {
        self.main_frames.load(Relaxed)
    }
    pub fn take_finished(&self) -> Option<u64> {
        Some(self.finished.swap(0, Relaxed)).filter(|&e| e != 0)
    }
    pub fn preview_frames(&self) -> Option<u64> {
        self.preview_active.load(Relaxed).then(|| self.preview_frames.load(Relaxed))
    }
    pub fn set_error(&self) {
        self.error.store(true, Relaxed);
    }
    pub fn take_error(&self) -> bool {
        self.error.swap(false, Relaxed)
    }
}

/// The real-time mixer for one output device. `render` is the whole cpal callback body:
/// it never locks, allocates or frees.
pub struct OutputState {
    channels: usize,
    main: Option<Box<Voice>>,
    fading: Option<Box<Voice>>,
    fade_left: u32,
    fade_gain: f32, // pause gain when it was replaced, so Stop while paused stays silent
    preview: Option<Box<Voice>>,
    mic: Option<MicIn>,
    limiter: Limiter,
    gain: f32,
    gain_step: f32,
    pause_gain: f32,
    pause_step: f32,
    shared: Arc<OutputShared>,
    cmds: rtrb::Consumer<OutCmd>,
    trash: rtrb::Producer<Box<Voice>>,
    mix: Vec<f32>,
    tmp: Vec<f32>,
    mic_buf: Vec<f32>,
}

impl OutputState {
    pub fn new(channels: usize, rate: u32, mic: Option<MicIn>) -> (OutputState, OutputControl, Arc<OutputShared>) {
        let shared = Arc::new(OutputShared {
            gain_target: AtomicU32::new(1.0f32.to_bits()),
            paused: AtomicBool::new(false),
            main_entry: AtomicU64::new(0),
            main_frames: AtomicU64::new(0),
            finished: AtomicU64::new(0),
            preview_frames: AtomicU64::new(0),
            preview_active: AtomicBool::new(false),
            error: AtomicBool::new(false),
        });
        let (cmd_tx, cmd_rx) = rtrb::RingBuffer::new(16);
        let (trash_tx, trash_rx) = rtrb::RingBuffer::new(64);
        let state = OutputState {
            channels: channels.max(1),
            main: None,
            fading: None,
            fade_left: 0,
            fade_gain: 1.0,
            preview: None,
            mic,
            limiter: Limiter::new(rate, LIMIT_THRESHOLD),
            gain: 1.0,
            gain_step: 1.0 / (rate as f32 * 0.010),
            pause_gain: 1.0,
            pause_step: 1.0 / (rate as f32 * 0.005),
            shared: shared.clone(),
            cmds: cmd_rx,
            trash: trash_tx,
            mix: vec![0.0; BLOCK * 2],
            tmp: vec![0.0; BLOCK * 2],
            mic_buf: vec![0.0; BLOCK],
        };
        (state, OutputControl { cmds: cmd_tx, trash: trash_rx }, shared)
    }

    fn discard(&mut self, v: Box<Voice>) {
        if let Err(rtrb::PushError::Full(v)) = self.trash.push(v) {
            // ponytail: leak beats a free on the audio thread; trash ring is 64 deep
            std::mem::forget(v);
        }
    }

    pub fn render(&mut self, out: &mut [f32]) {
        while let Ok(cmd) = self.cmds.pop() {
            match cmd {
                OutCmd::SetMain(v) => {
                    if let Some(old) = self.main.take() {
                        if let Some(f) = self.fading.replace(old) {
                            self.discard(f);
                        }
                        self.fade_left = FADE_FRAMES;
                        self.fade_gain = self.pause_gain;
                    }
                    self.shared.main_entry.store(v.as_ref().map_or(0, |v| v.entry), Relaxed);
                    self.shared.main_frames.store(v.as_ref().map_or(0, |v| v.frames_done), Relaxed);
                    self.main = v;
                }
                OutCmd::SetPreview(v) => {
                    if let Some(old) = std::mem::replace(&mut self.preview, v) {
                        self.discard(old);
                    }
                    self.shared.preview_frames.store(0, Relaxed);
                    self.shared.preview_active.store(self.preview.is_some(), Relaxed);
                }
            }
        }
        let ch = self.channels;
        for dev in out.chunks_mut(BLOCK * ch) {
            let n = dev.len() / ch;
            self.mix_block(n);
            for (f, [l, r]) in dev.chunks_exact_mut(ch).zip(self.mix.as_chunks::<2>().0) {
                if ch == 1 {
                    f[0] = 0.5 * (l + r);
                } else {
                    f[0] = *l;
                    f[1] = *r;
                    f[2..].fill(0.0);
                }
            }
        }
    }

    fn mix_block(&mut self, n: usize) {
        // Borrow the scratch buffers out of self (pointer swaps, no allocation) so discard() can take &mut self.
        let (mut mix_buf, mut tmp_buf) = (std::mem::take(&mut self.mix), std::mem::take(&mut self.tmp));
        let mix = &mut mix_buf[..n * 2];
        mix.fill(0.0);
        let tmp = &mut tmp_buf[..n * 2];

        // Main voice, with the 5 ms pause ramp. Once fully paused it isn't read, so it holds position.
        let paused = self.shared.paused.load(Relaxed);
        if let Some(v) = self.main.as_mut() {
            let read_frames = if paused { n.min((self.pause_gain / self.pause_step).ceil() as usize) } else { n };
            v.read(&mut tmp[..read_frames * 2]);
            for f in 0..read_frames {
                let target = if paused { 0.0 } else { 1.0 };
                self.pause_gain += (target - self.pause_gain).clamp(-self.pause_step, self.pause_step);
                if (self.pause_gain - target).abs() < 1e-4 {
                    self.pause_gain = target; // float steps never land exactly
                }
                mix[f * 2] += tmp[f * 2] * self.pause_gain;
                mix[f * 2 + 1] += tmp[f * 2 + 1] * self.pause_gain;
            }
            self.shared.main_frames.store(v.frames_done, Relaxed);
            if v.finished() {
                self.shared.finished.store(v.entry, Relaxed);
                self.shared.main_entry.store(0, Relaxed);
                let v = self.main.take().unwrap();
                self.discard(v);
            }
        } else if !paused {
            self.pause_gain = 1.0;
        }

        // The voice that was just replaced or stopped fades out over 5 ms.
        if let Some(v) = self.fading.as_mut() {
            let m = n.min(self.fade_left as usize);
            v.read(&mut tmp[..m * 2]);
            for f in 0..m {
                let g = self.fade_left as f32 / FADE_FRAMES as f32 * self.fade_gain;
                mix[f * 2] += tmp[f * 2] * g;
                mix[f * 2 + 1] += tmp[f * 2 + 1] * g;
                self.fade_left -= 1;
            }
            if self.fade_left == 0 || v.finished() {
                let v = self.fading.take().unwrap();
                self.discard(v);
            }
        }

        if let Some(v) = self.preview.as_mut() {
            v.read(tmp);
            for (m, t) in mix.iter_mut().zip(tmp.iter()) {
                *m += *t;
            }
            self.shared.preview_frames.store(v.frames_done, Relaxed);
            if v.finished() {
                self.shared.preview_active.store(false, Relaxed);
                let v = self.preview.take().unwrap();
                self.discard(v);
            }
        }

        let target = f32::from_bits(self.shared.gain_target.load(Relaxed));
        for f in mix.as_chunks_mut::<2>().0 {
            self.gain += (target - self.gain).clamp(-self.gain_step, self.gain_step);
            f[0] *= self.gain;
            f[1] *= self.gain;
        }

        // The real mic goes in at unity, after the volume slider.
        if let Some(mic) = self.mic.as_mut() {
            let buf = &mut self.mic_buf[..n];
            mic.resampler.process(&mut mic.ring, buf);
            for (f, s) in mix.as_chunks_mut::<2>().0.iter_mut().zip(buf.iter()) {
                f[0] += s;
                f[1] += s;
            }
        }

        self.limiter.process(mix);
        self.mix = mix_buf;
        self.tmp = tmp_buf;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pre(v: Vec<f32>) -> Option<Arc<[f32]>> {
        Some(v.into())
    }
    fn run(o: &mut OutputState, ch: usize, frames: usize) -> Vec<f32> {
        let mut b = vec![9.0; frames * ch];
        o.render(&mut b);
        b
    }
    fn set_main(ctl: &mut OutputControl, v: Voice) {
        assert!(ctl.cmds.push(OutCmd::SetMain(Some(Box::new(v)))).is_ok());
    }

    #[test]
    fn plays_preroll_then_reports_finished() {
        let (mut o, mut ctl, sh) = OutputState::new(2, 48_000, None);
        set_main(&mut ctl, Voice::new(5, 0, 100, pre(vec![0.25; 200]), None, 1.0, false));
        let b = run(&mut o, 2, 1000);
        assert_eq!(b.iter().filter(|s| **s != 0.0).count(), 200);
        assert_eq!(sh.take_finished(), Some(5));
        assert_eq!(sh.take_finished(), None);
        assert!(ctl.trash.pop().is_ok());
    }

    #[test]
    fn stream_picks_up_after_preroll_without_gap() {
        let (mut o, mut ctl, _sh) = OutputState::new(2, 48_000, None);
        let (mut p, c) = rtrb::RingBuffer::new(1000);
        for _ in 0..100 {
            p.push(0.2).unwrap();
        }
        drop(p);
        set_main(&mut ctl, Voice::new(1, 0, 1000, pre(vec![0.1; 100]), Some(c), 1.0, false));
        let b = run(&mut o, 2, 1000);
        let first = b.iter().position(|s| *s != 0.0).unwrap();
        assert!(b[first..first + 100].iter().all(|s| (s - 0.1).abs() < 1e-6));
        assert!(b[first + 100..first + 200].iter().all(|s| (s - 0.2).abs() < 1e-6));
    }

    #[test]
    fn underrun_does_not_skip_content() {
        let (mut o, mut ctl, sh) = OutputState::new(2, 48_000, None);
        let (mut p, c) = rtrb::RingBuffer::new(1000);
        set_main(&mut ctl, Voice::new(1, 0, 1000, None, Some(c), 1.0, false));
        run(&mut o, 2, 100);
        assert_eq!(sh.main_frames(), 0);
        for _ in 0..100 {
            p.push(0.3).unwrap();
        }
        run(&mut o, 2, 100);
        assert_eq!(sh.main_frames(), 50);
    }

    #[test]
    fn pause_holds_position_and_silences() {
        let (mut o, mut ctl, sh) = OutputState::new(2, 48_000, None);
        set_main(&mut ctl, Voice::new(1, 0, 48_000, pre(vec![0.3; 96_000]), None, 1.0, false));
        run(&mut o, 2, 480);
        sh.set_paused(true);
        run(&mut o, 2, 480);
        let at = sh.main_frames();
        let b = run(&mut o, 2, 4800);
        assert_eq!(sh.main_frames(), at);
        assert!(b[2000..].iter().all(|s| s.abs() < 1e-6));
    }

    #[test]
    fn device_channel_mapping() {
        for (ch, check) in [(1usize, 0.25f32), (6, 0.5)] {
            let (mut o, mut ctl, _) = OutputState::new(ch, 48_000, None);
            let v: Vec<f32> = (0..400).map(|i| if i % 2 == 0 { 0.5 } else { 0.0 }).collect(); // L=0.5, R=0
            set_main(&mut ctl, Voice::new(1, 0, 1000, pre(v), None, 1.0, false));
            let b = run(&mut o, ch, 1000);
            let f = b.chunks(ch).find(|f| f[0] != 0.0).unwrap();
            assert!((f[0] - check).abs() < 1e-6);
            if ch == 6 {
                assert!(f[1..].iter().all(|s| *s == 0.0));
            }
        }
    }

    #[test]
    fn boosted_voice_never_exceeds_threshold() {
        let (mut o, mut ctl, _) = OutputState::new(2, 48_000, None);
        let v: Vec<f32> = (0..96_000).map(|i| 0.9 * ((i / 2) as f32 * 0.06).sin()).collect();
        set_main(&mut ctl, Voice::new(1, 0, 48_000, pre(v), None, crate::dsp::db_to_gain(18.0), false));
        let b = run(&mut o, 2, 48_000);
        assert!(b.iter().all(|s| s.abs() <= crate::dsp::LIMIT_THRESHOLD + 1e-6));
    }

    #[test]
    fn mic_is_mixed_in() {
        let (mut p, c) = rtrb::RingBuffer::new(48_000);
        let mic = MicIn { ring: c, resampler: crate::dsp::DriftResampler::new(48_000, 48_000) };
        let (mut o, _ctl, _) = OutputState::new(2, 48_000, Some(mic));
        let mut last = vec![];
        for _ in 0..20 {
            for _ in 0..480 {
                p.push(0.1).unwrap();
            }
            last = run(&mut o, 2, 480);
        }
        assert!(last.iter().all(|s| (s - 0.1).abs() < 1e-3));
    }

    #[test]
    fn voice_fades_out_at_its_end() {
        let (mut o, mut ctl, _) = OutputState::new(2, 48_000, None);
        set_main(&mut ctl, Voice::new(1, 0, 4800, pre(vec![0.5; 9600]), None, 1.0, false));
        let b = run(&mut o, 2, 9600);
        let last = b.iter().rposition(|s| *s != 0.0).unwrap();
        assert!(b[last - 2000..].windows(2).all(|w| (w[1] - w[0]).abs() < 0.01));
    }

    #[test]
    fn stop_while_paused_stays_silent() {
        let (mut o, mut ctl, sh) = OutputState::new(2, 48_000, None);
        set_main(&mut ctl, Voice::new(1, 0, 48_000, pre(vec![0.5; 96_000]), None, 1.0, false));
        run(&mut o, 2, 4800);
        sh.set_paused(true);
        run(&mut o, 2, 4800);
        sh.set_paused(false); // what Cmd::Stop does, along with SetMain(None)
        assert!(ctl.cmds.push(OutCmd::SetMain(None)).is_ok());
        assert!(run(&mut o, 2, 4800).iter().all(|s| s.abs() < 1e-6));
    }

    #[test]
    fn replaced_voice_fades_without_click() {
        let (mut o, mut ctl, _) = OutputState::new(2, 48_000, None);
        set_main(&mut ctl, Voice::new(1, 0, 48_000, pre(vec![0.5; 96_000]), None, 1.0, false));
        let mut b = run(&mut o, 2, 4800);
        assert!(ctl.cmds.push(OutCmd::SetMain(None)).is_ok());
        b.extend(run(&mut o, 2, 4800));
        assert!(b[4000..].windows(2).all(|w| (w[1] - w[0]).abs() < 0.01)); // skip the voice's own (unfaded) start
    }
}
