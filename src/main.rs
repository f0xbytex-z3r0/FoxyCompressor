use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use image::ImageReader;
use indicatif::{ProgressBar, ProgressStyle};
use sevenz_rust::{SevenZArchiveEntry, SevenZWriter};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Folder to scan. It is never modified.
    input: PathBuf,
    /// Folder in which the organized result is created.
    output: PathBuf,
    /// Image quality from 1 to 100 when writing WebP.
    #[arg(long, default_value_t = 80, value_parser = clap::value_parser!(u8).range(1..=100))]
    image_quality: u8,
    /// H.264 Constant Rate Factor. Lower values preserve more detail.
    #[arg(long, default_value_t = 28, value_parser = clap::value_parser!(u8).range(0..=51))]
    video_crf: u8,
    /// Optional video bitrate cap, such as 500k or 2M.
    #[arg(long, default_value = "500k")]
    video_bitrate: String,
    /// FFmpeg encoder speed preset.
    #[arg(long, default_value = "slow", value_parser = ["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow"])]
    video_preset: String,
    /// 7z LZMA2 compression level. Higher uses more CPU and memory.
    #[arg(long, default_value_t = 9, value_parser = clap::value_parser!(u32).range(0..=9))]
    archive_level: u32,
    /// FFmpeg executable used for videos. Defaults to assets/ffmpeg.exe or PATH.
    #[arg(long)]
    ffmpeg: Option<PathBuf>,
    /// Keep videos in their original format instead of invoking FFmpeg.
    #[arg(long)]
    copy_videos: bool,
    /// Only show the planned operations.
    #[arg(long)]
    dry_run: bool,
    /// Existing output files are replaced.
    #[arg(long)]
    overwrite: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, ValueEnum)]
enum Category {
    Images,
    Videos,
    Audio,
    Documents,
    Other,
}

impl Category {
    fn folder(self) -> &'static str {
        match self {
            Self::Images => "Images",
            Self::Videos => "Videos",
            Self::Audio => "Audio",
            Self::Documents => "Documents",
            Self::Other => "Other",
        }
    }

    fn archive_name(self) -> &'static str {
        match self {
            Self::Audio => "audio.7z",
            Self::Documents => "documents.7z",
            Self::Other => "other.7z",
            _ => unreachable!(),
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let input = fs::canonicalize(&args.input).context("could not resolve input folder")?;
    if !input.is_dir() {
        anyhow::bail!("input is not a folder: {}", input.display());
    }
    let output = fs::canonicalize(&args.output).unwrap_or(args.output.clone());
    if input == output {
        anyhow::bail!("input and output folders must be different");
    }

    let mut files: HashMap<Category, Vec<(PathBuf, PathBuf)>> = HashMap::new();
    for entry in WalkDir::new(&input).follow_links(false) {
        let entry = entry.context("could not scan input folder")?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry.path().strip_prefix(&input)?.to_path_buf();
        files
            .entry(category_for(entry.path()))
            .or_default()
            .push((entry.path().to_path_buf(), relative));
    }

    let total = files.values().map(Vec::len).sum::<usize>();
    let archive_count = [Category::Audio, Category::Documents, Category::Other]
        .iter()
        .filter(|category| {
            files
                .get(category)
                .is_some_and(|entries| !entries.is_empty())
        })
        .count();
    let work_total = files.get(&Category::Images).map_or(0, Vec::len)
        + files.get(&Category::Videos).map_or(0, Vec::len)
        + archive_count;
    println!("Found {total} files.");
    println!("@FOXY_TOTAL {work_total}");
    if args.dry_run {
        for (category, entries) in &files {
            for (_, relative) in entries {
                println!("{}: {}", category.folder(), relative.display());
            }
        }
        return Ok(());
    }
    fs::create_dir_all(&output).context("could not create output folder")?;

    let mut work_done = 0;
    for category in [Category::Images, Category::Videos] {
        for (source, relative) in files.remove(&category).unwrap_or_default() {
            process_media(&args, category, &source, &relative, &output)?;
            work_done += 1;
            println!(
                "@FOXY_PROGRESS {work_done} {work_total} {} {}",
                category.folder(),
                relative.display()
            );
        }
    }
    for category in [Category::Audio, Category::Documents, Category::Other] {
        let entries = files.remove(&category).unwrap_or_default();
        if !entries.is_empty() {
            println!("@FOXY_STAGE {} 0", category.folder());
            create_archive(
                category,
                &entries,
                &output,
                args.overwrite,
                args.archive_level,
            )?;
            work_done += 1;
            println!(
                "@FOXY_PROGRESS {work_done} {work_total} {} archive",
                category.folder()
            );
        }
    }
    println!("Finished. Output: {}", output.display());
    Ok(())
}

