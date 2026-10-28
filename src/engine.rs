use crate::decode::spawn_stream;
use crate::dsp::{DriftResampler, db_to_gain, slider_to_gain};
use crate::library::{frames_to_secs, out_frames};
use crate::queue::{Queue, QueueEntry};
use crate::settings::PadId;
use crate::voice::{MicIn, OutCmd, OutputControl, OutputShared, OutputState, Voice};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub struct PadPlay {
    pub id: PadId,
    pub path: PathBuf,
    pub rate: u32,
    pub start: u64,
    pub end: u64,
    pub boost_db: f32,
}

impl PadPlay {
    fn same_region(&self, o: &PadPlay) -> bool {
        (&self.path, self.rate, self.start, self.end) == (&o.path, o.rate, o.start, o.end)
    }
    fn label(&self) -> String {
        self.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

/// Slider positions (0..=1) and the global boost in dB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volumes {
    pub friends: f32,
    pub you: f32,
    pub boost_db: f32,
}

/// `None` = Windows default (mic, you) or auto-detect VB-Cable (friends).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceChoice {
    pub mic: Option<String>,
    pub friends: Option<String>,
    pub you: Option<String>,
}

pub enum Cmd {
    Enqueue(PadId),
    Skip,
    Stop,
    TogglePause,
    Seek(f32),
    Remove(u64),
    Move(u64, usize),
    Preview(PadPlay),
    StopPreview,
    SetVolumes(Volumes),
    SetPads(Vec<PadPlay>),
    SetDevices(DeviceChoice),
}

#[derive(Clone)]
pub struct View {
    pub current: Option<(u64, PadId)>,
    pub up_next: Vec<(u64, PadId)>,
    pub pos_secs: f32,
    pub dur_secs: f32,
    pub paused: bool,
    pub preview_secs: Option<f32>,
    pub mic: Result<String, String>,
    pub friends: Result<String, String>,
    pub you: Result<String, String>,
    pub notice: Option<String>,
}

impl Default for View {
    fn default() -> Self {
        View {
            current: None,
            up_next: Vec::new(),
            pos_secs: 0.0,
            dur_secs: 0.0,
            paused: false,
            preview_secs: None,
            mic: Ok(String::new()),
            friends: Ok(String::new()),
            you: Ok(String::new()),
            notice: None,
        }
    }
}

pub struct Engine {
    tx: Sender<Cmd>,
    view: Arc<Mutex<View>>,
}

impl Engine {
    pub fn start(choice: DeviceChoice, volumes: Volumes, pads: Vec<PadPlay>, on_change: Box<dyn Fn() + Send>) -> Engine {
        let (tx, rx) = channel();
        let view = Arc::new(Mutex::new(View::default()));
        let v = view.clone();
        std::thread::Builder::new()
            .name("sp-engine".into())
            .spawn(move || Controller::new(choice, volumes, pads, v, on_change).run(rx))
            .expect("spawn engine thread");
        Engine { tx, view }
    }

    pub fn sender(&self) -> Sender<Cmd> {
        self.tx.clone()
    }

    pub fn view(&self) -> View {
        self.view.lock().unwrap().clone()
    }

    pub fn take_notice(&self) -> Option<String> {
        self.view.lock().unwrap().notice.take()
    }
}

fn device_name(d: &cpal::Device) -> String {
    d.description().map(|x| x.name().to_string()).unwrap_or_default()
}

pub fn list_devices() -> (Vec<String>, Vec<String>) {
    let host = cpal::default_host();
    let ins = host.input_devices().map(|d| d.map(|x| device_name(&x)).collect()).unwrap_or_default();
    let outs = host.output_devices().map(|d| d.map(|x| device_name(&x)).collect()).unwrap_or_default();
    (ins, outs)
}

/// The picked name if it's there. Otherwise friends (`auto`) takes the first name containing it, and the
/// others take Windows' default, or the first device when that's a VB-Cable end: the app must never hear
/// its own cable as your mic, or play "You hear" into it.
pub fn pick_device(names: &[String], picked: Option<&str>, auto: Option<&str>, default: Option<&str>) -> Option<usize> {
    let mine = |n: &str| !n.contains("CABLE");
    picked.and_then(|w| names.iter().position(|n| n == w)).or_else(|| match auto {
        Some(a) => names.iter().position(|n| n.contains(a)),
        None => default
            .filter(|d| mine(d))
            .and_then(|d| names.iter().position(|n| n == d))
            .or_else(|| names.iter().position(|n| mine(n))),
    })
}

