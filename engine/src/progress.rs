//! Reading ffmpeg's `-progress pipe:1` stream. ffmpeg prints `key=value`
//! lines, one block per update, ending each block with `progress=continue`
//! or `progress=end`. `out_time_us` (and the misnamed `out_time_ms`) is the
//! output timestamp in microseconds, which against the planned output
//! duration gives a percentage.

/// Extra ffmpeg arguments that make it report progress on stdout.
pub const PROGRESS_ARGS: &[&str] = &["-progress", "pipe:1", "-nostats"];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProgressEvent {
    /// Output written so far, in seconds.
    OutTime(f64),
    /// The final block; the render is finished.
    End,
}

/// Parse one line of the progress stream. Lines we don't use (`frame=`,
/// `speed=`, `progress=continue`, `out_time_us=N/A`) return `None`.
pub fn parse_progress_line(line: &str) -> Option<ProgressEvent> {
    let (key, value) = line.trim().split_once('=')?;
    match key {
        "out_time_us" | "out_time_ms" => {
            let micros: i64 = value.trim().parse().ok()?;
            Some(ProgressEvent::OutTime(micros.max(0) as f64 / 1_000_000.0))
        }
        "progress" if value.trim() == "end" => Some(ProgressEvent::End),
        _ => None,
    }
}

/// `out_time` as a fraction of `planned`, clamped to `[0, 1]`. A zero or
/// negative plan reports 1 so a degenerate export still completes.
pub fn progress_fraction(out_time: f64, planned: f64) -> f64 {
    if planned <= 0.0 {
        return 1.0;
    }
    (out_time / planned).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_out_time_in_microseconds_under_both_names() {
        assert_eq!(
            parse_progress_line("out_time_us=1500000"),
            Some(ProgressEvent::OutTime(1.5))
        );
        assert_eq!(
            parse_progress_line("out_time_ms=250000\n"),
            Some(ProgressEvent::OutTime(0.25))
        );
    }

    #[test]
    fn end_block_is_reported_and_continue_is_not() {
        assert_eq!(
            parse_progress_line("progress=end"),
            Some(ProgressEvent::End)
        );
        assert_eq!(parse_progress_line("progress=continue"), None);
    }

    #[test]
    fn ignores_other_keys_and_garbage() {
        assert_eq!(parse_progress_line("frame=12"), None);
        assert_eq!(parse_progress_line("speed=1.58e+03x"), None);
        assert_eq!(parse_progress_line("out_time_us=N/A"), None);
        assert_eq!(parse_progress_line("out_time=00:00:01.000000"), None);
        assert_eq!(parse_progress_line(""), None);
        assert_eq!(parse_progress_line("no equals sign"), None);
    }

    #[test]
    fn negative_out_time_clamps_to_zero() {
        assert_eq!(
            parse_progress_line("out_time_us=-9223372036854775808"),
            Some(ProgressEvent::OutTime(0.0))
        );
    }

    #[test]
    fn fraction_is_clamped_and_tolerates_a_zero_plan() {
        assert_eq!(progress_fraction(5.0, 10.0), 0.5);
        assert_eq!(progress_fraction(12.0, 10.0), 1.0);
        assert_eq!(progress_fraction(-1.0, 10.0), 0.0);
        assert_eq!(progress_fraction(0.0, 0.0), 1.0);
    }
}