fn category_for(path: &Path) -> Category {
    let extension = path
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    if ["jpg", "jpeg", "png", "bmp", "tif", "tiff", "gif", "webp"].contains(&extension.as_str()) {
        Category::Images
    } else if [
        "mp4", "avi", "mpeg", "mpg", "wmv", "mov", "flv", "mkv", "webm",
    ]
    .contains(&extension.as_str())
    {
        Category::Videos
    } else if [
        "mp3", "wav", "aac", "wma", "flac", "ogg", "m4a", "mid", "midi",
    ]
    .contains(&extension.as_str())
    {
        Category::Audio
    } else if [
        "txt", "text", "rtf", "doc", "docx", "docm", "pdf", "odt", "xls", "xlsx", "csv", "ods",
        "ppt", "pptx", "odp", "html", "htm", "css", "js", "py",
    ]
    .contains(&extension.as_str())
    {
        Category::Documents
    } else {
        Category::Other
    }
}

fn destination(
    output: &Path,
    category: Category,
    relative: &Path,
    extension: Option<&str>,
) -> PathBuf {
    let mut path = output.join(category.folder()).join(relative);
    if let Some(extension) = extension {
        path.set_extension(extension);
    }
    path
}

fn process_media(
    args: &Args,
    category: Category,
    source: &Path,
    relative: &Path,
    output: &Path,
) -> Result<()> {
    let is_image = category == Category::Images;
    let destination = destination(
        output,
        category,
        relative,
        if is_image { Some("webp") } else { Some("mp4") },
    );
    fs::create_dir_all(destination.parent().unwrap())?;
    if destination.exists() && !args.overwrite {
        anyhow::bail!(
            "destination exists (use --overwrite): {}",
            destination.display()
        );
    }
    if is_image
        && source
            .extension()
            .and_then(OsStr::to_str)
            .map(|e| e.eq_ignore_ascii_case("svg"))
            .unwrap_or(false)
    {
        let destination = destination.with_extension("svg");
        fs::copy(source, destination)?;
    } else if is_image {
        let image = ImageReader::open(source)
            .with_context(|| format!("could not read {}", source.display()))?
            .decode()?;
        let encoded = webp::Encoder::from_image(&image)
            .map_err(|error| anyhow::anyhow!("could not initialize WebP encoder: {error}"))?
            .encode(args.image_quality as f32);
        fs::write(&destination, &*encoded)?;
    } else if !args.copy_videos {
        let ffmpeg = find_ffmpeg(args);
        let converted = if let Some(ffmpeg) = ffmpeg {
            match Command::new(&ffmpeg)
                .args([
                    "-y",
                    "-hide_banner",
                    "-loglevel",
                    "info",
                    "-stats_period",
                    "1",
                    "-i",
                ])
                .arg(source)
                .args([
                    "-c:v",
                    "libx264",
                    "-b:v",
                    &args.video_bitrate,
                    "-crf",
                    &args.video_crf.to_string(),
                    "-preset",
                    &args.video_preset,
                    "-movflags",
                    "+faststart",
                    "-vf",
                    "scale=iw:-1",
                    "-map_metadata",
                    "0",
                ])
                .arg(&destination)
                .stderr(Stdio::piped())
                .stdout(Stdio::null())
                .spawn()
            {
                Ok(mut child) => match child.stderr.take() {
                    Some(stderr) => {
                        show_ffmpeg_progress(BufReader::new(stderr));
                        match child.wait() {
                            Ok(status) if status.success() => true,
                            Ok(_) => {
                                eprintln!(
                                    "Warning: FFmpeg failed for {}; copying original",
                                    source.display()
                                );
                                false
                            }
                            Err(error) => {
                                eprintln!(
                                    "Warning: could not finish FFmpeg ({error}); copying original"
                                );
                                false
                            }
                        }
                    }
                    None => {
                        eprintln!("Warning: FFmpeg produced no progress stream; copying original");
                        false
                    }
                },
                Err(error) => {
                    eprintln!("Warning: could not run FFmpeg ({error}); copying original");
                    false
                }
            }
        } else {
            eprintln!("Warning: FFmpeg not found; copying videos unchanged");
            false
        };
        if !converted {
            if destination.exists() {
                fs::remove_file(&destination)?;
            }
            copy_video(source, relative, output, args.overwrite)?;
        }
    } else {
        let destination = destination.with_extension(source.extension().unwrap_or_default());
        fs::copy(source, destination)?;
    }
    println!("Processed {}", relative.display());
    Ok(())
}

