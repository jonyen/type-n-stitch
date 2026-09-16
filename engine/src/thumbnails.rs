//! Scrubber thumbnails: one JPEG sprite sheet of evenly spaced frames, so the
//! client can show a preview while hovering the timeline with a single request.

use serde::Serialize;

/// Width and height of one thumbnail cell. Frames are letterboxed into it.
pub const THUMB_WIDTH: u32 = 160;
pub const THUMB_HEIGHT: u32 = 90;
/// Upper bound on frames per sheet; long sources get a coarser interval.
pub const MAX_THUMBS: u32 = 120;
/// Never sample more often than this, however short the source.
pub const MIN_INTERVAL: f64 = 0.5;
const COLUMNS: u32 = 10;

/// Layout of a sprite sheet. Cell `i` shows the frame at `i * interval` and
/// sits at column `i % columns`, row `i / columns`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailSheet {
    pub count: u32,
    pub columns: u32,
    pub rows: u32,
    pub interval: f64,
    pub width: u32,
    pub height: u32,
}

/// Plan a sheet for a source of `duration` seconds.
pub fn thumbnail_sheet(duration: f64) -> ThumbnailSheet {
    let duration = duration.max(0.0);
    let interval = (duration / f64::from(MAX_THUMBS)).max(MIN_INTERVAL);
    let count = ((duration / interval).ceil() as u32).clamp(1, MAX_THUMBS);
    let columns = count.min(COLUMNS);
    ThumbnailSheet {
        count,
        columns,
        rows: count.div_ceil(columns),
        interval,
        width: THUMB_WIDTH,
        height: THUMB_HEIGHT,
    }
}

/// ffmpeg arguments that render `sheet` from `input` into the JPEG `output`.
/// Frames are taken at the start of each interval.
pub fn thumbnail_args(input: &str, output: &str, sheet: &ThumbnailSheet) -> Vec<String> {
    let filter = format!(
        "fps=1/{interval}:start_time=0:round=down,\
         scale={w}:{h}:force_original_aspect_ratio=decrease,\
         pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black,\
         tile={cols}x{rows}",
        interval = sheet.interval,
        w = sheet.width,
        h = sheet.height,
        cols = sheet.columns,
        rows = sheet.rows,
    );
    [
        "-y",
        "-loglevel",
        "error",
        "-i",
        input,
        "-an",
        "-vf",
        &filter,
        "-frames:v",
        "1",
        "-q:v",
        "5",
        output,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// Index of the cell to show for source time `t`.
pub fn thumbnail_index(t: f64, sheet: &ThumbnailSheet) -> u32 {
    let i = (t.max(0.0) / sheet.interval).floor() as u32;
    i.min(sheet.count - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_sources_sample_every_half_second() {
        let sheet = thumbnail_sheet(20.0);
        assert_eq!(sheet.interval, 0.5);
        assert_eq!(sheet.count, 40);
        assert_eq!((sheet.columns, sheet.rows), (10, 4));
    }

    #[test]
    fn long_sources_cap_the_frame_count() {
        let sheet = thumbnail_sheet(3600.0);
        assert_eq!(sheet.count, MAX_THUMBS);
        assert_eq!(sheet.interval, 30.0);
        assert_eq!(sheet.rows, 12);
    }

    #[test]
    fn tiny_sources_still_get_one_cell() {
        let sheet = thumbnail_sheet(0.2);
        assert_eq!(sheet.count, 1);
        assert_eq!((sheet.columns, sheet.rows), (1, 1));
        let empty = thumbnail_sheet(0.0);
        assert_eq!(empty.count, 1);
    }

    #[test]
    fn partial_last_row_rounds_up() {
        let sheet = thumbnail_sheet(6.2);
        assert_eq!(sheet.count, 13);
        assert_eq!((sheet.columns, sheet.rows), (10, 2));
    }

    #[test]
    fn index_clamps_to_the_sheet() {
        let sheet = thumbnail_sheet(20.0);
        assert_eq!(thumbnail_index(-1.0, &sheet), 0);
        assert_eq!(thumbnail_index(0.49, &sheet), 0);
        assert_eq!(thumbnail_index(0.5, &sheet), 1);
        assert_eq!(thumbnail_index(20.0, &sheet), 39);
    }

    #[test]
    fn args_tile_the_planned_grid() {
        let sheet = thumbnail_sheet(20.0);
        let args = thumbnail_args("in.mp4", "out.jpg", &sheet);
        let filter = &args[args.iter().position(|a| a == "-vf").unwrap() + 1];
        assert!(filter.starts_with("fps=1/0.5:"));
        assert!(filter.contains("scale=160:90:force_original_aspect_ratio=decrease"));
        assert!(filter.ends_with("tile=10x4"));
        assert_eq!(args.last().unwrap(), "out.jpg");
    }
}
