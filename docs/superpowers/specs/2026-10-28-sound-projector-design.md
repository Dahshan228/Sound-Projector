# Sound Projector: design

## Intent

Sound Projector is a lightweight Windows app for playing songs, and some short sounds, into Discord calls and in-game voice chat while gaming with friends.

- **Library:** you import songs and sounds, trim them, boost them, and mark sections of a song.
- **Queue:** you queue songs and sounds up with hotkeys or clicks, and they play one at a time.
- **Two volumes:** friends hear the audio through your mic, mixed with your voice, and you hear it in your headphones. Each path has its own volume slider.

**Why it exists:** the user was using Voicemod and found it "too heavy and laggy and sounds like shit." This app is meant to be the opposite on every count:

- **Light:** it opens instantly and uses so little CPU and memory that it never affects a game.
- **Instant:** playback starts the moment you press a hotkey.
- **Crisp:** friends hear no crackle, distortion, delay, or squished audio.
- **Clean:** it looks and feels polished.

**Main use:** the user said they are "mainly going for songs." Short sound effects are secondary.

**What success looks like:** you queue songs mid-game with a hotkey, they play clearly for everyone with no lag, and you forget the app is running.

## Goals

- Import songs and sound files. The expected library is 20–50 items.
- Play full songs from start to finish, as well as marked sections of songs.
- Edit without changing the original file: trim the start and end, and boost the volume.
- Mark several named sections in one song. Each section acts as its own pad.
- Play one item at a time from a queue. You can pause, skip, stop, seek, reorder, and remove.
- Send the audio into a virtual mic, mixed with your real voice, so friends hear it on Discord or in in-game voice chat.
- Give the user two independent volume sliders: **Friends hear** and **You hear**.
- Support global hotkeys that work while a game has focus.
- Keep CPU and memory use very low, both at idle and during playback.

## Non-goals