fn show_ffmpeg_progress<R: BufRead>(mut reader: R) {
    let progress = ProgressBar::new_spinner();
    progress.set_style(ProgressStyle::with_template("{spinner} {bar:40.cyan/blue} {msg}").unwrap());
    progress.enable_steady_tick(std::time::Duration::from_millis(120));
    let mut duration = None;
    let mut buffer = Vec::new();
    while reader.read_until(b'\r', &mut buffer).unwrap_or(0) > 0 {
        let text = String::from_utf8_lossy(&buffer);
        if let Some(value) = text
            .split("Duration:")
            .nth(1)
            .and_then(|value| value.split(',').next())
        {
            duration = parse_timestamp(value.trim());
        }
        if let Some(value) = text
            .split("time=")
            .nth(1)
            .and_then(|value| value.split_whitespace().next())
        {
            if let Some(position) = parse_timestamp(value) {
                if let Some(total) = duration {
                    progress.set_length(total as u64);
                    progress.set_position(position as u64);
                    progress.set_message(format!("Video: {:.0}%", position / total * 100.0));
                    println!("@FOXY_VIDEO {:.4}", (position / total).clamp(0.0, 1.0));
                } else {
                    progress.set_message(format!("Video: {:.0}s", position));
                }
            }
        }
        buffer.clear();
    }
    progress.finish_and_clear();
}

fn parse_timestamp(value: &str) -> Option<f64> {
    let mut parts = value.trim().split(':');
    let hours = parts.next()?.parse::<f64>().ok()?;
    let minutes = parts.next()?.parse::<f64>().ok()?;
    let seconds = parts.next()?.parse::<f64>().ok()?;
    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

fn find_ffmpeg(args: &Args) -> Option<PathBuf> {
    if let Some(path) = &args.ffmpeg {
        return path.is_file().then(|| path.clone());
    }
    let executable_assets = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("assets")));
    let project_assets = std::env::current_dir().ok().map(|path| path.join("assets"));
    let names = if cfg!(windows) {
        ["ffmpeg.exe", "ffmpeg"]
    } else {
        ["ffmpeg", "ffmpeg.exe"]
    };
    for directory in [executable_assets, project_assets].into_iter().flatten() {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    Some(PathBuf::from("ffmpeg"))
}

fn copy_video(source: &Path, relative: &Path, output: &Path, overwrite: bool) -> Result<()> {
    let destination = destination(
        output,
        Category::Videos,
        relative,
        source.extension().and_then(OsStr::to_str),
    );
    if destination.exists() && !overwrite {
        anyhow::bail!(
            "destination exists (use --overwrite): {}",
            destination.display()
        );
    }
    fs::create_dir_all(destination.parent().unwrap())?;
    fs::copy(source, destination)?;
    Ok(())
}

fn create_archive(
    category: Category,
    entries: &[(PathBuf, PathBuf)],
    output: &Path,
    overwrite: bool,
    archive_level: u32,
) -> Result<()> {
    let archive_path = output.join(category.archive_name());
    if archive_path.exists() && !overwrite {
        anyhow::bail!(
            "destination exists (use --overwrite): {}",
            archive_path.display()
        );
    }
    let mut writer = SevenZWriter::create(&archive_path)?;
    writer.set_content_methods(vec![
        sevenz_rust::lzma::LZMA2Options::with_preset(archive_level).into(),
    ]);
    for (source, relative) in entries {
        let archive_name = relative.to_string_lossy().replace('\\', "/");
        let entry = SevenZArchiveEntry::from_path(source, archive_name);
        writer.push_archive_entry(entry, Some(File::open(source)?))?;
    }
    writer.finish()?;
    println!("Created {}", archive_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_extensions_case_insensitively() {
        assert_eq!(category_for(Path::new("photo.JpG")), Category::Images);
        assert_eq!(category_for(Path::new("movie.MKV")), Category::Videos);
        assert_eq!(category_for(Path::new("notes.PDF")), Category::Documents);
        assert_eq!(category_for(Path::new("sound.FLAC")), Category::Audio);
        assert_eq!(category_for(Path::new("binary.bin")), Category::Other);
    }

    #[test]
    fn destination_keeps_nested_relative_path() {
        let result = destination(
            Path::new("out"),
            Category::Images,
            Path::new("nested/photo.jpg"),
            Some("webp"),
        );
        assert_eq!(result, PathBuf::from("out/Images/nested/photo.webp"));
    }

    #[test]
    fn archive_names_are_stable() {
        assert_eq!(Category::Audio.archive_name(), "audio.7z");
        assert_eq!(Category::Documents.archive_name(), "documents.7z");
        assert_eq!(Category::Other.archive_name(), "other.7z");
    }
}
