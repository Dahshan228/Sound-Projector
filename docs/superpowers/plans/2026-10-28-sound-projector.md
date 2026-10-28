# Sound Projector Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a lightweight Windows soundboard. It queues songs and sounds and plays them one at a time into VB-Cable, mixed with the real mic so friends hear them. It also plays them to the user's headphones. Each path has its own volume.

**Architecture:** The app is a single Rust binary with three layers.

- **UI:** an eframe/egui UI on the main thread.
- **Controller:** an engine controller thread owns the queue and the cpal (WASAPI) streams.
- **Audio callbacks:** they only mix pre-built `Voice`s and never block, allocate, or free memory.

Every output device (friends = CABLE Input, you = headphones) plays its own `Voice` of the current item. Each `Voice` has its own decode thread, which uses symphonia to decode and rubato to resample to that device's exact rate. This way there is no drift compensation on the music path. Only the mic → cable path needs drift handling.

Every pad caches a 250 ms "preroll" at each device rate, so playback starts instantly. The rest is streamed from disk. This replaces the spec's "short pads fully in memory" with one code path and less memory; that was Claude's own call in the spec, and the spec is updated to match. Global hotkeys use `RegisterHotKey` on a dedicated message-loop thread, so they work while the window is hidden.

**Tech Stack:**
- Rust (stable, `x86_64-pc-windows-gnu`)
- eframe/egui (glow renderer)
- cpal
- symphonia
- rubato
- rtrb
- serde/serde_json
- tray-icon
- rfd
- windows-sys
- dev: hound

**Spec:** `docs/superpowers/specs/2026-10-28-sound-projector-design.md`

## Global Constraints

**Platform and build:**
- Windows 11 only. The app must work **without admin rights**.
- One `.exe`. GNU toolchain, so no Visual Studio is needed. `#![windows_subsystem = "windows"]` is set in release builds.

**Files and storage:**
- Supported formats: **MP3, WAV, OGG (Vorbis), FLAC**.
- Library folder: `%APPDATA%\SoundProjector\sounds\`. Settings: `%APPDATA%\SoundProjector\settings.json`, one human-readable file.
- **The original file is never modified, and audio is never re-encoded.** Import copies the file. Edits are stored only as settings.

**Audio:**
- Boost is per pad, ranges from **0 to +18 dB**, and defaults to **0 dB**.
- Latency target: under **~30 ms** from a hotkey press to audio at CABLE Input and the headphones, when nothing else is playing.
- Real-time audio callbacks never lock a Mutex, never allocate, and never drop a `Box<Voice>`. Voices to free go to a trash ring that the controller drains.
- The friends output defaults to the output device whose name contains **`CABLE Input`**. If it's not there, the friends output is *off*. It never falls back to the speakers.
- The real mic goes into the friends mix at unity gain. It is **never** sent to the headphones.
- Limiter threshold: `LIMIT_THRESHOLD = 0.977` (−0.2 dBFS) on both outputs (Claude's value).

**Look:**
- Colors:

  | Element | Value |
  |---|---|
  | Background | `#111216` |
  | Top bar and panels | `#18191e` / `#15161b` |
  | Pads | `#1c1d23` |
  | Borders | `#2a2b31` |
  | Accent | `#6450f0` (fills with white text) / `#7c6cff` (borders, slider fill) |
  | "You hear" slider | green `#3ecf8e` |
  | Muted text | `#8e8e99` or brighter |

  All text meets WCAG AA contrast.
- UI copy:
  - Window title: **Sound Projector**
  - Sliders: **Friends hear** and **You hear**
  - Queue panel: **Up next**
  - Now-playing buttons: **Pause/Resume**, **Skip**, **Stop**
  - Section pads show **Song clip**

**Behavior:**
- One item plays at a time from a queue. The same pad can be queued more than once. The queue isn't saved between runs.
- Closing the window sends the app to the tray. The app doesn't start with Windows by default.

## Review Focus

These are conditions the spec implies but no feature test covers. Each one has a test, or a manual check, in the task that owns it.

1. **VB-Cable missing, or a device unplugged mid-call** (for example, the headset). The app keeps running and shows which device is missing. It reopens the device within about 2 s of it returning and resumes the current item where it was. *Owned by Task 7* (the `pick_device` test, plus a manual unplug check).
2. **A second copy launched**, for example by autostart and then by hand. The second copy brings up the first one's window and exits, so there are never two apps fighting over hotkeys and devices. *Owned by Task 11* (manual check).
3. **Bad audio files.** This covers a corrupt or unsupported import, and a library file that was deleted later. Import refuses with a message. Playing a pad whose file is gone shows a notice and moves on to the next item, with no panic. *Owned by Task 2* (the `import_rejects_non_audio` test), *Task 5* (the `missing_file_errors` test), and *Task 7* (the skip-on-error path).
4. **A corrupt `settings.json`**, from a crash mid-write or a bad hand edit. Writes are atomic. A corrupt file is kept as `settings.json.bad-<unix>`, and the app starts empty with a notice. The corrupt file is never overwritten. *Owned by Task 2* (the `corrupt_file_is_kept_aside` test).
5. **Unusual audio.** This covers mono, 5.1, and 96 kHz sources; very short pads (shorter than the preroll); a VBR MP3 whose length estimate is too long; and 1-channel or 6-channel output devices. All of these play correctly and finish, with no hang. *Owned by Task 5* (the `surround_and_mono_become_stereo` and `region_past_end_still_finishes` tests) and *Task 6* (the `device_channel_mapping` and `plays_preroll_then_reports_finished` tests).

---

## File Structure

```
Cargo.toml
README.md                 setup guide + build steps
src/main.rs               entry: single-instance, load settings, start engine/hotkeys, run eframe
src/testutil.rs           #[cfg(test)] temp dirs + WAV writer
src/settings.rs           Settings/Source/Pad/Hotkey structs, paths, atomic load/save
src/library.rs            import (probe+copy), region math, labels
src/queue.rs              Queue (pure logic)
src/dsp.rs                db/slider gain, Limiter, DriftResampler
src/decode.rs             probe, Pipeline (decode→trim→stereo→resample), preroll, stream thread, peaks
src/voice.rs              Voice, OutputState::render (the real-time mixer), OutputShared/OutCmd
src/engine.rs             devices, cpal streams, controller thread, prerolls, Engine API
src/hotkeys.rs            RegisterHotKey thread, key capture, key names
src/tray.rs               tray icon/menu, show/hide via HWND, autostart registry, single-instance find
src/ui/mod.rs             App: theme, top bar, pad grid, queue panel, now-playing, import, notices, hotkey field
src/ui/editor.rs          waveform editor
src/ui/settings_panel.rs  devices, control hotkeys, autostart, setup guide
```

---

### Task 1: Toolchain and empty window

**Files:**
- Create: `Cargo.toml`, `src/main.rs`
- Modify: `.gitignore`

**Interfaces:**
- Produces: a crate named `sound-projector` with binary `sound-projector.exe`, and `mod` declarations in `main.rs` that later tasks add to.

- [ ] **Step 1: Install Rust with the GNU host. No Visual Studio is needed.**

Run in PowerShell:
```powershell
curl.exe -sSfLo "$env:TEMP\rustup-init.exe" https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe
& "$env:TEMP\rustup-init.exe" -y --default-host x86_64-pc-windows-gnu --profile minimal -c clippy
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
cargo --version
```
Expected: `cargo 1.x.y`. If a dependency later fails to link under GNU, install VS Build Tools (`winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"`), then run `rustup default stable-x86_64-pc-windows-msvc`.

- [ ] **Step 2: Create the crate and add dependencies.**

```powershell
cargo init --name sound-projector
cargo add eframe --no-default-features --features glow,default_fonts,persistence,accesskit
cargo add cpal rtrb rubato serde_json rfd tray-icon
cargo add serde --features derive
cargo add symphonia --features mp3
cargo add windows-sys --features Win32_Foundation,Win32_UI_WindowsAndMessaging,Win32_UI_Input_KeyboardAndMouse,Win32_System_Threading,Win32_System_Registry
cargo add --dev hound
```
Append to `Cargo.toml`:
```toml
[profile.dev]
opt-level = 1
[profile.dev.package."*"]
opt-level = 3          # DSP/decoders too slow to play glitch-free unoptimized

[profile.release]
lto = true
codegen-units = 1
strip = true
```
Add `/target` to `.gitignore`.

- [ ] **Step 3: Write `main.rs` to open an empty, dark, titled window.**

- Put `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` at the top.
- Call `eframe::run_native("Sound Projector", NativeOptions { persist_window: true, viewport: ViewportBuilder::default().with_title("Sound Projector").with_inner_size([820.0, 560.0]).with_min_inner_size([560.0, 380.0]).with_drag_and_drop(true), .. })`.
- The window shows a `CentralPanel` filled with `#111216`.

