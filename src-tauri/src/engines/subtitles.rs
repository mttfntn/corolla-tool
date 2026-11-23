//! SRT, VTT and plain text.

use super::Run;

#[derive(Debug, PartialEq, Clone)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// How long each line shows when plain text becomes subtitles.
const TEXT_CUE_MS: u64 = 3000;

/// Accepts `HH:MM:SS,mmm`, `HH:MM:SS.mmm` and VTT's `MM:SS.mmm`.
fn parse_time(text: &str) -> Option<u64> {
    let text = text.trim().replace(',', ".");
    let (clock, fraction) = text.split_once('.')?;
    let milliseconds: u64 = format!("{fraction:0<3}").get(..3)?.parse().ok()?;
    let parts: Vec<u64> = clock.split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let seconds = match parts.as_slice() {
        [m, s] => m * 60 + s,
        [h, m, s] => h * 3600 + m * 60 + s,
        _ => return None,
    };
    Some(seconds * 1000 + milliseconds)
}

fn parse_timing(line: &str) -> Option<(u64, u64)> {
    let (start, rest) = line.split_once("-->")?;
    // VTT may put settings after the end time, like "align:start".
    let end = rest.split_whitespace().next()?;
    Some((parse_time(start)?, parse_time(end)?))
}

/// Parses SRT or VTT. Anything that isn't a timed cue (VTT header, NOTE and
/// STYLE blocks) is ignored, since those blocks have no timing line.
pub fn parse(text: &str) -> Vec<Cue> {
    let clean = text.trim_start_matches('\u{FEFF}').replace("\r\n", "\n").replace('\r', "\n");
    let mut cues = Vec::new();
    for block in clean.split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        let Some(timing_at) = lines.iter().position(|l| l.contains("-->")) else { continue };
        let Some((start_ms, end_ms)) = parse_timing(lines[timing_at]) else { continue };
        let body = lines[timing_at + 1..].join("\n");
        if !body.trim().is_empty() {
            cues.push(Cue { start_ms, end_ms, text: body });
        }
    }
    cues
}

fn format_time(ms: u64, separator: char) -> String {
    format!(
        "{:02}:{:02}:{:02}{separator}{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

pub fn to_srt(cues: &[Cue]) -> String {
    let mut out = String::new();
    for (i, cue) in cues.iter().enumerate() {
        out += &format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            format_time(cue.start_ms, ','),
            format_time(cue.end_ms, ','),
            cue.text
        );
    }
    out
}

pub fn to_vtt(cues: &[Cue]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for cue in cues {
        out += &format!(
            "{} --> {}\n{}\n\n",
            format_time(cue.start_ms, '.'),
            format_time(cue.end_ms, '.'),
            cue.text
        );
    }
    out
}

/// Spoken text only: inline tags like `<i>` are removed.
pub fn to_text(cues: &[Cue]) -> String {
    let mut out = String::new();
    for cue in cues {
        let mut in_tag = false;
        for c in cue.text.chars() {
            match c {
                '<' => in_tag = true,
                '>' if in_tag => in_tag = false,
                c if !in_tag => out.push(c),
                _ => {}
            }
        }
        out.push('\n');
    }
    out
}

/// One cue per non-empty line, evenly spaced, ready for retiming in an editor.
pub fn from_text(text: &str) -> Vec<Cue> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .enumerate()
        .map(|(i, line)| Cue {
            start_ms: i as u64 * TEXT_CUE_MS,
            end_ms: (i as u64 + 1) * TEXT_CUE_MS,
            text: line.to_string(),
        })
        .collect()
}

/// Reads text in UTF-8, or Windows-1252 as older subtitle files often are.
pub fn read_text(path: &std::path::Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Can't read file: {e}"))?;
    Ok(match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => error.into_bytes().iter().map(|b| windows_1252(*b)).collect(),
    })
}

fn windows_1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}',
        '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
    ];
    match byte {
        0x80..=0x9F => HIGH[(byte - 0x80) as usize],
        other => other as char,
    }
}

pub fn convert_file(run: &Run) -> Result<(), String> {
    let source = read_text(run.source)?;
    let cues = parse(&source);
    if cues.is_empty() {
        return Err("No subtitles found in this file".into());
    }
    let result = match run.target {
        "srt" => to_srt(&cues),
        "vtt" => to_vtt(&cues),
        "txt" => to_text(&cues),
        other => return Err(format!("Unknown subtitle target {other}")),
    };
    std::fs::write(run.output, result).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRT: &str = "1\r\n00:00:01,500 --> 00:00:03,000\r\nHello\r\nthere\r\n\r\n2\r\n01:02:03,004 --> 01:02:04,000\r\n<i>Bye</i>\r\n";

    #[test]
    fn parses_srt_with_crlf_and_multiple_lines() {
        let cues = parse(SRT);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0], Cue { start_ms: 1500, end_ms: 3000, text: "Hello\nthere".into() });
        assert_eq!(cues[1].start_ms, 3_723_004);
    }

    #[test]
    fn parses_vtt_skipping_header_notes_and_settings() {
        let vtt = "\u{FEFF}WEBVTT - title\n\nNOTE a comment\n\nSTYLE\n::cue {}\n\nintro\n00:01.000 --> 00:02.500 align:start\nHi\n";
        assert_eq!(parse(vtt), vec![Cue { start_ms: 1000, end_ms: 2500, text: "Hi".into() }]);
    }

    #[test]
    fn srt_to_vtt_round_trips() {
        let cues = parse(SRT);
        let vtt = to_vtt(&cues);
        assert!(vtt.starts_with("WEBVTT\n\n"));
        assert!(vtt.contains("00:00:01.500 --> 00:00:03.000"));
        assert_eq!(parse(&vtt), cues);
        assert_eq!(parse(&to_srt(&cues)), cues);
    }

    #[test]
    fn srt_numbers_cues_and_uses_commas() {
        let srt = to_srt(&parse(SRT));
        assert!(srt.starts_with("1\n00:00:01,500 --> 00:00:03,000\n"));
        assert!(srt.contains("\n2\n01:02:03,004"));
    }

    #[test]
    fn text_output_drops_timing_and_tags() {
        assert_eq!(to_text(&parse(SRT)), "Hello\nthere\nBye\n");
    }

    #[test]
    fn plain_text_becomes_timed_cues() {
        let cues = from_text("one\n\n  two  \n");
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[1], Cue { start_ms: 3000, end_ms: 6000, text: "two".into() });
    }

    #[test]
    fn short_fractions_are_milliseconds_not_units() {
        assert_eq!(parse_time("00:00:01.5"), Some(1500));
    }

    #[test]
    fn broken_timing_is_skipped_not_fatal() {
        let cues = parse("1\nnonsense --> also\nx\n\n2\n00:00:01,000 --> 00:00:02,000\nok\n");
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "ok");
    }

    #[test]
    fn windows_1252_accents_decode() {
        assert_eq!(windows_1252(0xE8), 'è');
        assert_eq!(windows_1252(0x80), '€');
    }
}
