/// −0.2 dBFS, applied on both outputs.
pub const LIMIT_THRESHOLD: f32 = 0.977;

pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Perceptual volume curve for the sliders.
pub fn slider_to_gain(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    v * v
}

/// Stereo-linked lookahead peak limiter. Output is delayed by `latency_frames()`.
///
/// The gain is a moving average (length W = L+1) of a release-limited sliding minimum (length W)
/// of each frame's required gain. Every value averaged at a peak's output time has that peak in
/// its window, so the gain there is never above what the peak needs: no overshoot.
pub struct Limiter {
    threshold: f32,
    lookahead: usize,
    release: f32,
    delay: Vec<f32>,      // stereo ring, `lookahead` frames
    delay_pos: usize,
    minq: Vec<(u64, f32)>, // monotonic deque (ring) for the sliding minimum
    minq_head: usize,
    minq_len: usize,
    box_ring: Vec<f32>,   // last W release-limited minimums
    box_pos: usize,
    box_sum: f64,
    held: f32,
    n: u64,
}

impl Limiter {
    pub fn new(rate: u32, threshold: f32) -> Self {
        let lookahead = (rate as usize * 2 / 1000).max(1); // 2 ms
        let w = lookahead + 1;
        Limiter {
            threshold,
            lookahead,
            release: 1.0 - (-1.0 / (0.080 * rate as f32)).exp(), // 80 ms
            delay: vec![0.0; lookahead * 2],
            delay_pos: 0,
            minq: vec![(0, 1.0); w + 1],
            minq_head: 0,
            minq_len: 0,
            box_ring: vec![1.0; w],
            box_pos: 0,
            box_sum: w as f64,
            held: 1.0,
            n: 0,
        }
    }

    pub fn latency_frames(&self) -> usize {
        self.lookahead
    }

    pub fn process(&mut self, stereo: &mut [f32]) {
        let w = self.lookahead + 1;
        let cap = self.minq.len();
        for frame in stereo.chunks_exact_mut(2) {
            let (l, r) = (frame[0], frame[1]);
            let peak = l.abs().max(r.abs());
            let req = if peak > self.threshold { self.threshold / peak } else { 1.0 };

            // Sliding minimum of `req` over the last W frames.
            while self.minq_len > 0 && self.minq[(self.minq_head + self.minq_len - 1) % cap].1 >= req {
                self.minq_len -= 1;
            }
            self.minq[(self.minq_head + self.minq_len) % cap] = (self.n, req);
            self.minq_len += 1;
            while self.minq[self.minq_head].0 + (w as u64) <= self.n {
                self.minq_head = (self.minq_head + 1) % cap;
                self.minq_len -= 1;
            }
            let window_min = self.minq[self.minq_head].1;

            self.held = window_min.min(self.held + (1.0 - self.held) * self.release);

            self.box_sum += self.held as f64 - self.box_ring[self.box_pos] as f64;
            self.box_ring[self.box_pos] = self.held;
            self.box_pos = (self.box_pos + 1) % w;
            let g = (self.box_sum / w as f64) as f32;

            let d = self.delay_pos * 2;
            let (dl, dr) = (self.delay[d], self.delay[d + 1]);
            self.delay[d] = l;
            self.delay[d + 1] = r;
            self.delay_pos = (self.delay_pos + 1) % self.lookahead;

            // Clamp only catches float rounding; the gain already did the work.
            frame[0] = (dl * g).clamp(-self.threshold, self.threshold);
            frame[1] = (dr * g).clamp(-self.threshold, self.threshold);
            self.n += 1;
        }
    }
}

/// Mono mic → output-rate resampler that follows the mic's clock by watching how full its ring is.
pub struct DriftResampler {
    ratio: f64,
    target: usize,
    hist: [f32; 4], // x[-1], x0, x1, x2; output interpolates between x0 and x1
    frac: f64,
    smoothed_fill: f64,
    primed: bool,
}