- [ ] **Step 4: Build and run.**

Run: `cargo run`
Expected: a dark window titled "Sound Projector" opens. Task Manager shows about 0% CPU while it sits idle.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs .gitignore
git commit -m "feat: scaffold eframe app"
```

---

### Task 2: Settings and library

**Files:**
- Create: `src/testutil.rs`, `src/settings.rs`, `src/library.rs`, `src/decode.rs` (only `probe` for now; Task 5 fills in the rest)
- Modify: `src/main.rs` (add `mod settings; mod library; mod decode; #[cfg(test)] mod testutil;`)

**Interfaces:**
- Produces:
```rust
// settings.rs
pub type PadId = u32;
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Hotkey { pub mods: u32, pub vk: u32 }          // mods: MOD_ALT=1, MOD_CONTROL=2, MOD_SHIFT=4, MOD_WIN=8
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Source { pub id: u32, pub file: String, pub name: String, pub rate: u32, pub frames: u64, pub channels: u16 }
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pad { pub id: PadId, pub source: u32, pub name: String, pub start: u64, pub end: u64, pub boost_db: f32, pub hotkey: Option<Hotkey> }
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub sources: Vec<Source>, pub pads: Vec<Pad>, pub next_id: u32,
    pub friends_volume: f32, pub you_volume: f32,            // defaults 0.8 / 0.5
    pub mic_device: Option<String>, pub friends_device: Option<String>, pub you_device: Option<String>,
    pub pause_key: Option<Hotkey>, pub skip_key: Option<Hotkey>, pub stop_key: Option<Hotkey>,
    pub start_with_windows: bool,
}
impl Settings {
    pub fn load(path: &Path) -> (Settings, Option<String>);   // notice is Some when a corrupt file was set aside
    pub fn save(&self, path: &Path) -> std::io::Result<()>;    // write path.tmp then rename over path
    pub fn new_id(&mut self) -> u32;                           // returns next_id, then increments
}
pub fn app_dir() -> PathBuf;        // %APPDATA%\SoundProjector (created if missing)
pub fn sounds_dir() -> PathBuf;     // app_dir()\sounds (created if missing)
pub fn settings_path() -> PathBuf;  // app_dir()\settings.json

// library.rs
pub fn import(path: &Path, sounds_dir: &Path, id: u32) -> Result<Source, String>; // probe first, then copy; Err never leaves a file behind
pub fn whole_pad(src: &Source, id: PadId) -> Pad;      // name = src.name, 0..src.frames, boost 0, no hotkey
pub fn secs_to_frames(secs: f64, rate: u32) -> u64;    // rounded, negative → 0
pub fn frames_to_secs(frames: u64, rate: u32) -> f64;
pub fn out_frames(src_frames: u64, src_rate: u32, out_rate: u32) -> u64; // ceil
pub fn clamp_region(start: u64, end: u64, total: u64, min_len: u64) -> (u64, u64);
pub fn is_clip(pad: &Pad, src: &Source) -> bool;       // src > 60 s && pad < src − 1 s
pub fn fmt_time(secs: f64) -> String;                  // "m:ss"
// testutil.rs
pub fn temp_dir(name: &str) -> PathBuf;                // fresh empty dir per call
pub fn write_wav(path: &Path, rate: u32, channels: u16, samples: &[i16]);
// decode.rs
pub struct SourceInfo { pub rate: u32, pub frames: u64, pub channels: u16 }
pub fn probe(path: &Path) -> Result<SourceInfo, String>;  // symphonia; if n_frames is unknown, decode once to count
```

- [ ] **Step 1: Write the failing tests.**

```rust
// settings.rs
#[test] fn missing_file_gives_defaults() {
    let d = temp_dir("missing");
    let (s, notice) = Settings::load(&d.join("settings.json"));
    assert!(s.pads.is_empty() && notice.is_none());
    assert_eq!((s.friends_volume, s.you_volume), (0.8, 0.5));
}
#[test] fn save_then_load_round_trips() {
    let d = temp_dir("roundtrip"); let p = d.join("settings.json");
    let mut s = Settings::default();
    s.pads.push(Pad { id: 1, source: 1, name: "Drop".into(), start: 10, end: 20, boost_db: 6.0, hotkey: Some(Hotkey { mods: 2, vk: 0x61 }) });
    s.save(&p).unwrap();
    assert_eq!(Settings::load(&p).0, s);
    assert!(!d.join("settings.json.tmp").exists());
}
#[test] fn corrupt_file_is_kept_aside() {
    let d = temp_dir("corrupt"); let p = d.join("settings.json");
    std::fs::write(&p, "{ not json").unwrap();
    let (s, notice) = Settings::load(&p);
    assert!(s.pads.is_empty() && notice.is_some());
    let bad: Vec<_> = std::fs::read_dir(&d).unwrap().flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("settings.json.bad")).collect();
    assert_eq!(bad.len(), 1);
    assert_eq!(std::fs::read_to_string(bad[0].path()).unwrap(), "{ not json");
}
#[test] fn unknown_and_missing_fields_are_tolerated() {
    let d = temp_dir("fields"); let p = d.join("settings.json");
    std::fs::write(&p, r#"{"friends_volume":0.3,"future_field":1}"#).unwrap();
    let (s, notice) = Settings::load(&p);
    assert_eq!((s.friends_volume, s.you_volume, notice), (0.3, 0.5, None));
}

// library.rs
#[test] fn region_math() {
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
#[test] fn import_copies_and_dedupes_names() {
    let (src_dir, lib) = (temp_dir("imp-src"), temp_dir("imp-lib"));
    let f = src_dir.join("Bruh.wav"); write_wav(&f, 48_000, 1, &[0i16; 4800]);
    let a = import(&f, &lib, 1).unwrap();
    let b = import(&f, &lib, 2).unwrap();
    assert_eq!((a.rate, a.frames, a.channels, a.name.as_str()), (48_000, 4800, 1, "Bruh"));
    assert_eq!((a.file.as_str(), b.file.as_str()), ("Bruh.wav", "Bruh (2).wav"));
    assert!(lib.join("Bruh (2).wav").exists() && f.exists());
}
#[test] fn import_rejects_non_audio() {
    let (src_dir, lib) = (temp_dir("bad-src"), temp_dir("bad-lib"));
    let f = src_dir.join("notes.mp3"); std::fs::write(&f, "hello").unwrap();
    assert!(import(&f, &lib, 1).is_err());
    assert_eq!(std::fs::read_dir(&lib).unwrap().count(), 0);
}

// decode.rs
#[test] fn probe_reads_wav_info() {
    let p = temp_dir("probe").join("a.wav"); write_wav(&p, 48_000, 1, &[0i16; 10_000]);
    let i = probe(&p).unwrap(); assert_eq!((i.rate, i.frames, i.channels), (48_000, 10_000, 1));
}
#[test] fn probe_rejects_non_audio() {
    let p = temp_dir("junk").join("x.mp3"); std::fs::write(&p, "hello").unwrap(); assert!(probe(&p).is_err());
}
```

- [ ] **Step 2: Run the tests to verify they fail.**

Run: `cargo test settings library decode`
Expected: compile errors (items not defined).

- [ ] **Step 3: Implement `settings.rs`, `library.rs`, `testutil.rs`, and `decode::probe` to the interfaces above.**

- **`Settings::default()`:** implement `Default` by hand so the volumes are 0.8 and 0.5.
- **`Settings::load`:**
  - A missing file returns the defaults with no notice.
  - On a parse error, rename the file to `settings.json.bad-<unix secs>` and return the defaults with the notice `"Your settings file was damaged, so it was set aside as <name> and a fresh one was started."`.
- **`Settings::save`:** write `serde_json::to_string_pretty` to the `.tmp` file, then `std::fs::rename`. On Windows the rename replaces the existing file.
- **`library::import`:**
  - `name` is the file stem.
  - On a filename collision, use `"<stem> (n).<ext>"`, with n counting up from 2.

- [ ] **Step 4: Run the tests to verify they pass.**

Run: `cargo test settings library decode`
Expected: 9 passed.

- [ ] **Step 5: Commit**

```bash
git add src && git commit -m "feat: settings storage and library import"
```

---

### Task 3: Queue

**Files:**
- Create: `src/queue.rs`. Modify: `src/main.rs` (`mod queue;`)

