# Sound Projector

A lightweight Windows soundboard for Discord calls and in-game voice chat. Load your songs and sounds, trim them, boost them, and mark sections of a song without touching the original file. Queue them up with global hotkeys while you play. Your friends hear them through your mic, mixed with your voice, and you hear them in your headphones, each with its own volume slider.

## Setup guide

1. Install VB-Cable, then reboot.
2. In Discord, go to **Voice & Video** and set **Input Device** to **CABLE Output**.
3. For the cleanest sound, turn off Discord's **Noise Suppression** (Krisp), **Echo Cancellation**, and **Automatic Gain Control**. If they stay on, Discord's processing squashes music no matter how clean the app's output is.
4. Use **Voice Activity** mode. With **Push to Talk**, friends only hear songs while the key is held.
5. A server admin can raise the voice channel's **bitrate** (Edit Channel), which makes a large difference for music.
6. Do the same for in-game voice chat, using **CABLE Output** as the mic.

Get VB-Cable from <https://vb-audio.com/Cable/>.

## Using it

- **Add sounds:** drop MP3, WAV, OGG or FLAC files on the window, or click **+ Add**.
- **Play:** click a pad's ▶ or press its hotkey to add it to the queue. One item plays at a time.
- **Preview and snippets:** click a pad to open its preview. Click anywhere on the waveform to hear it from there (only you hear it), press Space to pause, and drag across it to mark a snippet. **Play for friends** queues the selected snippet.
- **Pad options:** right-click a pad to **Rename**, **Set hotkey…** or **Delete** it.
- **Editor:**
  - drag across the waveform to mark a section (it becomes its own pad)
  - drag a section's edges to trim it
  - set **Boost**
  - **Preview** plays it to you only
- **Hotkeys:** pick keys your game doesn't use, like the numpad or F13–F24. Games may also see the key.
- **Tray:** closing the window keeps the app running in the tray, so hotkeys still work. Quit from the tray menu.
- **Settings (⚙):**
  - pick devices
  - set Pause/Resume, Skip and Stop hotkeys
  - turn on **Start with Windows**

Your library lives in `%APPDATA%\SoundProjector\sounds\` (imported files are copies; originals are never touched), and everything else in `%APPDATA%\SoundProjector\settings.json`.

## Building

1. Install Rust with the GNU toolchain (no Visual Studio needed):
   ```powershell
   curl.exe -sSfLo "$env:TEMP\rustup-init.exe" https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe
   & "$env:TEMP\rustup-init.exe" -y --default-host x86_64-pc-windows-gnu --profile minimal -c clippy
   ```
2. Install a MinGW toolchain, because some Windows crates need `dlltool` and `as`. Download the WinLibs zip (UCRT, POSIX threads) from <https://winlibs.com>, extract it to `%LOCALAPPDATA%\winlibs`, and put `%LOCALAPPDATA%\winlibs\mingw64\bin` on your `PATH`.
3. Build:
   ```powershell
   cargo build --release
   ```
   The app is `target\release\sound-projector.exe`.

Run the tests with `cargo test`. Two hardware tests are opt-in: `cargo test -- --ignored`. One plays a silent tone through your default output; the other adds and then removes the Start with Windows registry entry.