impl DriftResampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        let target = (in_rate as usize / 50).max(4); // 20 ms
        DriftResampler {
            ratio: in_rate as f64 / out_rate as f64,
            target,
            hist: [0.0; 4],
            frac: 0.0,
            smoothed_fill: target as f64,
            primed: false,
        }
    }

    pub fn process(&mut self, input: &mut rtrb::Consumer<f32>, out: &mut [f32]) {
        let mut fill = input.slots();
        if !self.primed {
            if fill < self.target {
                out.fill(0.0);
                return;
            }
            for h in &mut self.hist {
                *h = input.pop().unwrap_or(0.0);
            }
            fill -= 4;
            self.frac = 0.0;
            self.smoothed_fill = self.target as f64;
            self.primed = true;
        }
        if fill > 4 * self.target {
            for _ in 0..fill - self.target {
                let _ = input.pop();
            }
            fill = self.target;
        }
        self.smoothed_fill += 0.01 * (fill as f64 - self.smoothed_fill);
        let err = (self.smoothed_fill - self.target as f64) / self.target as f64;
        let step = self.ratio * (1.0 + (0.002 * err).clamp(-0.002, 0.002));

        for i in 0..out.len() {
            while self.frac >= 1.0 {
                let Ok(s) = input.pop() else {
                    self.primed = false;
                    out[i..].fill(0.0);
                    return;
                };
                self.hist = [self.hist[1], self.hist[2], self.hist[3], s];
                self.frac -= 1.0;
            }
            let [xm1, x0, x1, x2] = self.hist;
            let t = self.frac as f32;
            // 4-point cubic Hermite (Catmull-Rom).
            let c1 = 0.5 * (x1 - xm1);
            let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
            let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
            out[i] = ((c3 * t + c2) * t + c1) * t + x0;
            self.frac += step;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(f: f32, amp: f32, rate: u32, frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let s = amp * (std::f32::consts::TAU * f * i as f32 / rate as f32).sin();
                [s, s]
            })
            .collect()
    }
    fn crossings(x: &[f32]) -> usize {
        x.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count()
    }

    #[test]
    fn gains() {
        assert!((db_to_gain(0.0) - 1.0).abs() < 1e-6);
        assert!((db_to_gain(6.0) - 1.9953).abs() < 1e-3);
        assert!((db_to_gain(18.0) - 7.943).abs() < 1e-2);
        assert_eq!((slider_to_gain(0.5), slider_to_gain(1.5), slider_to_gain(-1.0)), (0.25, 1.0, 0.0));
    }

    #[test]
    fn limiter_passes_quiet_audio_unchanged() {
        let mut l = Limiter::new(48_000, LIMIT_THRESHOLD);
        let input = sine(1000.0, 0.5, 48_000, 4800);
        let mut buf = input.clone();
        for c in buf.chunks_mut(960) {
            l.process(c);
        }
        let d = l.latency_frames() * 2;
        for i in d..buf.len() {
            assert!((buf[i] - input[i - d]).abs() < 1e-6);
        }
    }

    #[test]
    fn limiter_never_exceeds_threshold_with_18db_boost() {
        let mut l = Limiter::new(48_000, LIMIT_THRESHOLD);
        let mut buf: Vec<f32> = sine(440.0, 1.0, 48_000, 48_000).iter().map(|s| s * db_to_gain(18.0)).collect();
        for c in buf.chunks_mut(960) {
            l.process(c);
        }
        assert!(buf.iter().all(|s| s.abs() <= LIMIT_THRESHOLD + 1e-6));
    }

    #[test]
    fn limiter_catches_single_sample_spike() {
        let mut l = Limiter::new(48_000, LIMIT_THRESHOLD);
        let mut buf = vec![0.0f32; 9600];
        buf[2000] = 10.0;
        buf[2001] = -10.0;
        for c in buf.chunks_mut(960) {
            l.process(c);
        }
        assert!(buf.iter().all(|s| s.abs() <= LIMIT_THRESHOLD + 1e-6));
    }

    #[test]
    fn limiter_recovers_after_burst() {
        let mut l = Limiter::new(48_000, LIMIT_THRESHOLD);
        let mut input = sine(440.0, 4.0, 48_000, 4800);
        input.extend(sine(440.0, 0.5, 48_000, 48_000));
        let mut buf = input.clone();
        for c in buf.chunks_mut(960) {
            l.process(c);
        }
        let d = l.latency_frames() * 2;
        let n = buf.len();
        for i in n - 19_200..n {
            assert!((buf[i] - input[i - d]).abs() < 1e-3);
        }
    }

    #[test]
    fn drift_resampler_keeps_dc_and_pitch_at_equal_rates() {
        let (mut p, mut c) = rtrb::RingBuffer::new(48_000);
        let mut r = DriftResampler::new(48_000, 48_000);
        let src: Vec<f32> = sine(1000.0, 0.5, 48_000, 96_000).iter().step_by(2).copied().collect();
        let mut out = Vec::new();
        let mut blk = vec![0.0; 480];
        for chunk in src.chunks(480) {
            for &s in chunk {
                p.push(s).unwrap();
            }
            r.process(&mut c, &mut blk);
            out.extend_from_slice(&blk);
        }
        let tail = &out[48_000..]; // after priming
        assert!((crossings(tail) as i32 - 2000).abs() <= 4);
    }

    #[test]
    fn drift_resampler_follows_a_faster_mic_without_underrun() {
        let (mut p, mut c) = rtrb::RingBuffer::new(48_000);
        let mut r = DriftResampler::new(48_000, 48_000);
        let mut blk = vec![0.0; 480];
        for call in 0..6000 {
            // 60 s of 10 ms callbacks; mic runs ~0.1% fast
            let n = if call % 2 == 0 { 481 } else { 480 };
            for _ in 0..n {
                let _ = p.push(0.25);
            }
            r.process(&mut c, &mut blk);
            if call > 100 {
                assert!(blk.iter().all(|s| (s - 0.25).abs() < 1e-3));
            }
            assert!(c.slots() < 4 * 960 + 481);
        }
    }

    #[test]
    fn drift_resampler_underrun_is_silence() {
        let (_p, mut c) = rtrb::RingBuffer::<f32>::new(1024);
        let mut r = DriftResampler::new(44_100, 48_000);
        let mut blk = vec![1.0; 480];
        r.process(&mut c, &mut blk);
        assert!(blk.iter().all(|&s| s == 0.0));
    }
}