**Interfaces:**
- Produces:
```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QueueEntry { pub id: u64, pub pad: PadId }  // ids start at 1, never reused
#[derive(Default)]
pub struct Queue { /* current: Option<QueueEntry>, up_next: VecDeque<QueueEntry>, next_id: u64 */ }
impl Queue {
    pub fn enqueue(&mut self, pad: PadId) -> u64;           // appends to up_next
    pub fn advance(&mut self) -> Option<QueueEntry>;         // current = pop front (None when empty)
    pub fn stop(&mut self);                                   // clears current and up_next
    pub fn remove(&mut self, entry: u64) -> bool;             // from up_next only
    pub fn move_entry(&mut self, entry: u64, index: usize);   // index clamped; unknown id = no-op
    pub fn remove_pad(&mut self, pad: PadId) -> bool;         // purge from up_next; returns true if current is that pad
    pub fn current(&self) -> Option<QueueEntry>;
    pub fn up_next(&self) -> &VecDeque<QueueEntry>;
    pub fn position_of(&self, pad: PadId) -> Option<usize>;   // first index in up_next
}
```

- [ ] **Step 1: Write the failing tests.**

```rust
fn pads(q: &Queue) -> Vec<PadId> { q.up_next().iter().map(|e| e.pad).collect() }
fn ids(q: &Queue) -> Vec<u64> { q.up_next().iter().map(|e| e.id).collect() }

#[test] fn order_holds_across_enqueue_skip_remove_reorder_stop() {
    let mut q = Queue::default();
    let (a, b, c, d) = (q.enqueue(1), q.enqueue(2), q.enqueue(3), q.enqueue(2));
    assert_eq!(q.advance().map(|e| e.id), Some(a));
    assert_eq!(pads(&q), vec![2, 3, 2]);
    assert!(q.remove(c)); assert_eq!(pads(&q), vec![2, 2]);
    q.move_entry(d, 0); assert_eq!(ids(&q), vec![d, b]);
    assert_eq!(q.position_of(2), Some(0));
    assert_eq!(q.advance().map(|e| e.id), Some(d));
    assert_eq!(q.current().map(|e| e.id), Some(d));
    q.stop(); assert!(q.current().is_none() && q.up_next().is_empty());
    assert!(q.advance().is_none());
}
#[test] fn same_pad_twice_gets_distinct_ids() {
    let mut q = Queue::default(); assert_ne!(q.enqueue(7), q.enqueue(7));
}
#[test] fn move_entry_clamps_and_ignores_unknown() {
    let mut q = Queue::default(); let (a, b) = (q.enqueue(1), q.enqueue(2));
    q.move_entry(a, 99); assert_eq!(ids(&q), vec![b, a]);
    q.move_entry(12345, 0); assert_eq!(ids(&q), vec![b, a]);
    assert!(!q.remove(12345));
}
#[test] fn deleting_a_pad_purges_it() {
    let mut q = Queue::default(); q.enqueue(1); q.enqueue(2); q.enqueue(1); q.advance();
    assert!(q.remove_pad(1));            // current was pad 1
    assert_eq!(pads(&q), vec![2]);
}
```

- [ ] **Step 2: Run the tests to verify they fail.** Run: `cargo test queue`. Expected: compile errors.
- [ ] **Step 3: Implement `Queue`** with a `VecDeque`.
- [ ] **Step 4: Run the tests to verify they pass.** Run: `cargo test queue`. Expected: 4 passed.
- [ ] **Step 5: Commit:** `git add src && git commit -m "feat: playback queue"`

---

### Task 4: DSP (gain, limiter, mic drift resampler)

**Files:**
- Create: `src/dsp.rs`. Modify: `src/main.rs` (`mod dsp;`)

**Interfaces:**
- Produces:
```rust
pub const LIMIT_THRESHOLD: f32 = 0.977;
pub fn db_to_gain(db: f32) -> f32;
pub fn slider_to_gain(v: f32) -> f32;                       // clamp(v,0,1)^2
pub struct Limiter;  // stereo-linked lookahead peak limiter
impl Limiter {
    pub fn new(rate: u32, threshold: f32) -> Self;          // lookahead 2 ms, release 80 ms; only place it allocates
    pub fn process(&mut self, stereo: &mut [f32]);          // in place, interleaved L/R, delays by latency_frames()
    pub fn latency_frames(&self) -> usize;
}
pub struct DriftResampler;  // mono, mic rate → output rate, follows the mic's clock via ring fill level
impl DriftResampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self;        // target fill = 20 ms of in_rate
    pub fn process(&mut self, input: &mut rtrb::Consumer<f32>, out: &mut [f32]); // never allocates
}
```

The limiter algorithm isn't determined by its tests, so here it is in pseudocode. `L` is the lookahead in frames.
```
per frame x=(l,r):
  req   = min(1, threshold / max(|l|,|r|))           // gain this frame needs
  m     = min(sliding_min(req over last L frames),   // O(1) amortised: monotonic deque in a fixed array
              m_prev + (1 - m_prev) * release_coef)   // release toward 1
  g     = moving_average(m over last L frames)        // box filter → smooth attack that lands in time
  out   = delay_line(x, L) * g                        // signal delayed by L, so g reaches the peak's need before the peak
```
Every `m` in the box window ending at a peak's output time has that peak in its lookahead window, so `g ≤ req(peak)`. That makes overshoot impossible.

`DriftResampler`:
- It uses a 4-point cubic Hermite interpolator. The step is `in_rate/out_rate × (1 + c)`. `c` is `clamp(0.002 × (smoothed_fill − target)/target, ±0.002)`, and `smoothed_fill` is a one-pole average with α = 0.01 per call.
- **Underrun:** if the ring has no samples, output zeros and wait until `fill ≥ target` before resuming.
- **Overflow:** if `fill > 4 × target`, discard input down to the target.

- [ ] **Step 1: Write the failing tests.**

```rust
fn sine(f: f32, amp: f32, rate: u32, frames: usize) -> Vec<f32> {
    (0..frames).flat_map(|i| { let s = amp * (std::f32::consts::TAU * f * i as f32 / rate as f32).sin(); [s, s] }).collect()
}
fn crossings(x: &[f32]) -> usize { x.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count() }

#[test] fn gains() {
    assert!((db_to_gain(0.0) - 1.0).abs() < 1e-6);
    assert!((db_to_gain(6.0) - 1.9953).abs() < 1e-3);
    assert!((db_to_gain(18.0) - 7.943).abs() < 1e-2);
    assert_eq!((slider_to_gain(0.5), slider_to_gain(1.5), slider_to_gain(-1.0)), (0.25, 1.0, 0.0));
}
#[test] fn limiter_passes_quiet_audio_unchanged() {
    let mut l = Limiter::new(48_000, LIMIT_THRESHOLD);
    let input = sine(1000.0, 0.5, 48_000, 4800); let mut buf = input.clone();
    for c in buf.chunks_mut(960) { l.process(c); }
    let d = l.latency_frames() * 2;
    for i in d..buf.len() { assert!((buf[i] - input[i - d]).abs() < 1e-6); }
}
#[test] fn limiter_never_exceeds_threshold_with_18db_boost() {
    let mut l = Limiter::new(48_000, LIMIT_THRESHOLD);
    let mut buf: Vec<f32> = sine(440.0, 1.0, 48_000, 48_000).iter().map(|s| s * db_to_gain(18.0)).collect();
    for c in buf.chunks_mut(960) { l.process(c); }
    assert!(buf.iter().all(|s| s.abs() <= LIMIT_THRESHOLD + 1e-6));
}
#[test] fn limiter_catches_single_sample_spike() {
    let mut l = Limiter::new(48_000, LIMIT_THRESHOLD);
    let mut buf = vec![0.0f32; 9600]; buf[2000] = 10.0; buf[2001] = -10.0;
    for c in buf.chunks_mut(960) { l.process(c); }
    assert!(buf.iter().all(|s| s.abs() <= LIMIT_THRESHOLD + 1e-6));
}
#[test] fn limiter_recovers_after_burst() {
    let mut l = Limiter::new(48_000, LIMIT_THRESHOLD);
    let mut input = sine(440.0, 4.0, 48_000, 4800); input.extend(sine(440.0, 0.5, 48_000, 48_000));
    let mut buf = input.clone(); for c in buf.chunks_mut(960) { l.process(c); }
    let d = l.latency_frames() * 2; let n = buf.len();
    for i in n - 19_200..n { assert!((buf[i] - input[i - d]).abs() < 1e-3); }
}
#[test] fn drift_resampler_keeps_dc_and_pitch_at_equal_rates() {
    let (mut p, mut c) = rtrb::RingBuffer::new(48_000);
    let mut r = DriftResampler::new(48_000, 48_000);
    let src: Vec<f32> = sine(1000.0, 0.5, 48_000, 96_000).iter().step_by(2).copied().collect();
    let mut out = Vec::new(); let mut blk = vec![0.0; 480];
    for chunk in src.chunks(480) { for &s in chunk { p.push(s).unwrap(); } r.process(&mut c, &mut blk); out.extend_from_slice(&blk); }
    let tail = &out[48_000..]; // after priming
    assert!((crossings(tail) as i32 - 2000).abs() <= 4);
}
#[test] fn drift_resampler_follows_a_faster_mic_without_underrun() {
    let (mut p, mut c) = rtrb::RingBuffer::new(48_000);
    let mut r = DriftResampler::new(48_000, 48_000);
    let mut blk = vec![0.0; 480];
    for call in 0..6000 {                       // 60 s of 10 ms callbacks; mic runs ~0.1% fast
        let n = if call % 2 == 0 { 481 } else { 480 };
        for _ in 0..n { let _ = p.push(0.25); }
        r.process(&mut c, &mut blk);
        if call > 100 { assert!(blk.iter().all(|s| (s - 0.25).abs() < 1e-3)); }
        assert!(c.slots() < 4 * 960 + 481);
    }
}
#[test] fn drift_resampler_underrun_is_silence() {
    let (_p, mut c) = rtrb::RingBuffer::<f32>::new(1024);
    let mut r = DriftResampler::new(44_100, 48_000); let mut blk = vec![1.0; 480];
    r.process(&mut c, &mut blk); assert!(blk.iter().all(|&s| s == 0.0));
}
```