/// The device (and its name) a slot should be on right now; see `pick_device`.
fn find(input: bool, picked: Option<&str>, auto: Option<&str>) -> Option<(cpal::Device, String)> {
    let host = cpal::default_host();
    let (mut devs, default): (Vec<cpal::Device>, _) = if input {
        (host.input_devices().map(|d| d.collect()).unwrap_or_default(), host.default_input_device())
    } else {
        (host.output_devices().map(|d| d.collect()).unwrap_or_default(), host.default_output_device())
    };
    let mut names: Vec<String> = devs.iter().map(device_name).collect();
    let default_name = default.as_ref().map(device_name);
    let i = pick_device(&names, picked, auto, default_name.as_deref())?;
    let dev = match default {
        // Windows' default itself rather than the same endpoint from the list, so cpal reports when it changes.
        Some(d) if auto.is_none() && picked != Some(&names[i]) && default_name.as_ref() == Some(&names[i]) => d,
        _ => devs.swap_remove(i),
    };
    Some((dev, names.swap_remove(i)))
}

/// Xruns and a refused real-time boost come through the error callback, but the stream keeps running.
fn fatal(e: &cpal::Error) -> bool {
    !matches!(e.kind(), cpal::ErrorKind::Xrun | cpal::ErrorKind::RealtimeDenied)
}

pub const CABLE: &str = "CABLE Input";
const RETRY: Duration = Duration::from_secs(2);

struct Out {
    _stream: cpal::Stream,
    rate: u32,
    ctl: OutputControl,
    shared: Arc<OutputShared>,
}

impl Out {
    fn send(&mut self, cmd: OutCmd) {
        // ponytail: a full command ring (16) drops the command; the controller sends a few per user action
        let _ = self.ctl.cmds.push(cmd);
    }
}

struct Controller {
    choice: DeviceChoice,
    volumes: Volumes,
    pads: HashMap<PadId, PadPlay>,
    queue: Queue,
    paused: bool,
    friends: Option<Out>,
    you: Option<Out>,
    mic: Option<cpal::Stream>,
    mic_error: Arc<AtomicBool>,
    status: (Result<String, String>, Result<String, String>, Result<String, String>), // mic, friends, you
    prerolls: HashMap<(PadId, u32), Arc<[f32]>>,
    jobs: Sender<(PadPlay, u32)>,
    done: Receiver<(PadPlay, u32, Result<Vec<f32>, String>)>,
    last_src_pos: u64,
    last_retry: Instant,
    view: Arc<Mutex<View>>,
    on_change: Box<dyn Fn() + Send>,
}

impl Controller {
    fn new(choice: DeviceChoice, volumes: Volumes, pads: Vec<PadPlay>, view: Arc<Mutex<View>>, on_change: Box<dyn Fn() + Send>) -> Controller {
        let (jobs, job_rx) = channel::<(PadPlay, u32)>();
        let (done_tx, done) = channel();
        std::thread::Builder::new()
            .name("sp-preroll".into())
            .spawn(move || {
                for (p, rate) in job_rx {
                    let r = crate::decode::read_preroll(&p.path, p.start, p.end, rate, rate as usize / 4);
                    if done_tx.send((p, rate, r)).is_err() {
                        return;
                    }
                }
            })
            .expect("spawn preroll thread");
        let mut c = Controller {
            choice,
            volumes,
            pads: pads.into_iter().map(|p| (p.id, p)).collect(),
            queue: Queue::default(),
            paused: false,
            friends: None,
            you: None,
            mic: None,
            mic_error: Arc::new(AtomicBool::new(false)),
            status: (Err(String::new()), Err(String::new()), Err(String::new())),
            prerolls: HashMap::new(),
            jobs,
            done,
            last_src_pos: 0,
            last_retry: Instant::now(),
            view,
            on_change,
        };
        c.open_friends();
        c.open_you();
        c
    }

