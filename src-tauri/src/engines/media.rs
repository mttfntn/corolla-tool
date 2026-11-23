//! Audio, video and GIF, through FFmpeg.

use super::{hide_console, require_output, run_tool_watching, Quality, Run, EXE};
use crate::tools::Op;
use std::process::Command;

/// Encoder arguments for an audio target.
fn audio_args(target: &str, quality: Quality) -> Vec<String> {
    let pick = |high: &str, balanced: &str, small: &str| {
        match quality {
            Quality::High => high,
            Quality::Balanced => balanced,
            Quality::Small => small,
        }
        .to_string()
    };
    let list: Vec<String> = match target {
        "mp3" => vec!["-c:a".into(), "libmp3lame".into(), "-q:a".into(), pick("0", "2", "6")],
        "m4a" => vec!["-c:a".into(), "aac".into(), "-b:a".into(), pick("256k", "192k", "96k")],
        "wav" => vec!["-c:a".into(), "pcm_s16le".into()],
        "flac" => vec!["-c:a".into(), "flac".into()],
        "ogg" => vec!["-c:a".into(), "libvorbis".into(), "-q:a".into(), pick("8", "5", "2")],
        "opus" => vec!["-c:a".into(), "libopus".into(), "-b:a".into(), pick("192k", "128k", "64k")],
        "aiff" => vec!["-c:a".into(), "pcm_s16be".into()],
        "wma" => vec!["-c:a".into(), "wmav2".into(), "-b:a".into(), pick("256k", "192k", "96k")],
        _ => Vec::new(),
    };
    list
}

/// How hard to compress, in each encoder's own scale.
#[derive(Clone, Copy, Debug, PartialEq)]
struct VideoLevel {
    x264_crf: u8,
    vp9_crf: u8,
    /// MPEG-4 and WMV quantiser, 2 best to 31 worst.
    quantiser: u8,
    /// Audio bitrate in kb/s.
    audio: u16,
    /// Limits the longest side, keeping the picture's shape.
    max_edge: Option<u32>,
}

impl VideoLevel {
    fn from_quality(quality: Quality) -> Self {
        let (x264_crf, vp9_crf, quantiser, audio) = match quality {
            Quality::High => (18, 24, 2, 256),
            Quality::Balanced => (22, 32, 4, 192),
            Quality::Small => (28, 40, 8, 128),
        };
        Self { x264_crf, vp9_crf, quantiser, audio, max_edge: None }
    }

    /// A preset's 1 to 100 quality. 70 lands near the usual "good but much smaller".
    fn from_percent(quality: u8, max_edge: Option<u32>) -> Self {
        let lost = 100.0 - f32::from(quality.clamp(1, 100));
        Self {
            x264_crf: (16.0 + lost * 0.34) as u8,
            vp9_crf: (22.0 + lost * 0.4) as u8,
            quantiser: (2.0 + lost * 0.2) as u8,
            audio: if quality >= 80 { 192 } else if quality >= 50 { 128 } else { 96 },
            max_edge,
        }
    }
}

/// Encoder arguments for a video target, picture and sound.
fn video_args(target: &str, level: VideoLevel) -> Vec<String> {
    // Common players need even dimensions and 4:2:0 colour.
    let picture = match level.max_edge {
        Some(edge) => format!(
            "scale='if(gt(iw,ih),trunc(min({edge},iw)/2)*2,-2)':'if(gt(iw,ih),-2,trunc(min({edge},ih)/2)*2)',format=yuv420p"
        ),
        None => "scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p".to_string(),
    };
    let audio = format!("{}k", level.audio);
    let x264 = level.x264_crf.to_string();
    let vp9 = level.vp9_crf.to_string();
    let quantiser = level.quantiser.to_string();
    let parts: Vec<&str> = match target {
        "mp4" | "mov" | "mkv" => vec![
            "-vf", &picture, "-c:v", "libx264", "-preset", "medium",
            "-crf", &x264, "-c:a", "aac", "-b:a", &audio,
        ],
        "webm" => vec![
            "-vf", &picture, "-c:v", "libvpx-vp9", "-b:v", "0", "-row-mt", "1",
            "-crf", &vp9, "-c:a", "libopus", "-b:a", &audio,
        ],
        "avi" => vec![
            "-vf", &picture, "-c:v", "mpeg4", "-q:v", &quantiser,
            "-c:a", "libmp3lame", "-b:a", &audio,
        ],
        "wmv" => vec![
            "-vf", &picture, "-c:v", "wmv2", "-q:v", &quantiser,
            "-c:a", "wmav2", "-b:a", &audio,
        ],
        _ => Vec::new(),
    };
    let mut args: Vec<String> = parts.into_iter().map(String::from).collect();
    if target == "mp4" || target == "mov" {
        args.extend(["-movflags".to_string(), "+faststart".to_string()]);
    }
    args
}