- [ ] **Step 2: Run the tests to verify they fail.** Run: `cargo test dsp`. Expected: compile errors.
- [ ] **Step 3: Implement `dsp.rs`** to the interfaces and algorithm above.
- [ ] **Step 4: Run the tests to verify they pass.** Run: `cargo test dsp`. Expected: 7 passed.
- [ ] **Step 5: Commit:** `git add src && git commit -m "feat: limiter, gain curves, mic drift resampler"`

---

### Task 5: Decoding and streaming

**Files:**
- Modify: `src/decode.rs` (Task 2 created it with `probe`)

**Interfaces:**
- Consumes: `decode::probe`, `testutil::*` (Task 2).
- Produces:
```rust
pub fn read_preroll(path: &Path, start: u64, end: u64, out_rate: u32, frames: usize) -> Result<Vec<f32>, String>;
pub fn spawn_stream(path: &Path, start: u64, end: u64, out_rate: u32, skip_out_frames: u64)
    -> Result<rtrb::Consumer<f32>, String>;   // Err only if the file can't be opened; later errors just end the stream
pub fn compute_peaks(path: &Path, bucket: u64) -> Result<Vec<(f32, f32)>, String>; // mono (min,max) per `bucket` source frames
pub const RING_SECONDS: f32 = 1.0;
```
`start` and `end` are in source frames. All output is interleaved stereo f32 at `out_rate`. The stream thread exits when it reaches the end, hits a decode error, or finds the consumer abandoned. It drops its producer so the consumer sees `is_abandoned()`.

**Approach:** a private `Pipeline` used by all four functions:

1. Open with symphonia using `enable_gapless: true` and `SeekMode::Accurate`. Discard frames up to the exact `start`.
2. Downmix to stereo:
   - mono: `L = R = c0`
   - 6 channels: `L = c0 + 0.707·c2 + 0.707·c4`, `R = c1 + 0.707·c2 + 0.707·c5`
   - any other count above 2: `L = c0`, `R = c1`
3. Trim at `end`.
4. Feed fixed 1024-frame chunks to a rubato sinc resampler. Bypass it entirely when `rate == out_rate`. Drop its `output_delay()` frames at the start and flush with zeros at the end, so the output length is `out_frames(end − start, rate, out_rate)`.

Fixed chunking makes the output deterministic. That's what lets a preroll plus a stream with `skip_out_frames = preroll_len` join sample-exactly. The stream thread sleeps 2 ms whenever the ring is full.

Add this marker: `// ponytail: only 5.1 gets a real downmix; other surround layouts take front L/R.`

- [ ] **Step 1: Write the failing tests.**

```rust
fn ramp_wav(name: &str, rate: u32, frames: usize) -> (PathBuf, Vec<i16>) {
    let s: Vec<i16> = (0..frames).map(|i| ((i % 1000) as i16) * 30).collect();
    let p = temp_dir(name).join("ramp.wav"); write_wav(&p, rate, 1, &s); (p, s)
}
fn drain(mut c: rtrb::Consumer<f32>) -> Vec<f32> {
    let t = std::time::Instant::now(); let mut v = Vec::new();
    loop { while let Ok(s) = c.pop() { v.push(s); }
           if c.is_abandoned() && c.is_empty() { return v; }
           assert!(t.elapsed().as_secs() < 5, "stream hung"); std::thread::sleep(std::time::Duration::from_millis(1)); }
}
#[test] fn preroll_matches_source_region() {
    let (p, s) = ramp_wav("pre", 48_000, 10_000);
    let v = read_preroll(&p, 1000, 3000, 48_000, 500).unwrap();
    assert_eq!(v.len(), 1000);
    for k in 0..500 { let e = s[1000 + k] as f32 / 32768.0; assert!((v[2*k] - e).abs() < 1e-4 && (v[2*k+1] - e).abs() < 1e-4); }
}
#[test] fn stream_continues_exactly_after_preroll() {
    let (p, s) = ramp_wav("cont", 48_000, 10_000);
    let v = drain(spawn_stream(&p, 1000, 3000, 48_000, 500).unwrap());
    assert_eq!(v.len(), 3000);
    for k in 0..1500 { assert!((v[2*k] - s[1500 + k] as f32 / 32768.0).abs() < 1e-4); }
}
#[test] fn preroll_plus_stream_is_seamless_when_resampling() {
    let (p, _) = ramp_wav("seam", 44_100, 44_100);
    let whole = drain(spawn_stream(&p, 0, 44_100, 48_000, 0).unwrap());
    let mut joined = read_preroll(&p, 0, 44_100, 48_000, 4800).unwrap();
    joined.extend(drain(spawn_stream(&p, 0, 44_100, 48_000, 4800).unwrap()));
    assert_eq!(whole, joined);
}
#[test] fn resampled_stream_has_right_length_and_pitch() {
    let s: Vec<i16> = (0..44_100).map(|i| (16_000.0 * (std::f32::consts::TAU * 1000.0 * i as f32 / 44_100.0).sin()) as i16).collect();
    let p = temp_dir("pitch").join("sine.wav"); write_wav(&p, 44_100, 1, &s);
    let v = drain(spawn_stream(&p, 0, 44_100, 48_000, 0).unwrap());
    assert!((v.len() as i64 / 2 - 48_000).abs() <= 2);
    let l: Vec<f32> = v.iter().step_by(2).copied().collect();
    let x = l.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count() as i64;
    assert!((x - 2000).abs() <= 4);
}
#[test] fn surround_and_mono_become_stereo() {
    let p = temp_dir("51").join("s.wav");
    let frame = [16384i16, -16384, 8192, 0, 8192, 8192]; // FL FR FC LFE BL BR
    write_wav(&p, 48_000, 6, &frame.repeat(100));
    let v = read_preroll(&p, 0, 100, 48_000, 10).unwrap();
    assert!((v[0] - (0.5 + 0.707 * 0.25 + 0.707 * 0.25)).abs() < 1e-3);
    assert!((v[1] - (-0.5 + 0.707 * 0.25 + 0.707 * 0.25)).abs() < 1e-3);
}
#[test] fn missing_file_errors() { assert!(spawn_stream(Path::new("C:/nope/x.wav"), 0, 10, 48_000, 0).is_err()); }
#[test] fn region_past_end_still_finishes() {
    let (p, _) = ramp_wav("past", 48_000, 10_000);
    let v = drain(spawn_stream(&p, 9000, 15_000, 48_000, 0).unwrap());
    assert!(v.len() <= 2 * 6000);
}
#[test] fn peaks_cover_whole_file() {
    let (p, _) = ramp_wav("peaks", 48_000, 10_000);
    let pk = compute_peaks(&p, 256).unwrap();
    assert_eq!(pk.len(), 40);
    assert!(pk.iter().all(|&(lo, hi)| lo <= hi && hi <= 1.0 && lo >= -1.0));
}
```

- [ ] **Step 2: Run the tests to verify they fail.** Run: `cargo test decode`. Expected: compile errors.
- [ ] **Step 3: Implement `decode.rs`** as described above.
- [ ] **Step 4: Run the tests to verify they pass.** Run: `cargo test decode`. Expected: 10 passed.
- [ ] **Step 5: Commit:** `git add src && git commit -m "feat: decode pipeline, preroll and streaming"`

---

### Task 6: Voice and the real-time mixer

