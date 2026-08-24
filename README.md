
# FoxyCompressor

> Hello, FoxyCompressor v3.0 beta. This version is a complete rewrite in Rust,
> focused on lower overhead, faster startup, safer file handling, and a more
> reliable foundation for future releases. Depending on the workload and
> hardware, it can provide up to 2x faster processing than the previous version.
> Actual results vary by file type, compression settings, and hardware.

<div align="center">
    <img src="https://github.com/user-attachments/assets/453b0588-e758-46c9-854b-097e497bb30d" alt="FoxyCompressor Logo">
</div>

FoxyCompressor is a Rust application for processing folders in batches. It
classifies files, preserves their relative folder structure, and writes
organized compressed output without modifying the input folder.

I originally created the software because I had **1.8 Terabytes of data** unsorted, huge,
and needed something to make the most of my space. If you are anything like me, you will
benefit hugely from using FoxyCompressor.

## Features

- Converts supported images to WebP, controllable via quality settings.
- Converts supported videos to H.264 MP4 using FFmpeg controllable with quality settings.
- Stores audio, documents, and other files in `.7z` archives.
- Organizes output into `Images`, `Videos`, `Audio`, `Documents`, and `Other`.
- Preserves relative paths inside category folders and archives.
- Includes both a command-line interface and a desktop GUI.
- Supports dry runs, overwrite control, compression settings, and video-copy mode.

## Installation

### Windows users

1. Open the project's [Releases](../../releases) page.
2. Download `foxycompressor.zip` from the latest release.
3. Unpack the ZIP file into a folder.
4. Run `foxycompressor-gui.exe`.

Keep the files together after unpacking. The ZIP should contain both program
executables and the `assets` folder, including the bundled FFmpeg executable.

### Linux users (untested)

Linux support is currently **untested**. These instructions are provided as a
best-effort guide and may require adjustments for your distribution.

1. Install Rust and Cargo using [rustup.rs](https://rustup.rs/).
2. Install FFmpeg. On Debian or Ubuntu:

```bash
sudo apt update
sudo apt install ffmpeg
```

3. Clone or download this repository.
4. Open a terminal in the repository directory.
5. Build the CLI and GUI:

```bash
cargo build --release --bins
```

6. Run the GUI on a desktop system:

```bash
./target/release/foxycompressor-gui
```

7. Alternatively, run the CLI:

```bash
./target/release/foxycompressor <input-folder> <output-folder>
```

## GUI Usage

1. Choose the source folder. The source folder is never modified.
2. Choose a destination folder for the processed output.
3. Select image and video compression profiles, or open the advanced controls.
4. Click **Start compression**.

The GUI launches the CLI beside it, so `foxycompressor.exe` must remain beside
`foxycompressor-gui.exe` on Windows. For video conversion, `ffmpeg.exe` should
be in an `assets` folder beside the CLI executable. On Linux, install `ffmpeg`
through the system package manager or place the executable in the equivalent
`assets` folder.

## CLI Usage

Run the CLI with an input folder and a different output folder:

```bash
cargo run --release -- <input-folder> <output-folder>
```

The input folder is left untouched. Images are converted to lossy WebP at
quality 80 by default. Videos are converted to H.264 MP4 with FFmpeg using CRF
28 and the `slow` preset by default. Audio, documents, and other files are
stored in `.7z` archives using LZMA2. Already-compressed formats such as MP3
and AAC may not become smaller.

### CLI options

```text
--dry-run                 Show the classification plan without writing files
--image-quality <1-100>   WebP quality (default: 80)
--video-crf <0-51>        H.264 visual quality; lower preserves more detail
--video-bitrate <value>   Video bitrate cap, for example 500k or 2M
--video-preset <value>    FFmpeg speed/efficiency preset (default: slow)
--archive-level <0-9>     7z LZMA2 strength; higher is smaller but slower
--ffmpeg <path>           Path to a specific FFmpeg executable
--copy-videos             Keep videos unchanged instead of converting them
--overwrite               Replace existing output files
```

If FFmpeg is unavailable, video files are copied unchanged and a warning is
shown.

## Building

Build both binaries in release mode:

```bash
cargo build --release --bins
```

The resulting files are:

```text
target/release/foxycompressor
target/release/foxycompressor-gui
```

## Roadmap

FoxyCompressor is still in beta. Planned work includes improved compression
controls, additional file formats, UI improvements, and a future companion
organization tool.

## Disclaimer

This is beta software. Back up important files before processing them and
verify the output before deleting any originals. Linux support is currently
untested.

## Contributing

Bug reports, testing feedback, and code contributions are welcome.

## License

FoxyCompressor is licensed under the AGPL-3.0 license.
```
