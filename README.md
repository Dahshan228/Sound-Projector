# Sound Projector

## [⬇ Download for Windows](https://github.com/Dahshan228/Sound-Projector/releases/latest/download/SoundProjectorSetup.exe)

A lightweight Windows soundboard for Discord calls and in-game voice chat. Load your songs and sounds, trim them, boost them, and mark sections of a song without touching the original file. Queue them up with global hotkeys while you play. Your friends hear them through your mic, mixed with your voice, and you hear them in your headphones, each with its own volume slider.

## Download

1. Download **SoundProjectorSetup.exe** from the [latest release](https://github.com/Dahshan228/Sound-Projector/releases/latest) and run it.
2. If Windows says "Windows protected your PC", click **More info**, then **Run anyway**. The installer isn't code-signed, so Windows doesn't recognise it yet.
3. Click **Install**. If you don't have VB-Cable, Setup installs it too. Click **Yes** when Windows asks to allow it, click **Install** if Windows asks about the driver, then restart when Setup asks.
4. Sound Projector opens with the setup guide below, after the restart if there was one. The Discord steps are the only part left to do by hand.

VB-CABLE is a free virtual audio cable by [VB-Audio Software](https://vb-audio.com/Cable/), bundled under their donationware terms. If you find it useful, consider donating to them.

## Setup guide

1. Install VB-Cable, then reboot. The installer above does this for you.
2. In Discord, go to **Voice & Video** and set **Input Device** to **CABLE Output**.
3. For the cleanest sound, turn off Discord's **Noise Suppression** (Krisp), **Echo Cancellation**, and **Automatic Gain Control**. If they stay on, Discord's processing squashes music no matter how clean the app's output is.
4. Use **Voice Activity** mode. With **Push to Talk**, friends only hear songs while the key is held.
5. A server admin can raise the voice channel's **bitrate** (Edit Channel), which makes a large difference for music.
6. Do the same for in-game voice chat, using **CABLE Output** as the mic.

Get VB-Cable from <https://vb-audio.com/Cable/>.

## Using it

- **Add sounds:** drop MP3, WAV, OGG or FLAC files on the window, or click **+ Add**.
- **Add from YouTube:** click **+ YouTube** and paste a link to a video you own or have permission to use. The first time, the app downloads the tools it needs (about 160 MB). Audio is saved as lossless FLAC from YouTube's best stream, so it never goes through a second round of lossy compression. Songs you already added as MP3s stay at their old quality until you add them again.
- **Play:** click a pad's ▶ or press its hotkey to add it to the queue. One item plays at a time.
- **Preview and snippets:** click a pad to open its preview. Click anywhere on the waveform to hear it from there (only you hear it), press Space to pause, and drag across it to mark a snippet. **Play for friends** queues the selected snippet.
- **Pad options:** right-click a pad to **Rename**, **Set hotkey…** or **Delete** it.
- **Editor:**
  - drag across the waveform to mark a section (it becomes its own pad)
  - drag a section's edges to trim it
  - set **Boost**
  - **Preview** plays it to you only
- **Hotkeys:** pick keys your game doesn't use, like the numpad or F13–F24. Games may also see the key.
- **Updates:** when a new version is out, click **Update now** in the banner. The app closes, updates and reopens by itself, and your songs and settings stay.
- **Tray:** closing the window keeps the app running in the tray, so hotkeys still work. Quit from the tray menu.
- **Settings (⚙):**
  - pick devices
  - set Pause/Resume, Skip and Stop hotkeys
  - turn on **Start with Windows**

Your library lives in `%APPDATA%\SoundProjector\sounds\` (imported files are copies; originals are never touched), and everything else in `%APPDATA%\SoundProjector\settings.json`.

## YouTube to MP3 script

Use `scripts/yt_to_mp3.py` for videos you own or have permission to download.
It accepts direct video links, including Shorts and `youtu.be` links. Search
results and playlist URLs are not accepted.

Install Python 3.10+ and the downloader:

```powershell
python -m pip install -U "yt-dlp[default]"
```

Install [FFmpeg](https://ffmpeg.org/download.html) and add its `bin` directory
(containing both `ffmpeg.exe` and `ffprobe.exe`) to `PATH`. Install
[Deno](https://docs.deno.com/runtime/getting_started/installation/) or a current
[Node.js](https://nodejs.org/en/download) release, then reopen your terminal.
These dependencies follow the [yt-dlp setup instructions](https://github.com/yt-dlp/yt-dlp#dependencies).

Run from the project directory, replacing `VIDEO_ID` with your video's ID:

```powershell
python scripts/yt_to_mp3.py "https://www.youtube.com/watch?v=VIDEO_ID"
```

For several videos, pass multiple quoted URLs or create `urls.txt` with one
direct video URL per line (blank lines and lines starting with `#` are ignored):

```powershell
python scripts/yt_to_mp3.py --file urls.txt --bitrate 320
```

MP3s go into this project's `song/` folder by default, even when the script runs
from another directory, with titles and video IDs in their filenames.
Use `--output` to choose a different folder. Direct YouTube song links are
saved in [song/links.md](song/links.md) as a reference list, not a download batch.
Existing files are preserved. Failed videos are reported while the remaining
links continue; any failure produces a nonzero exit code. Higher bitrates do not
improve the original source's audio quality. Drag the resulting MP3s into Sound
Projector to import them. If YouTube extraction stops working, update yt-dlp with
the install command above.

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
4. To build the installer, install Inno Setup (`winget install JRSoftware.InnoSetup`), then run:
   ```powershell
   powershell -ExecutionPolicy Bypass -File installer\build.ps1
   ```
   It downloads VB-CABLE into `target\vbcable` the first time, and writes `target\installer\SoundProjectorSetup.exe`. Upload that file to a new GitHub release.

Run the tests with `cargo test`. Two hardware tests are opt-in: `cargo test -- --ignored`. One plays a silent tone through your default output; the other adds and then removes the Start with Windows registry entry.