**Files:**
- Create: `src/voice.rs`. Modify: `src/main.rs` (`mod voice;`)

**Interfaces:**
- Consumes: `dsp::{Limiter, DriftResampler, LIMIT_THRESHOLD}`.
- Produces:
```rust
pub struct Voice;
impl Voice {
    pub fn new(entry: u64, offset_frames: u64, total_frames: u64, preroll: Option<Arc<[f32]>>,
               stream: Option<rtrb::Consumer<f32>>, gain: f32, fade_in: bool) -> Voice;
}
pub struct MicIn { pub ring: rtrb::Consumer<f32>, pub resampler: DriftResampler }
pub enum OutCmd { SetMain(Option<Box<Voice>>), SetPreview(Option<Box<Voice>>) }
pub struct OutputControl { pub cmds: rtrb::Producer<OutCmd>, pub trash: rtrb::Consumer<Box<Voice>> }
pub struct OutputShared; // atomics only
impl OutputShared {
    pub fn set_gain(&self, g: f32); pub fn set_paused(&self, p: bool);
    pub fn main_entry(&self) -> u64;          // 0 = nothing playing
    pub fn main_frames(&self) -> u64;         // offset + frames played of the main voice
    pub fn take_finished(&self) -> Option<u64>; // entry id whose voice ended, consumed once
    pub fn preview_frames(&self) -> Option<u64>;
    pub fn set_error(&self); pub fn take_error(&self) -> bool;
}
pub struct OutputState;
impl OutputState {
    pub fn new(channels: usize, rate: u32, mic: Option<MicIn>) -> (OutputState, OutputControl, Arc<OutputShared>);
    pub fn render(&mut self, out: &mut [f32]);   // interleaved device channels; the cpal callback body
}
```

**How `render` works:**
1. Drain `cmds`.
   - `SetMain` moves the old main voice into a `fading` slot, which fades to zero over 5 ms. Anything already in `fading` goes straight to trash.
   - `SetPreview` sends the old preview voice straight to trash.
2. In fixed-size internal stereo blocks (scratch is preallocated, 2048 frames):
   - Read `main`: preroll first, then the stream. On underrun, write zeros and **don't** advance. Advance the "pause" ramp (5 ms) toward 0 or 1. While paused, `main` is not read.
   - Multiply by the voice gain. Then multiply by the output gain, ramped toward `gain_target` over 10 ms. Add `fading` and `preview`.
   - Add the mic (mono to both channels, at unity gain).
   - Run `Limiter`, then hard-clamp to ±1.
3. Map to the device's channels:
   - 1 channel: `(L+R)/2`
   - 2 or more: L and R go to channels 0 and 1; the rest get 0
4. When a voice ends (`frames ≥ total`, or the stream is abandoned and empty with the preroll used up), store its entry in `finished` and push the voice to trash. If trash is full, `mem::forget` it, with the comment `// ponytail: leak beats a free on the audio thread; trash ring is 64 deep`.

- [ ] **Step 1: Write the failing tests.**

```rust
fn pre(v: Vec<f32>) -> Option<Arc<[f32]>> { Some(v.into()) }
fn run(o: &mut OutputState, ch: usize, frames: usize) -> Vec<f32> { let mut b = vec![9.0; frames * ch]; o.render(&mut b); b }

#[test] fn plays_preroll_then_reports_finished() {
    let (mut o, mut ctl, sh) = OutputState::new(2, 48_000, None);
    ctl.cmds.push(OutCmd::SetMain(Some(Box::new(Voice::new(5, 0, 100, pre(vec![0.25; 200]), None, 1.0, false))))).ok();
    let b = run(&mut o, 2, 1000);
    assert_eq!(b.iter().filter(|s| (**s - 0.25).abs() < 1e-6).count(), 200);
    assert_eq!(sh.take_finished(), Some(5)); assert_eq!(sh.take_finished(), None);
    assert!(ctl.trash.pop().is_ok());
}
#[test] fn stream_picks_up_after_preroll_without_gap() {
    let (mut o, mut ctl, _sh) = OutputState::new(2, 48_000, None);
    let (mut p, c) = rtrb::RingBuffer::new(1000); for _ in 0..100 { p.push(0.2).unwrap(); } drop(p);
    ctl.cmds.push(OutCmd::SetMain(Some(Box::new(Voice::new(1, 0, 100, pre(vec![0.1; 100]), Some(c), 1.0, false))))).ok();
    let b = run(&mut o, 2, 1000);
    let first = b.iter().position(|s| *s != 0.0).unwrap();
    assert!(b[first..first + 100].iter().all(|s| (s - 0.1).abs() < 1e-6));
    assert!(b[first + 100..first + 200].iter().all(|s| (s - 0.2).abs() < 1e-6));
}
#[test] fn underrun_does_not_skip_content() {
    let (mut o, mut ctl, sh) = OutputState::new(2, 48_000, None);
    let (mut p, c) = rtrb::RingBuffer::new(1000);
    ctl.cmds.push(OutCmd::SetMain(Some(Box::new(Voice::new(1, 0, 1000, None, Some(c), 1.0, false))))).ok();
    run(&mut o, 2, 100); assert_eq!(sh.main_frames(), 0);
    for _ in 0..100 { p.push(0.3).unwrap(); }
    run(&mut o, 2, 100); assert_eq!(sh.main_frames(), 50);
}
#[test] fn pause_holds_position_and_silences() {
    let (mut o, mut ctl, sh) = OutputState::new(2, 48_000, None);
    ctl.cmds.push(OutCmd::SetMain(Some(Box::new(Voice::new(1, 0, 48_000, pre(vec![0.3; 96_000]), None, 1.0, false))))).ok();
    run(&mut o, 2, 480); sh.set_paused(true); run(&mut o, 2, 480);
    let at = sh.main_frames(); let b = run(&mut o, 2, 4800);
    assert_eq!(sh.main_frames(), at); assert!(b[2000..].iter().all(|s| s.abs() < 1e-6));
}
#[test] fn device_channel_mapping() {
    for (ch, check) in [(1usize, 0.25f32), (6, 0.5)] {
        let (mut o, mut ctl, _) = OutputState::new(ch, 48_000, None);
        let v: Vec<f32> = (0..400).map(|i| if i % 2 == 0 { 0.5 } else { 0.0 }).collect(); // L=0.5, R=0
        ctl.cmds.push(OutCmd::SetMain(Some(Box::new(Voice::new(1, 0, 200, pre(v), None, 1.0, false))))).ok();
        let b = run(&mut o, ch, 1000);
        let f = b.chunks(ch).find(|f| f[0] != 0.0).unwrap();
        assert!((f[0] - check).abs() < 1e-6);
        if ch == 6 { assert!(f[1..].iter().all(|s| *s == 0.0)); }
    }
}
#[test] fn boosted_voice_never_exceeds_threshold() {
    let (mut o, mut ctl, _) = OutputState::new(2, 48_000, None);
    let v: Vec<f32> = (0..96_000).map(|i| 0.9 * ((i / 2) as f32 * 0.06).sin()).collect();
    ctl.cmds.push(OutCmd::SetMain(Some(Box::new(Voice::new(1, 0, 48_000, pre(v), None, crate::dsp::db_to_gain(18.0), false))))).ok();
    let b = run(&mut o, 2, 48_000); assert!(b.iter().all(|s| s.abs() <= crate::dsp::LIMIT_THRESHOLD + 1e-6));
}
#[test] fn mic_is_mixed_in() {
    let (mut p, c) = rtrb::RingBuffer::new(48_000);
    let (mut o, _ctl, _) = OutputState::new(2, 48_000, Some(MicIn { ring: c, resampler: crate::dsp::DriftResampler::new(48_000, 48_000) }));
    let mut last = vec![];
    for _ in 0..20 { for _ in 0..480 { p.push(0.1).unwrap(); } last = run(&mut o, 2, 480); }
    assert!(last.iter().all(|s| (s - 0.1).abs() < 1e-3));
}
#[test] fn replaced_voice_fades_without_click() {
    let (mut o, mut ctl, _) = OutputState::new(2, 48_000, None);
    ctl.cmds.push(OutCmd::SetMain(Some(Box::new(Voice::new(1, 0, 48_000, pre(vec![0.5; 96_000]), None, 1.0, false))))).ok();
    let mut b = run(&mut o, 2, 4800);
    ctl.cmds.push(OutCmd::SetMain(None)).ok(); b.extend(run(&mut o, 2, 4800));
    assert!(b[4000..].windows(2).all(|w| (w[1] - w[0]).abs() < 0.01)); // skip the voice's own (unfaded) start
}
```