/// A per-clip palette keeps GIF colours clean.
fn gif_args(quality: Quality, max_edge: Option<u32>) -> Vec<String> {
    let (fps, width) = match quality {
        Quality::High => (20, 960),
        Quality::Balanced => (15, 720),
        Quality::Small => (10, 480),
    };
    let width = max_edge.map_or(width, |edge| edge.min(width));
    let filter = format!(
        "fps={fps},scale='min({width},iw)':-2:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=sierra2_4a"
    );
    vec!["-filter_complex".into(), filter, "-an".into()]
}

pub fn args_for(target: &str, quality: Quality) -> Option<Vec<String>> {
    match target {
        "mp3" | "m4a" | "wav" | "flac" | "ogg" | "opus" | "aiff" | "wma" => {
            let mut args = vec!["-vn".to_string()];
            args.extend(audio_args(target, quality));
            Some(args)
        }
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "wmv" => Some(video_args(target, VideoLevel::from_quality(quality))),
        "gif" => Some(gif_args(quality, None)),
        _ => None,
    }
}

/// The clip's length in seconds, for showing a percentage. Not knowing it only
/// means the percentage moves file by file.
fn duration(run: &Run) -> Option<f64> {
    let probe = run.tools.ffmpeg.as_ref()?.with_file_name(format!("ffprobe{EXE}"));
    let mut command = Command::new(probe);
    command.args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"]).arg(run.source);
    hide_console(&mut command);
    let result = command.output().ok()?;
    String::from_utf8_lossy(&result.stdout).trim().parse().ok().filter(|seconds: &f64| *seconds > 0.0)
}

/// `out_time_us=12345678` from FFmpeg's progress output, as seconds.
fn progress_seconds(line: &str) -> Option<f64> {
    line.strip_prefix("out_time_us=")?.trim().parse::<f64>().ok().map(|us| us / 1_000_000.0)
}

fn run_ffmpeg(run: &Run, args: Vec<String>) -> Result<(), String> {
    let ffmpeg = run.tools.ffmpeg.as_ref().ok_or("FFmpeg is missing")?;
    let total = duration(run);
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-nostats", "-progress", "pipe:1", "-i"]);
    command.arg(run.source).args(args).arg(run.output);
    run_tool_watching(run, &mut command, &|line| {
        if let (Some(total), Some(seconds)) = (total, progress_seconds(line)) {
            run.reporter.set(seconds / total);
        }
    })?;
    require_output(run.output)
}

pub fn convert(run: &Run) -> Result<(), String> {
    let args = args_for(run.target, run.quality).ok_or("Unknown media target")?;
    run_ffmpeg(run, args)
}

/// Same format, smaller file.
pub fn compress(run: &Run, op: &Op) -> Result<(), String> {
    let Op::Compress { quality, max_edge } = *op else {
        return Err("Video can only be compressed".into());
    };
    let args = if run.from == "gif" {
        let band = match quality {
            75.. => Quality::High,
            45.. => Quality::Balanced,
            _ => Quality::Small,
        };
        gif_args(band, max_edge)
    } else {
        video_args(run.from, VideoLevel::from_percent(quality, max_edge))
    };
    run_ffmpeg(run, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_media_target_in_the_matrix_has_args() {
        for (name, format) in &crate::formats::matrix().formats {
            if format.family == "audio" || format.family == "video" {
                for target in &format.targets {
                    assert!(args_for(target, Quality::Balanced).is_some(), "{name} -> {target}");
                }
            }
        }
    }

    #[test]
    fn quality_changes_lossy_settings() {
        assert_ne!(args_for("mp4", Quality::High), args_for("mp4", Quality::Small));
        assert_ne!(args_for("mp3", Quality::High), args_for("mp3", Quality::Small));
        assert_ne!(args_for("gif", Quality::High), args_for("gif", Quality::Small));
    }

    #[test]
    fn lower_preset_quality_means_harder_compression() {
        let good = VideoLevel::from_percent(90, None);
        let poor = VideoLevel::from_percent(30, None);
        assert!(poor.x264_crf > good.x264_crf && poor.x264_crf <= 51);
        assert!(poor.vp9_crf > good.vp9_crf && poor.vp9_crf <= 63);
        assert!(poor.quantiser > good.quantiser && VideoLevel::from_percent(1, None).quantiser <= 31);
        assert!(video_args("mp4", good) != video_args("mp4", poor));
    }

    #[test]
    fn maximum_size_limits_the_longest_side() {
        let level = VideoLevel { max_edge: Some(1280), ..VideoLevel::from_quality(Quality::Balanced) };
        assert!(video_args("mp4", level).iter().any(|a| a.contains("min(1280,iw)")));
    }

    #[test]
    fn progress_lines_are_read_and_others_ignored() {
        assert_eq!(progress_seconds("out_time_us=2500000"), Some(2.5));
        assert_eq!(progress_seconds("out_time_us=N/A"), None);
        assert_eq!(progress_seconds("frame=12"), None);
    }

    #[test]
    fn extracting_audio_drops_the_picture() {
        assert_eq!(args_for("mp3", Quality::Balanced).unwrap()[0], "-vn");
    }
}