    fn run(mut self, rx: Receiver<Cmd>) {
        loop {
            let wait = if self.queue.current().is_some() { Duration::from_millis(5) } else { Duration::from_millis(100) };
            match rx.recv_timeout(wait) {
                Ok(cmd) => self.handle(cmd),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            while let Ok(cmd) = rx.try_recv() {
                self.handle(cmd);
            }
            for out in [&mut self.friends, &mut self.you].into_iter().flatten() {
                while out.ctl.trash.pop().is_ok() {}
            }
            while let Ok((p, rate, r)) = self.done.try_recv() {
                if let (Ok(v), Some(cur)) = (r, self.pads.get(&p.id))
                    && cur.same_region(&p) {
                        self.prerolls.insert((p.id, rate), v.into());
                    }
            }
            let finished = self.primary().and_then(|o| o.shared.take_finished());
            if let Some(o) = self.secondary() {
                o.shared.take_finished();
            }
            if finished.is_some() && finished == self.queue.current().map(|e| e.id) {
                self.advance();
            }
            if self.last_retry.elapsed() >= RETRY {
                self.last_retry = Instant::now();
                self.recover();
            }
            self.publish();
        }
    }

    /// Friends drives the queue when it's open; otherwise the headphones do.
    fn primary(&self) -> Option<&Out> {
        if self.friends.is_some() { self.friends.as_ref() } else { self.you.as_ref() }
    }

    fn secondary(&self) -> Option<&Out> {
        if self.friends.is_some() { self.you.as_ref() } else { None }
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Enqueue(pad) => {
                if self.pads.contains_key(&pad) {
                    self.queue.enqueue(pad);
                    if self.queue.current().is_none() {
                        self.advance();
                    }
                }
            }
            Cmd::Skip => self.advance(),
            Cmd::Stop => {
                self.queue.stop();
                self.silence_all();
                self.set_paused(false);
            }
            Cmd::TogglePause => {
                if self.queue.current().is_some() {
                    self.set_paused(!self.paused);
                }
            }
            Cmd::Seek(f) => {
                if let Some(e) = self.queue.current()
                    && let Some(p) = self.pads.get(&e.pad) {
                        let at = p.start + ((p.end - p.start) as f64 * f.clamp(0.0, 1.0) as f64) as u64;
                        self.start_entry(e, Some(at));
                    }
            }
            Cmd::Remove(id) => {
                self.queue.remove(id);
            }
            Cmd::Move(id, i) => self.queue.move_entry(id, i),
            Cmd::Preview(p) => {
                if let Some(you) = self.you.as_mut() {
                    let total = out_frames(p.end - p.start, p.rate, you.rate);
                    match spawn_stream(&p.path, p.start, p.end, you.rate, 0) {
                        Ok(s) => you.send(OutCmd::SetPreview(Some(Box::new(Voice::new(0, 0, total, None, Some(s), db_to_gain(p.boost_db), p.start > 0))))),
                        Err(e) => self.notice(format!("Couldn't preview '{}': {e}", p.label())),
                    }
                }
            }
            Cmd::StopPreview => {
                if let Some(you) = self.you.as_mut() {
                    you.send(OutCmd::SetPreview(None));
                }
            }
            Cmd::SetVolumes(v) => {
                self.volumes = v;
                self.apply_volumes();
            }
            Cmd::SetPads(list) => self.set_pads(list),
            Cmd::SetDevices(choice) => {
                self.choice = choice;
                self.friends = None;
                self.mic = None;
                self.you = None;
                self.open_friends();
                self.open_you();
                self.resume_current();
            }
        }
    }

    fn set_paused(&mut self, p: bool) {
        self.paused = p;
        for out in [&self.friends, &self.you].into_iter().flatten() {
            out.shared.set_paused(p);
        }
    }

    /// The boost rides on the output gain, so it changes a playing song live (the mic is mixed in after it).
    fn apply_volumes(&self) {
        let boost = db_to_gain(self.volumes.boost_db);
        if let Some(o) = &self.friends {
            o.shared.set_gain(slider_to_gain(self.volumes.friends) * boost);
        }
        if let Some(o) = &self.you {
            o.shared.set_gain(slider_to_gain(self.volumes.you) * boost);
        }
    }

    fn notice(&mut self, msg: String) {
        self.view.lock().unwrap().notice = Some(msg);
        (self.on_change)();
    }

    /// Move to the next entry, skipping any that can't start. Empty queue → silence.
    fn advance(&mut self) {
        loop {
            match self.queue.advance() {
                None => {
                    self.silence_all();
                    self.set_paused(false);
                    return;
                }
                Some(e) => {
                    if self.start_entry(e, None) {
                        return;
                    }
                }
            }
        }
    }