- [ ] **Step 2: Run the tests to verify they fail.** Run: `cargo test voice`. Expected: compile errors.
- [ ] **Step 3: Implement `voice.rs`.** Build the rings in `OutputState::new` with these sizes: `cmds` 16, `trash` 64.
- [ ] **Step 4: Run the tests to verify they pass.** Run: `cargo test voice`. Expected: 8 passed.
- [ ] **Step 5: Commit:** `git add src && git commit -m "feat: real-time voice mixer"`

---

### Task 7: Engine (devices, controller, prerolls)

**Files:**
- Create: `src/engine.rs`. Modify: `src/main.rs` (`mod engine;`)

**Interfaces:**
- Consumes: `Queue` (Task 3); `decode::{read_preroll, spawn_stream}` (Task 5); `voice::*` (Task 6); `library::out_frames`.
- Produces:
```rust
#[derive(Clone, Debug, PartialEq)]
pub struct PadPlay { pub id: PadId, pub path: PathBuf, pub rate: u32, pub start: u64, pub end: u64, pub boost_db: f32 }
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceChoice { pub mic: Option<String>, pub friends: Option<String>, pub you: Option<String> }
pub enum Cmd { Enqueue(PadId), Skip, Stop, TogglePause, Seek(f32), Remove(u64), Move(u64, usize),
               Preview(PadPlay), StopPreview, SetVolumes { friends: f32, you: f32 },
               SetPads(Vec<PadPlay>), SetDevices(DeviceChoice) }
#[derive(Clone, Default)]
pub struct View { pub current: Option<(u64, PadId)>, pub up_next: Vec<(u64, PadId)>, pub pos_secs: f32, pub dur_secs: f32,
                  pub paused: bool, pub preview_secs: Option<f32>,
                  pub mic: Result<String, String>, pub friends: Result<String, String>, pub you: Result<String, String>,
                  pub notice: Option<String> }
pub struct Engine;
impl Engine {
    pub fn start(choice: DeviceChoice, volumes: (f32, f32), pads: Vec<PadPlay>, on_change: Box<dyn Fn() + Send>) -> Engine;
    pub fn sender(&self) -> std::sync::mpsc::Sender<Cmd>;
    pub fn view(&self) -> View;              // clone of Arc<Mutex<View>>
    pub fn take_notice(&self) -> Option<String>;
}
pub fn list_devices() -> (Vec<String>, Vec<String>);   // (inputs, outputs)
pub fn pick_device(names: &[String], wanted: Option<&str>, auto_contains: Option<&str>) -> Option<usize>;
```
`volumes` are slider values (0–1). The engine applies `slider_to_gain`. `Seek` takes a 0–1 fraction of the current pad.

**Controller behavior.** All of this runs on its own thread, and that thread owns every `cpal::Stream`.

- **Device selection:**
  - mic and you: `pick_device(names, wanted, None)`. `None` means the host default.
  - friends: `pick_device(names, wanted, Some("CABLE Input"))`. `None` means **off**. In that case set `View.friends = Err("VB-Cable not found")`.
  - Output streams use each device's default config. Accept F32 or I16 sample formats; anything else becomes `Err` in `View`. The mic callback downmixes to mono and pushes to a ring (1 s), which `MicIn` reads on the friends output.
- **Loop timing:** `recv_timeout(5 ms)` while something is playing and `100 ms` otherwise.
- **Each loop iteration:**
  1. Handle `Cmd`s.
  2. Drain the trash rings.
  3. `take_finished()` on the *primary* output: friends if it's open, otherwise you. A finished item means `queue.advance()`, then start it.
  4. Write `View`, and call `on_change` when `current`, `up_next`, or `paused` changed.
- **Starting an entry.** For each open output:
  - `total = out_frames(end − start, rate, dev_rate)`.
  - The preroll comes from the cache at `(pad, dev_rate)` if it's there.
  - `stream = spawn_stream(.., skip = preroll_len)`, skipped entirely if the preroll covers `total`.
  - Push `SetMain(Voice::new(entry, 0, total, preroll, stream, db_to_gain(boost), false))`.
  - If `spawn_stream` errs, set `notice = "Couldn't play '<pad name or file>': <err>"` and advance to the next entry.
- **`Enqueue`:** if nothing is current, advance immediately.
- **`Seek(f)`:** start new voices at `start + f·(end − start)` with `offset_frames` set, no preroll, and `fade_in = true`.
- **`Stop`:** `queue.stop()`, then `SetMain(None)` on both outputs.
- **`TogglePause`:** `set_paused` on both outputs.
- **`Preview` / `StopPreview`:** send `SetPreview` to the you output only.
- **Prerolls:**
  - A single worker thread builds `read_preroll(.., frames = dev_rate / 4)` for every `(pad, dev_rate)`.
  - `SetPads` invalidates any pad whose `path`, `start`, or `end` changed.
  - A pad played before its preroll is ready simply streams without one.
- **Recovery:**
  - Every 2 s, check each output and the mic: if it's wanted and missing, or `take_error()` is true, re-enumerate and try to reopen it.
  - When an output reopens while an entry is current, start that entry on it at the primary output's current position, using the seek path.
  - Report the result in `View`.

- [ ] **Step 1: Write the failing tests.**

```rust
#[test] fn pick_device_rules() {
    let n: Vec<String> = ["Speakers (Realtek)", "CABLE Input (VB-Audio Virtual Cable)", "Headset"].map(String::from).to_vec();
    assert_eq!(pick_device(&n, Some("Headset"), None), Some(2));
    assert_eq!(pick_device(&n, None, Some("CABLE Input")), Some(1));
    assert_eq!(pick_device(&n, Some("Gone"), Some("CABLE Input")), Some(1));
    assert_eq!(pick_device(&n[..1], None, Some("CABLE Input")), None);
    assert_eq!(pick_device(&n, None, None), None);
}
#[test] #[ignore] // needs real audio hardware: cargo test engine_smoke -- --ignored
fn engine_smoke() {
    let p = crate::testutil::temp_dir("smoke").join("tone.wav");
    let s: Vec<i16> = (0..48_000).map(|i| (3000.0 * (i as f32 * 0.0577).sin()) as i16).collect();
    crate::testutil::write_wav(&p, 48_000, 1, &s);
    let e = Engine::start(DeviceChoice::default(), (0.8, 0.5),
        vec![PadPlay { id: 1, path: p, rate: 48_000, start: 0, end: 48_000, boost_db: 0.0 }], Box::new(|| {}));
    e.sender().send(Cmd::Enqueue(1)).unwrap();
    let wait = |f: &dyn Fn(&View) -> bool| { let t = std::time::Instant::now();
        while !f(&e.view()) { assert!(t.elapsed().as_secs() < 4); std::thread::sleep(std::time::Duration::from_millis(10)); } };
    wait(&|v| v.current.is_some());
    wait(&|v| v.current.is_none());
}
```

- [ ] **Step 2: Run the tests to verify they fail.** Run: `cargo test engine`. Expected: compile errors.
- [ ] **Step 3: Implement `engine.rs`** as described above.
- [ ] **Step 4: Run the tests.** First run `cargo test engine`; expected: `pick_device_rules` passes. Then run `cargo test engine_smoke -- --ignored`; expected: you hear a 1 s tone in your default output, and the test passes.
- [ ] **Step 5: Commit:** `git add src && git commit -m "feat: audio engine controller"`

---

### Task 8: Global hotkeys

**Files:**
- Create: `src/hotkeys.rs`. Modify: `src/main.rs` (`mod hotkeys;`)

**Interfaces:**
- Produces:
```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HotkeyAction { Pad(PadId), Pause, Skip, Stop }
pub struct HotkeyService;
impl HotkeyService {
    pub fn start(on_press: impl Fn(HotkeyAction) + Send + 'static) -> HotkeyService;
    pub fn set_bindings(&self, b: Vec<(HotkeyAction, Hotkey)>);  // replaces all; applied on the hotkey thread
    pub fn failed(&self) -> Vec<HotkeyAction>;                     // ones Windows refused (taken by another app)
}
pub fn key_name(h: Hotkey) -> String;         // "Ctrl+Num 1", "F13"
pub fn keyboard_snapshot() -> [bool; 256];    // GetAsyncKeyState high bit per VK
pub enum Captured { Key(Hotkey), Cancel }
pub fn poll_capture(baseline: &[bool; 256]) -> Option<Captured>;
```

**Approach:**
- **Hotkey thread:**
  - The thread calls `PeekMessageW(PM_NOREMOVE)` to create its message queue, then sends its thread id back over a channel. After that it runs a `GetMessageW` loop.
  - `WM_HOTKEY` maps the id to an action and calls `on_press`.
  - `WM_APP` (sent by `set_bindings` via `PostThreadMessageW`) does three things: `UnregisterHotKey` on all ids, `RegisterHotKey(NULL, id, mods | MOD_NOREPEAT, vk)` for each binding, and records which ones failed.