- A voice changer or voice effects. Voicemod's main feature is out of scope.
- Layering, meaning several sounds playing at once. The user said they probably wouldn't use it. If it's wanted later, it can be added as a setting.
- Recording sounds inside the app.
- Sound-sharing or cloud libraries.
- macOS or Linux.
- Categories, pages, or folders of sounds.
- Saving the queue between app restarts. The queue starts empty on launch (Claude's call).

## Anti-goals

The app is a failure, even if it technically works, if any of these happens:

- It feels heavy: slow to start, or it uses enough CPU or GPU to affect a game.
- There's audible lag between pressing a hotkey and playback starting, when nothing is already playing.
- Audio crackles or clips, or gets squashed when boosted.
- Editing a file degrades its quality. Audio is never re-encoded.
- It looks like a debug tool or a programmer's prototype.

## Constraints

- **Platform:** Windows 11, which is the user's machine.
- **Language:** Rust. This was Claude's call; the user asked for "whatever's fastest." Rust gives a single small `.exe` with no runtime, and its GPU-drawn UI sits near 0% CPU when idle.
- **Toolchain:** the machine currently has only Python and Node, so building needs a one-time Rust toolchain install. The shipped app is just the `.exe`.
- **Virtual mic:** the app depends on **VB-Cable**, a free and widely used virtual audio cable driver.
  - The user installs it once.
  - In Discord or the game, they select **CABLE Output** as the input device.
  - The app writes to **CABLE Input**.
- **No admin rights:** the app must work without running as administrator.

## Data model

There is one concept for everything (Claude's call):

- **Source:** a file in the library.
- **Pad:** a region of a source, with these fields:
  - start
  - end
  - name
  - boost
  - hotkey

When you import a file, it gets one pad that covers the whole file. A short sound is a source with one trimmed pad. A song is a source with a "whole song" pad, plus one pad for each section you mark. You can delete the whole-song pad if you only want its sections.

## How it behaves

The user picked layout "A" in both mockup rounds.

### Main window

- **Look:** dark theme.
  - The background is near-black (about `#111216`).
  - Pads are slightly lighter (about `#1c1d23`) with subtle borders.
  - There's one purple accent (about `#6450f0` to `#7c6cff`), plus green for the "You hear" slider.
  - Text meets WCAG AA contrast.
- **Top bar:** the app name, the **Friends hear** slider, and the **You hear** slider.
- **Pad grid:** fills the main area.
  - It has as many columns as fit the width, about 3 at the default size, and it scrolls when the pads don't all fit.
  - Each pad shows its name, its length (or "Song clip" for a section), and a hotkey badge.
  - The pad that's playing lights up with an accent border and a tinted fill. Queued pads show their position, such as "#1 in queue".
- **Queue panel:** on the right, titled **Up next**.
  - It lists queued items in order.
  - Drag a handle to reorder an item, or click × to remove it.
  - It's empty and quiet when nothing is queued.
- **Now-playing bar:** along the bottom.
  - It shows the current item's name.
  - It has **Pause/Resume**, **Skip**, and **Stop** buttons.
  - It has a progress bar with the elapsed and total time. You can drag the bar to seek.
- **Adding sounds:** drag audio files onto the window to add them. An "+ Add" pad opens a file picker.
- **Pad actions:** right-click a pad to:
  - **Edit**, which opens the editor for its source and shows all of that source's pads/sections
  - **Rename**
  - **Set hotkey**
  - **Delete**
- **Window state:** the window remembers its size and position.

### Queue rules

- Clicking a pad, or pressing its hotkey, adds it to the end of the queue. If nothing is playing, it starts immediately.
- The same pad can be queued more than once.
- When an item ends, the next one starts with no gap.
- **Skip:** ends the current item and starts the next.
- **Stop:** ends the current item and clears the queue.
- **Pause:** pauses the current item. Your mic keeps passing through.
- **Fades:** stop, skip, and pause fade over a few milliseconds so there's no click.
- **Global hotkeys:** Pause/Resume, Skip, and Stop each have one. They're unassigned by default; the user sets them.

### Editor

- **Waveform:** the editor shows the source's waveform, with each of its pads drawn as a colored region.
- **Regions:** drag a region's edges to trim it. Drag across empty waveform to create a new named section, which becomes a new pad. Sections can be adjusted or deleted.
- **Boost:** set per pad in dB. The default is 0 dB and the range is roughly 0 to +18 dB; the builder may tune the range. Boost goes through a limiter, so it gets louder without clipping or crackling.
- **Preview:** plays the selected region to **you only**. It's independent of the queue and doesn't pause it (Claude's call).
- **Originals untouched:** the original file is never modified. Regions, boost, names, and hotkeys are stored as settings.

### Background behavior

- Closing the window sends the app to the system tray, and hotkeys keep working. Quit from the tray menu.
- The app doesn't start with Windows by default. A setting turns that on.

### Settings

This is a small panel that's rarely touched. It holds:

- **Device pickers:**
  - **Mic:** your real microphone. Defaults to the Windows default recording device.
  - **Friends output:** defaults to **CABLE Input** if it's present.
  - **You hear output:** defaults to the Windows default playback device.
- **Start with Windows**, off by default.
- **Pause, Skip, and Stop hotkeys.**
- **A short setup guide** (see below).

## Audio routing

```
real mic ─────────────────────────────────┐
                                          ├─► mix ─► limiter ─► CABLE Input ─► (Discord/game reads CABLE Output)
current item ─► boost+limit ─► × Friends hear ┘
             └──────────────► × You hear ─► headphones
editor preview ─────────────► × You hear ─► headphones
```

- **Mic passthrough:** your mic goes into the friends mix at unity gain. It is **not** sent to your headphones, because hearing yourself is distracting.
- **Signal path:** everything is processed as 32-bit float and resampled to each output device's rate with a high-quality resampler. Nothing is re-encoded.
- **Latency target:** under about 30 ms from a hotkey press to audio at CABLE Input and the headphones, when nothing is already playing. Mic passthrough adds as little delay as possible.
- **Clock drift:** the real mic and the VB-Cable device run on separate clocks. Mic passthrough must handle that drift without audible glitches over hours of use.

### Memory and streaming

A decoded 4-minute song takes about 90 MB, so songs are not held fully in memory. This approach is Claude's call; the exact mechanism is left to the builder.

- **Short pads:** pads under roughly 10 seconds are decoded into memory at startup.
- **Long pads:** full songs and long sections are streamed from disk during playback. Their first moment is pre-decoded so they still start instantly.
- **Audio callback:** the real-time audio callback must never block or allocate. Streaming happens on another thread.
- **Editor:** the full waveform is computed only while a source is open in the editor.

## Library storage

These are Claude's calls:

- **Import copies the file** into the app's library folder (`%APPDATA%\SoundProjector\sounds\`). Moving or deleting the original then doesn't break anything.
- **One human-readable settings file** lives next to the library. It holds sources, pads, regions, boost, hotkeys, slider levels, devices, and window position.
- **Supported formats** are MP3, WAV, OGG (Vorbis), and FLAC.

## Hotkeys

- **System-wide:** hotkeys use Windows' standard global hotkey registration (`RegisterHotKey`), so they work while a fullscreen, borderless, or elevated game has focus. The app doesn't need admin rights, so drag and drop and start-with-Windows keep working simply.
- **Key choice:** the game may also see a bound key, so the user picks keys the game doesn't use, such as the numpad or F13 and up. This was Claude's call after builder review. The in-app hotkey picker should say so briefly.
- **Assigning keys:** any key or modifier combo can be assigned per pad. Click the hotkey field and press the key.
- **Conflicts:** duplicate assignments are refused with a clear message. If Windows refuses a key because another app already owns it, the app says that too.

## Setup guide (shipped in the app and in the README)

1. Install VB-Cable, then reboot.
2. In Discord, go to **Voice & Video** and set **Input Device** to **CABLE Output**.
3. For the cleanest sound, turn off Discord's **Noise Suppression** (Krisp), **Echo Cancellation**, and **Automatic Gain Control**. If they stay on, Discord's processing squashes music no matter how clean the app's output is.
4. Use **Voice Activity** mode. With **Push to Talk**, friends only hear songs while the key is held.
5. A server admin can raise the voice channel's **bitrate** (Edit Channel), which makes a large difference for music.
6. Do the same for in-game voice chat, using **CABLE Output** as the mic.

## Verification

- **One small test file** covers the logic that's easy to get wrong:
  - region boundaries map to the right samples
  - boost plus the limiter never exceeds full scale
  - queue order holds across enqueue, skip, remove, reorder, and stop
- **A manual check in a real Discord call:**
  - songs arrive crisp
  - nothing lags
  - your voice still comes through
  - the two sliders work independently
  - hotkeys work with a fullscreen game focused
  - a full song streams start to finish without dropouts
- **A resource check:** idle CPU is near 0%, and memory stays reasonable with about 50 pads, including several full songs.

## Left to the builder

- **Crate and library choices** for:
  - the UI
  - audio I/O
  - decoding
  - resampling
  - the limiter
  - global hotkeys
  - the tray
- **Audio internals:**
  - how mic-to-cable clock drift is handled
  - the streaming and pre-decode mechanism
  - the threading model
- **Settings file format.**
- **Visual details:** exact spacing, fonts, the waveform rendering style, and how the drag-to-reorder interaction looks, all within the look described above.
- **Buffer sizes:** the lowest that stays glitch-free.