    /// Start `e` on every open output, from the top (with preroll) or from source frame `at`.
    fn start_entry(&mut self, e: QueueEntry, at: Option<u64>) -> bool {
        let Some(p) = self.pads.get(&e.pad).cloned() else { return false };
        self.last_src_pos = at.unwrap_or(p.start);
        let prerolls = &self.prerolls;
        let mut err = None;
        let make = |rate: u32| -> Result<Option<Box<Voice>>, String> {
            let total = out_frames(p.end - p.start, p.rate, rate);
            let gain = db_to_gain(p.boost_db);
            let v = match at {
                None => {
                    let pre = prerolls.get(&(p.id, rate)).cloned();
                    let pre_frames = pre.as_ref().map_or(0, |x| x.len() / 2) as u64;
                    let stream = if pre_frames >= total { None } else { Some(spawn_stream(&p.path, p.start, p.end, rate, pre_frames)?) };
                    Voice::new(e.id, 0, total, pre, stream, gain, p.start > 0)
                }
                Some(at) => {
                    let offset = out_frames(at.saturating_sub(p.start), p.rate, rate);
                    let stream = spawn_stream(&p.path, at, p.end, rate, 0)?;
                    Voice::new(e.id, offset, total, None, Some(stream), gain, true)
                }
            };
            Ok(Some(Box::new(v)))
        };
        for out in [&mut self.friends, &mut self.you].into_iter().flatten() {
            match make(out.rate) {
                Ok(v) => out.send(OutCmd::SetMain(v)),
                Err(x) => {
                    err = Some(x);
                    break;
                }
            }
        }
        if let Some(x) = err {
            self.notice(format!("Couldn't play '{}': {x}", p.label()));
            return false;
        }
        true
    }

    fn silence_all(&mut self) {
        for out in [&mut self.friends, &mut self.you].into_iter().flatten() {
            out.send(OutCmd::SetMain(None));
        }
    }

    /// After a device (re)opens mid-item, pick the current item up where it was.
    fn resume_current(&mut self) {
        if let Some(e) = self.queue.current() {
            let at = self.last_src_pos;
            self.start_entry(e, Some(at));
        }
    }

    fn set_pads(&mut self, list: Vec<PadPlay>) {
        let new: HashMap<PadId, PadPlay> = list.into_iter().map(|p| (p.id, p)).collect();
        for (id, old) in &self.pads {
            if new.get(id).is_none_or(|n| !n.same_region(old)) {
                self.prerolls.retain(|(pid, _), _| pid != id);
            }
        }
        let removed: Vec<PadId> = self.pads.keys().filter(|id| !new.contains_key(id)).copied().collect();
        let changed: Vec<PadPlay> =
            new.values().filter(|p| self.pads.get(&p.id).is_none_or(|o| !o.same_region(p))).cloned().collect();
        self.pads = new;
        let mut current_gone = false;
        for id in removed {
            current_gone |= self.queue.remove_pad(id);
        }
        if current_gone {
            self.advance();
        }
        for rate in self.rates() {
            for p in &changed {
                let _ = self.jobs.send((p.clone(), rate));
            }
        }
    }

    fn rates(&self) -> Vec<u32> {
        let mut r: Vec<u32> = [&self.friends, &self.you].into_iter().flatten().map(|o| o.rate).collect();
        r.dedup();
        r
    }

    fn queue_prerolls(&self, rate: u32) {
        for p in self.pads.values() {
            if !self.prerolls.contains_key(&(p.id, rate)) {
                let _ = self.jobs.send((p.clone(), rate));
            }
        }
    }

    fn open_friends(&mut self) {
        self.friends = None;
        self.mic = None;
        self.mic_error.store(false, Relaxed);
        let Some((dev, name)) = find(false, self.choice.friends.as_deref(), Some(CABLE)) else {
            self.status.1 = Err("VB-Cable not found".into());
            self.status.0 = Err("Not in use until VB-Cable is set up".into());
            return;
        };
        let rate = dev.default_output_config().map(|c| c.sample_rate()).unwrap_or(48_000);
        let mic = match open_mic(self.choice.mic.as_deref(), self.mic_error.clone()) {
            Ok((stream, ring, mic_rate, name)) => {
                self.mic = Some(stream);
                self.status.0 = Ok(name);
                Some(MicIn { ring, resampler: DriftResampler::new(mic_rate, rate) })
            }
            Err(e) => {
                self.status.0 = Err(e);
                None
            }
        };
        match open_output(&dev, mic) {
            Ok(out) => {
                self.status.1 = Ok(name);
                out.shared.set_paused(self.paused);
                self.queue_prerolls(out.rate);
                self.friends = Some(out);
                self.apply_volumes();
            }
            Err(e) => {
                self.mic = None;
                self.status.1 = Err(e);
            }
        }
    }

