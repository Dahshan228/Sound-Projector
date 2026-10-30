#!/usr/bin/env python3
"""Download authorized YouTube videos as MP3 files using yt-dlp and FFmpeg."""

import argparse
import importlib.util
from pathlib import Path
import re
import shutil
import subprocess
import sys
from urllib.parse import parse_qs, urlsplit


def video_url(value):
    """Accept direct video links and discard playlist/tracking parameters."""
    try:
        parts = urlsplit(value.strip())
        host = parts.hostname
    except ValueError as exc:
        raise ValueError(f"Invalid URL: {value}") from exc
    if parts.scheme not in ("http", "https"):
        raise ValueError(f"Use a full https:// YouTube video URL: {value}")
    segments = parts.path.strip("/").split("/")
    video_id = ""
    if host == "youtu.be" and len(segments) == 1:
        video_id = segments[0]
    elif host in ("youtube.com", "www.youtube.com", "m.youtube.com", "music.youtube.com"):
        if parts.path == "/watch":
            video_id = parse_qs(parts.query).get("v", [""])[0]
        elif len(segments) == 2 and segments[0] in ("shorts", "embed", "live"):
            video_id = segments[1]
    if not re.fullmatch(r"[A-Za-z0-9_-]{11}", video_id):
        raise ValueError(f"Expected a direct YouTube video link (not search/playlist): {value}")
    return f"https://www.youtube.com/watch?v={video_id}"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("urls", nargs="*", help="Direct YouTube video URLs")
    parser.add_argument("-f", "--file", type=Path, help="UTF-8 file with one URL per line")
    parser.add_argument("-o", "--output", type=Path,
                        default=Path(__file__).resolve().parent.parent / "song",
                        help="Output folder (default: this project's song folder)")
    parser.add_argument("--bitrate", choices=("128", "192", "256", "320"), default="320",
                        help="MP3 bitrate in kbps (default: 320)")
    args = parser.parse_args(argv)
    entries = list(args.urls)
    if args.file:
        try:
            entries.extend(line.strip() for line in args.file.read_text(encoding="utf-8-sig").splitlines()
                           if line.strip() and not line.lstrip().startswith("#"))
        except (OSError, UnicodeError) as exc:
            parser.error(f"Cannot read URL file: {exc}")
    if not entries:
        parser.error("Provide a video URL or --file urls.txt")
    try:
        urls = list(dict.fromkeys(video_url(entry) for entry in entries))
    except ValueError as exc:
        parser.error(str(exc))

    missing = []
    if importlib.util.find_spec("yt_dlp") is None:
        missing.append('yt-dlp: install with python -m pip install -U "yt-dlp[default]"')
    for program in ("ffmpeg", "ffprobe"):
        if not shutil.which(program):
            missing.append(f"{program}: install FFmpeg and add its bin directory to PATH")
    runtime = next((name for name in ("deno", "node") if shutil.which(name)), None)
    if runtime is None:
        missing.append("JavaScript runtime: install Deno or a current Node.js release")
    if missing:
        print("Missing dependencies:\n  " + "\n  ".join(missing), file=sys.stderr)
        return 1

    try:
        output = args.output.expanduser().resolve()
        output.mkdir(parents=True, exist_ok=True)
        command = [
            sys.executable, "-m", "yt_dlp",
            "--ignore-config", "--no-playlist", "--no-overwrites",
            "--js-runtimes", runtime,
            "--format", "bestaudio/best", "--extract-audio",
            "--audio-format", "mp3", "--audio-quality", args.bitrate + "K",
            "--embed-metadata", "--windows-filenames",
            "--paths", str(output),
            "--output", "%(title).120s [%(id)s].%(ext)s",
        ]
        failed = 0
        for index, url in enumerate(urls, 1):
            print(f"\n[{index}/{len(urls)}] {url}", flush=True)
            if subprocess.run([*command, "--", url], check=False).returncode:
                failed += 1
        if failed:
            print(f"{failed} of {len(urls)} video(s) failed. See errors above.", file=sys.stderr)
            return 1
        print(f"\nMP3 files saved in: {output}")
        return 0
    except OSError as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("\nCancelled. Run again to resume incomplete downloads.", file=sys.stderr)
        return 130


if __name__ == "__main__":
    sys.exit(main())