- **`key_name`:** builds the modifier prefix from `mods` and gets the key's name from `GetKeyNameTextW(MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) << 16)`, setting the extended bit for navigation keys. If that returns nothing, fall back to `"Key 0x{vk:02X}"`.
- **`poll_capture`:**
  - Returns the first VK in `0x08..=0xFE` that is down now but wasn't in `baseline`.
  - It skips mouse buttons, Shift/Ctrl/Alt/Win and their left/right variants.
  - Esc (0x1B) returns `Cancel`.
  - Modifiers are read at the same moment as the key.

- [ ] **Step 1: Write the failing tests.**

```rust
#[test] fn key_names() {
    assert_eq!(key_name(Hotkey { mods: 0, vk: 0x7C }), "F13");
    assert_eq!(key_name(Hotkey { mods: 2 | 4, vk: 0x70 }), "Ctrl+Shift+F1");
}
#[test] fn registers_and_reports_failures() {
    let svc = HotkeyService::start(|_| {});
    let free = Hotkey { mods: 1 | 2 | 4, vk: 0x87 };         // Ctrl+Alt+Shift+F24
    svc.set_bindings(vec![(HotkeyAction::Skip, free), (HotkeyAction::Stop, free)]); // duplicate → second refused
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert_eq!(svc.failed(), vec![HotkeyAction::Stop]);
    svc.set_bindings(vec![]);
}
```

- [ ] **Step 2: Run the tests to verify they fail.** Run: `cargo test hotkeys`. Expected: compile errors.
- [ ] **Step 3: Implement `hotkeys.rs`** as described above.
- [ ] **Step 4: Run the tests to verify they pass.** Run: `cargo test hotkeys`. Expected: 2 passed.
- [ ] **Step 5: Commit:** `git add src && git commit -m "feat: global hotkeys via RegisterHotKey"`

---

### Task 9: Main window (grid, queue panel, now-playing, import)

**Files:**
- Create: `src/ui/mod.rs`
- Modify: `src/main.rs` (wire everything together)

**Interfaces:**
- Consumes: `Settings`, `library::*`, `Engine`/`Cmd`/`View`/`PadPlay`/`DeviceChoice`, `HotkeyService`/`HotkeyAction`/`key_name`/`poll_capture`.
- Produces:
```rust
pub struct App;  // eframe::App
impl App { pub fn new(cc: &eframe::CreationContext, settings: Settings, notice: Option<String>) -> App; }
pub fn pad_plays(s: &Settings) -> Vec<PadPlay>;       // sounds_dir().join(src.file) per pad
pub fn bindings(s: &Settings) -> Vec<(HotkeyAction, Hotkey)>;
pub enum CaptureTarget { Pad(PadId), Pause, Skip, Stop }
pub fn hotkey_field(ui: &mut egui::Ui, key: Option<Hotkey>, failed: bool) -> egui::Response; // click = start capture; right-click = clear
```
`App` keeps:
- `settings`
- `engine`
- `hotkeys`
- `capture: Option<(CaptureTarget, [bool; 256])>`
- `editor: Option<editor::Editor>` (Task 10)
- `import_rx`
- `notice: Option<(String, Instant)>`

`App::sync(&mut self)` does three things: sends `SetPads(pad_plays)`, calls `hotkeys.set_bindings(bindings)`, and runs `settings.save(settings_path())`. A save error becomes a notice. Call `sync()` after every committed change.

**Main window behavior** (copy and colors per Global Constraints):

- **Theme:**
  - `egui::Visuals::dark()` with the spec colors and corner radius 8.
  - Fonts: load `C:\Windows\Fonts\segoeui.ttf` as the proportional font and `seguisb.ttf` as a `"semibold"` family, if they exist. egui's default fonts stay as the fallback, which covers the ▶ ❚❚ ⏭ ■ glyphs.
- **Top bar** (`TopBottomPanel::top`, fill `#18191e`):
  - The title "Sound Projector".
  - The sliders **Friends hear** (accent fill) and **You hear** (green fill). Dragging a slider sends `SetVolumes`. Releasing it saves.
  - A ⚙ button that toggles the settings panel (Task 11).
- **Banners and notices** under the top bar:
  - A persistent warning row when `View.friends`, `View.you`, or `View.mic` is `Err`. For example: "Friends can't hear sounds: VB-Cable not found." with a "Setup guide" button.
  - Below it, the latest notice, which fades after 6 s.
- **Now-playing bar** (`TopBottomPanel::bottom`):
  - The current pad's name, then **Pause/Resume**, **Skip**, and **Stop**.
  - A progress bar showing the elapsed and total time (`fmt_time`). Dragging it sends `Seek` on release.
  - While something plays and the window is visible, call `ctx.request_repaint_after(100 ms)`. Otherwise, nothing repaints.
- **Queue panel** (`SidePanel::right`, about 200 px, fill `#15161b`):
  - The heading "Up next", then one row per entry: a ⋮⋮ drag handle (egui `dnd_drag_source` / `dnd_drop_zone` → `Cmd::Move`), the pad name, and × (`Cmd::Remove`).
  - When the queue is empty, show a quiet line: "Click a pad or press its hotkey to queue it."
- **Pad grid** (`CentralPanel` inside a `ScrollArea`):
  - The number of columns is `max(1, width / 180)`.
  - Each pad shows its name (semibold); a meta row with either "Song clip · m:ss" (`is_clip`) or "m:ss"; "#n in queue" (`position_of + 1`); and the hotkey badge, shown red with the tooltip "Another app is using this key" when it's in `failed()`.
  - The playing pad gets an accent border and a `#211f33` fill.
  - Left-click sends `Enqueue`.
  - Right-click opens `context_menu`: **Edit**, **Rename**, **Set hotkey…**, **Delete**.
    - **Delete** asks "Delete '<name>'?". If it was the source's last pad, the library file is deleted too.
  - The last tile is "+ Add", which opens an `rfd::FileDialog` filtered to mp3, wav, ogg, and flac.
- **Import:**
  - Files dropped on the window (`raw.dropped_files`) or picked in the dialog import on a thread (`library::import`). The results come back over `import_rx`.
  - On success: push the `Source` plus `whole_pad`, then `sync()`.
  - On failure: notice "Couldn't add '<file>': not a supported audio file (MP3, WAV, OGG, FLAC)."
- **Hotkey capture:**
  - Starting a capture first calls `hotkeys.set_bindings(vec![])`, so existing hotkeys don't fire while the user presses keys.
  - A small modal shows "Press a key… (Esc to cancel)" and the tip "Pick keys your game doesn't use, like the numpad or F13–F24."
  - While it's open, call `poll_capture` every frame (`request_repaint`).
  - If the key is already bound elsewhere, refuse with "Already used by '<name>'". Either way, `sync()` restores the bindings afterwards.
- **Startup:**
  - `main.rs` calls `Settings::load` and then `run_native`.
  - `App::new` does the rest in this order:
    1. `Engine::start(.., on_change = move || ctx.request_repaint())`
    2. `HotkeyService::start(..)`. Its closure holds an `engine.sender()` and maps `Pad(id)` → `Enqueue(id)`, `Pause` → `TogglePause`, `Skip` → `Skip`, and `Stop` → `Stop`.
    3. `sync()`

- [ ] **Step 1: Run the full test suite as a baseline.** Run: `cargo test`. Expected: all pass except the ignored test.
- [ ] **Step 2: Implement `ui/mod.rs` and wire up `main.rs`** as described above.
- [ ] **Step 3: Check it by hand.** Run `cargo run`, then:
  1. Drag in 3 files (one of them about 4 minutes long) → 3 pads appear with their lengths.
  2. Click two pads → the first plays and the second shows "#1 in queue", and both appear in Up next.
  3. Skip, Pause/Resume, Stop, and seek all work. Drag the second entry above the first; × removes an entry.
  4. Set a hotkey to Num 1 → with another window focused, pressing it queues the pad.
  5. Assign the same key to another pad → you see "Already used by…".
  6. Drop a `.txt` renamed to `.mp3` → the notice appears and nothing crashes.
  7. Delete `%APPDATA%\SoundProjector\sounds\<file>`, then click its pad → a "Couldn't play…" notice appears and the queue moves on.
  8. Quit and relaunch → the pads, hotkeys, and slider levels persist.
- [ ] **Step 4: Run the lint checks.** Run: `cargo clippy -- -D warnings`. Expected: no warnings.
- [ ] **Step 5: Commit:** `git add src && git commit -m "feat: main window with pad grid, queue and now-playing"`

---

### Task 10: Editor