    fn open_you(&mut self) {
        self.you = None;
        let Some((dev, name)) = find(false, self.choice.you.as_deref(), None) else {
            self.status.2 = Err("No headphones or speakers found".into());
            return;
        };
        match open_output(&dev, None) {
            Ok(out) => {
                self.status.2 = Ok(name);
                out.shared.set_paused(self.paused);
                self.queue_prerolls(out.rate);
                self.you = Some(out);
                self.apply_volumes();
            }
            Err(e) => self.status.2 = Err(e),
        }
    }

    /// Every couple of seconds: reopen anything that errored (cpal also reports Windows' default
    /// changing), is missing, or fell back from a picked device that has come back. Reopening the mic
    /// means reopening the friends output it feeds, so a mic that's there but won't open (privacy
    /// setting, held by another app) is tried on its own first, instead of rebuilding friends every pass.
    fn recover(&mut self) {
        // Only enumerates while on a fallback, so a normal idle pass costs nothing.
        let back = |picked: &Option<String>, using: &Result<String, String>, input: bool, auto: Option<&str>| {
            picked.as_ref().is_some_and(|p| {
                using.as_ref() != Ok(p) && find(input, Some(p), auto).is_some_and(|(_, n)| &n == p)
            })
        };
        let friends_bad = self.friends.as_ref().is_none_or(|o| o.shared.take_error())
            || back(&self.choice.friends, &self.status.1, false, Some(CABLE));
        let mic_bad = self.mic_error.swap(false, Relaxed)
            || (self.friends.is_some()
                && (self.mic.is_none() || back(&self.choice.mic, &self.status.0, true, None))
                && open_mic(self.choice.mic.as_deref(), Arc::default()).is_ok());
        let you_bad = self.you.as_ref().is_none_or(|o| o.shared.take_error())
            || back(&self.choice.you, &self.status.2, false, None);
        if friends_bad || mic_bad {
            self.open_friends();
            if self.friends.is_some() {
                self.resume_on_friends();
            }
        }
        if you_bad {
            self.open_you();
            if self.you.is_some() {
                self.resume_on_you();
            }
        }
    }

    fn resume_on_friends(&mut self) {
        let you = self.you.take();
        self.resume_current();
        self.you = you;
    }

    fn resume_on_you(&mut self) {
        let friends = self.friends.take();
        self.resume_current();
        self.friends = friends;
    }

    fn publish(&mut self) {
        let cur = self.queue.current();
        let pad = cur.and_then(|e| self.pads.get(&e.pad));
        let (pos, dur) = match (cur, pad, self.primary().map(|o| (o.shared.main_entry(), o.shared.main_frames(), o.rate))) {
            (Some(e), Some(p), Some((entry, frames, rate))) => {
                let dur = frames_to_secs(p.end - p.start, p.rate);
                let pos = if entry == e.id { frames as f64 / rate as f64 } else { 0.0 };
                (pos, dur)
            }
            (_, Some(p), _) => (0.0, frames_to_secs(p.end - p.start, p.rate)),
            _ => (0.0, 0.0),
        };
        if let Some(p) = pad {
            self.last_src_pos = p.start + (pos * p.rate as f64) as u64;
        }
        let preview = self.you.as_ref().and_then(|o| o.shared.preview_frames().map(|f| f as f32 / o.rate as f32));
        let up_next: Vec<(u64, PadId)> = self.queue.up_next().iter().map(|e| (e.id, e.pad)).collect();
        let mut v = self.view.lock().unwrap();
        let changed = v.current != cur.map(|e| (e.id, e.pad))
            || v.up_next != up_next
            || v.paused != self.paused
            || v.preview_secs.is_some() != preview.is_some()
            || (&v.mic, &v.friends, &v.you) != (&self.status.0, &self.status.1, &self.status.2);
        v.current = cur.map(|e| (e.id, e.pad));
        v.up_next = up_next;
        v.pos_secs = pos as f32;
        v.dur_secs = dur as f32;
        v.paused = self.paused;
        v.preview_secs = preview;
        v.mic = self.status.0.clone();
        v.friends = self.status.1.clone();
        v.you = self.status.2.clone();
        drop(v);
        if changed {
            (self.on_change)();
        }
    }
}

/// Opens the mic (downmixed to mono) into a 1 s ring. Returns (stream, ring, rate, device name).
fn open_mic(wanted: Option<&str>, err: Arc<AtomicBool>) -> Result<(cpal::Stream, rtrb::Consumer<f32>, u32, String), String> {
    let (dev, name) = find(true, wanted, None).ok_or("No microphone found")?;
    let cfg = dev.default_input_config().map_err(|e| e.to_string())?;
    let (rate, ch) = (cfg.sample_rate(), cfg.channels() as usize);
    let (mut tx, rx) = rtrb::RingBuffer::new(rate as usize);
    let on_err = move |e: cpal::Error| {
        if fatal(&e) {
            err.store(true, Relaxed)
        }
    };
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => dev.build_input_stream::<f32, _, _>(
            cfg.into(),
            move |data: &[f32], _| {
                for f in data.chunks_exact(ch) {
                    let _ = tx.push(f.iter().sum::<f32>() / ch as f32);
                }
            },
            on_err,
            None,
        ),
        cpal::SampleFormat::I16 => dev.build_input_stream::<i16, _, _>(
            cfg.into(),
            move |data: &[i16], _| {
                for f in data.chunks_exact(ch) {
                    let _ = tx.push(f.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / ch as f32);
                }
            },
            on_err,
            None,
        ),
        other => return Err(format!("Unsupported mic format {other:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, rx, rate, name))
}

fn open_output(dev: &cpal::Device, mic: Option<MicIn>) -> Result<Out, String> {
    let cfg = dev.default_output_config().map_err(|e| e.to_string())?;
    let (rate, ch) = (cfg.sample_rate(), cfg.channels() as usize);
    let (mut state, ctl, shared) = OutputState::new(ch, rate, mic);
    let err_shared = shared.clone();
    let on_err = move |e: cpal::Error| {
        if fatal(&e) {
            err_shared.set_error()
        }
    };
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => {
            dev.build_output_stream::<f32, _, _>(cfg.into(), move |data: &mut [f32], _| state.render(data), on_err, None)
        }
        cpal::SampleFormat::I16 => {
            let mut buf = vec![0.0f32; 4096 * ch];
            dev.build_output_stream::<i16, _, _>(
                cfg.into(),
                move |data: &mut [i16], _| {
                    for chunk in data.chunks_mut(buf.len()) {
                        let b = &mut buf[..chunk.len()];
                        state.render(b);
                        for (d, s) in chunk.iter_mut().zip(b.iter()) {
                            *d = (s * 32767.0) as i16;
                        }
                    }
                },
                on_err,
                None,
            )
        }
        other => return Err(format!("Unsupported output format {other:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Out { _stream: stream, rate, ctl, shared })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_device_rules() {
        let n: Vec<String> = ["Speakers (Realtek)", "CABLE Input (VB-Audio Virtual Cable)", "Headset"].map(String::from).to_vec();
        let cable = Some("CABLE Input (VB-Audio Virtual Cable)");
        assert_eq!(pick_device(&n, Some("Headset"), None, None), Some(2));
        assert_eq!(pick_device(&n, None, Some("CABLE Input"), None), Some(1));
        assert_eq!(pick_device(&n, Some("Gone"), Some("CABLE Input"), None), Some(1));
        assert_eq!(pick_device(&n[..1], None, Some("CABLE Input"), Some("Speakers (Realtek)")), None); // friends never falls back
        assert_eq!(pick_device(&n, Some("Gone"), None, Some("Headset")), Some(2));
        assert_eq!(pick_device(&n, None, None, cable), Some(0)); // a VB-Cable default is skipped
        assert_eq!(pick_device(&n, cable, None, None), Some(1)); // unless picked
    }

    #[test]
    #[ignore] // needs real audio hardware: cargo test engine_smoke -- --ignored
    fn engine_smoke() {
        let p = crate::testutil::temp_dir("smoke").join("tone.wav");
        let s: Vec<i16> = (0..48_000).map(|i| (3000.0 * (i as f32 * 0.0577).sin()) as i16).collect();
        crate::testutil::write_wav(&p, 48_000, 1, &s);
        // "You hear" at 0 so the check is silent; it still runs through the real device callbacks.
        let e = Engine::start(
            DeviceChoice::default(),
            Volumes { friends: 0.8, you: 0.0, boost_db: 0.0 },
            vec![PadPlay { id: 1, path: p, rate: 48_000, start: 0, end: 48_000, boost_db: 0.0 }],
            Box::new(|| {}),
        );
        e.sender().send(Cmd::Enqueue(1)).unwrap();
        let wait = |f: &dyn Fn(&View) -> bool| {
            let t = std::time::Instant::now();
            while !f(&e.view()) {
                assert!(t.elapsed().as_secs() < 4);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        wait(&|v| v.current.is_some());
        wait(&|v| v.current.is_none());
    }
}