**Files:**
- Create: `src/ui/editor.rs`. Modify: `src/ui/mod.rs` (open, show, and act on the editor)

**Interfaces:**
- Consumes: `decode::compute_peaks`, `library::{clamp_region, secs_to_frames, frames_to_secs}`, `View.preview_secs`, `hotkey_field`.
- Produces:
```rust
pub struct Editor { pub source: u32, /* peaks rx, selected pad, drag state */ }
pub enum EditorAction { None, Done, Changed, Preview(PadId), StopPreview, Capture(PadId), Delete(PadId) }
impl Editor {
    pub fn open(src: &Source, path: PathBuf) -> Editor;                 // compute_peaks(path, 256) on a thread
    pub fn show(&mut self, ui: &mut egui::Ui, s: &mut Settings, view: &View, failed: &[HotkeyAction]) -> EditorAction;
}
```

**Behavior:**

- **Placement:** the editor replaces the pad grid in the `CentralPanel`, and the queue panel and now-playing bar stay. The header shows "Editing: <source name> (m:ss)", the line "Original file is never changed", and a **Done** button.
- **Waveform:**
  - The peaks are drawn with `ui.painter()`, one min/max line per pixel column, in `#4a4b57` on `#16171c`. Show "Loading waveform…" until they arrive.
  - Each of the source's pads is a translucent region with a 2 px edge and a name tag. Colors cycle through `#7c6cff`, `#3ecf8e`, `#f5a524`, `#ef5da8`, `#4cc9f0`.
  - Clicking a region selects it.
  - Dragging within 6 px of a region edge moves that edge, through `clamp_region` with `min_len = 0.05 s`.
  - Dragging across empty waveform creates a new pad, "Section N", inserted after the source's last pad, and selects it. A drag shorter than 0.05 s counts as a click.
  - While previewing, draw a white playhead line at the region start plus `preview_secs`.
- **Selected pad fields:**
  - Name (`TextEdit`).
  - Start and End as `DragValue`s in seconds (step 0.01), clamped through `clamp_region`.
  - **Boost**: a `Slider` from 0 to 18 dB with a "+n dB" suffix.
  - The hotkey field.
  - **▶ Preview** / **■ Stop preview**.
  - **Delete section**. This is disabled when it's the only pad, because deleting the sound itself happens from the grid.
- **When to sync:** `Changed` is returned when a drag is released, when a text field loses focus, and when a slider is released. `App` then calls `sync()`.

- [ ] **Step 1: Implement `editor.rs` and hook it into `App`.**
- [ ] **Step 2: Check it by hand.** Run `cargo run`, then:
  1. Edit the 4-minute song → the waveform appears.
  2. Drag out two sections → two new pads with "Song clip" appear in the grid.
  3. Drag a region's edges, then type an exact start time → the region follows.
  4. Set boost to +12 dB and preview → it's louder with no crackle, you hear it, and nothing reaches CABLE.
  5. The original file in the import location is unchanged (compare its size and modified time).
- [ ] **Step 3: Run the lint checks.** Run: `cargo clippy -- -D warnings`. Expected: no warnings.
- [ ] **Step 4: Commit:** `git add src && git commit -m "feat: waveform editor with sections, trim and boost"`

---

### Task 11: Settings panel, tray, autostart, single instance

**Files:**
- Create: `src/tray.rs`, `src/ui/settings_panel.rs`
- Modify: `src/main.rs`, `src/ui/mod.rs`

**Interfaces:**
- Produces:
```rust
// tray.rs
pub const WINDOW_TITLE: &str = "Sound Projector";
pub fn icon_rgba() -> (Vec<u8>, u32, u32);      // 32×32 accent-purple circle, generated in code (no asset file)
pub fn find_window() -> Option<isize>;          // FindWindowW(null, WINDOW_TITLE)
pub fn show_window(hwnd: isize);                // ShowWindow(SW_SHOW) + SetForegroundWindow
pub fn hide_window(hwnd: isize);                // ShowWindow(SW_HIDE)
pub fn create_tray(ctx: egui::Context) -> tray_icon::TrayIcon; // keep alive in App
pub fn set_autostart(on: bool) -> Result<(), String>;          // HKCU\...\Run "SoundProjector" = "\"<exe>\" --tray"
pub fn already_running() -> bool;               // CreateMutexW("SoundProjector.SingleInstance") + ERROR_ALREADY_EXISTS
// ui/settings_panel.rs
pub fn show(ctx: &egui::Context, open: &mut bool, s: &mut Settings, view: &View, devices: &(Vec<String>, Vec<String>),
            failed: &[HotkeyAction]) -> SettingsAction;
pub enum SettingsAction { None, DevicesChanged, Capture(CaptureTarget), AutostartChanged }
```

**Behavior:**

- **Single instance** (`main.rs`): if `already_running()`, call `find_window().map(show_window)` and return before eframe starts.
- **Window handle and tray** (`App::new`): get the `hwnd` with `find_window()` on the first frame, then `create_tray`.
  - Tray menu: **Show Sound Projector**, **Quit**. Left-clicking the tray icon also shows the window.
  - The event handlers are set with `TrayIconEvent::set_event_handler` / `MenuEvent::set_event_handler`. **Show** calls `show_window(find_window())` and `ctx.request_repaint()`. **Quit** calls `std::process::exit(0)` with the comment `// ponytail: settings are saved on every change, so a hard exit loses nothing but eframe's last window-geometry autosave`.
- **Close to tray:** in `update`, when `close_requested()`, send `ViewportCommand::CancelClose` and call `hide_window`. When launched with `--tray`, hide on the first frame.
- **Settings window** (an `egui::Window` titled "Settings"):
  - **Devices:** a combo box each for **Mic** ("Default" + inputs), **Friends output** ("Auto (VB-Cable)" + outputs), and **You hear output** ("Default" + outputs). Changing one saves and sends `Cmd::SetDevices`. Each row shows its `View` status: the device name in use, or the error in red.
  - **Control hotkeys:** fields for **Pause/Resume**, **Skip**, and **Stop**, using `hotkey_field`. These are part of `bindings()`.
  - **Start with Windows:** a checkbox that calls `set_autostart`. An error becomes a notice.
  - **Setup guide:** a collapsing section, open by default when friends is `Err`. It contains the spec's six setup steps verbatim and the link `https://vb-audio.com/Cable/`.
- **Window icon:** set it from `icon_rgba()` via `ViewportBuilder::with_icon`.

- [ ] **Step 1: Implement `tray.rs`, `settings_panel.rs`, and the wiring.**
- [ ] **Step 2: Check it by hand.** Run `cargo run`, then:
  1. Close the window → it goes to the tray, and a pad hotkey still plays sound.
  2. Tray → **Show** brings the window back. **Quit** exits.
  3. Launch a second copy → the first window comes up and the second copy exits.
  4. Turn on Start with Windows → `reg query HKCU\Software\Microsoft\Windows\CurrentVersion\Run /v SoundProjector` shows the value. Turn it off → the value is gone.
  5. Switch the "You hear" device to another output → the sound moves there.
  6. Unplug a USB headset mid-song → the warning row appears. Plug it back in → it recovers within about 2 s and the song continues.
- [ ] **Step 3: Run the lint checks.** Run: `cargo clippy -- -D warnings`. Expected: no warnings.
- [ ] **Step 4: Commit:** `git add src && git commit -m "feat: settings panel, tray, autostart, single instance"`

---

### Task 12: README, release build, real-world check

**Files:**
- Create: `README.md`

- [ ] **Step 1: Write `README.md`.** It covers what the app is, in one paragraph; the spec's setup guide, all six steps verbatim; the build steps (`rustup` with the GNU host, `cargo build --release`, output at `target\release\sound-projector.exe`); and where the library and settings live.
- [ ] **Step 2: Build the release.** Run: `cargo build --release; (Get-Item target\release\sound-projector.exe).Length / 1MB`. Expected: the build succeeds and the exe is about 15 MB or less.
- [ ] **Step 3: Check resource use.**
  1. Start the release exe with about 50 pads, including 5 full songs.
  2. Run `Get-Process sound-projector | Select-Object CPU, WorkingSet64`, twice, 10 s apart, while idle with the window hidden.
  3. Expected: the CPU time barely moves between the two samples (about 0%), and the working set is roughly 120 MB or less.
- [ ] **Step 4: Check it in a real Discord call.** The user installs VB-Cable and reboots first. Then the spec's checklist:
  - songs arrive crisp
  - no lag
  - your voice still comes through
  - the two sliders work independently
  - hotkeys work with a fullscreen game focused
  - a full song streams start to finish without dropouts

  Record anything that fails as a follow-up.
- [ ] **Step 5: Commit:** `git add README.md && git commit -m "docs: README with setup guide"`
