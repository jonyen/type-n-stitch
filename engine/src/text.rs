//! Rasterising title cards and caption boxes.
//!
//! The installed ffmpeg builds have no `drawtext` (they are compiled without
//! libfreetype), so the engine draws text itself with `ab_glyph` and hands
//! ffmpeg PNGs to composite. Inter (SIL OFL) is bundled and is the only font.

use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont};

use crate::ffmpeg::VideoInfo;
use crate::types::{CaptionPos, TitleStyle};

pub const FONT_REGULAR: &[u8] = include_bytes!("../assets/inter/Inter-Regular.ttf");
pub const FONT_SEMIBOLD: &[u8] = include_bytes!("../assets/inter/Inter-SemiBold.ttf");

/// Straight (non-premultiplied) RGBA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba(pub [u8; 4]);

/// Text wraps at this share of the frame width.
const WRAP: f32 = 0.8;
/// Padding around a caption's lettering, in pixels.
const CAPTION_PAD: u32 = 12;
const CAPTION_BG: Rgba = Rgba([0, 0, 0, 140]);
const WHITE: Rgba = Rgba([255, 255, 255, 255]);

/// A rendered image: RGBA pixels, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Raster {
    /// A raster filled with one colour.
    pub fn filled(width: u32, height: u32, color: Rgba) -> Self {
        let rgba = color
            .0
            .iter()
            .copied()
            .cycle()
            .take((width * height * 4) as usize)
            .collect();
        Self {
            width,
            height,
            rgba,
        }
    }

    /// Source-over compositing of `color` at `coverage` (0..=1).
    fn blend(&mut self, x: u32, y: u32, color: Rgba, coverage: f32) {
        if x >= self.width || y >= self.height || coverage <= 0.0 {
            return;
        }
        let sa = color.0[3] as f32 / 255.0 * coverage.min(1.0);
        if sa <= 0.0 {
            return;
        }
        let i = ((y * self.width + x) * 4) as usize;
        let da = self.rgba[i + 3] as f32 / 255.0;
        let out_a = sa + da * (1.0 - sa);
        for c in 0..3 {
            let s = color.0[c] as f32 / 255.0;
            let d = self.rgba[i + c] as f32 / 255.0;
            let out = (s * sa + d * da * (1.0 - sa)) / out_a;
            self.rgba[i + c] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        self.rgba[i + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
    }

    /// PNG bytes, 8-bit RGBA.
    pub fn to_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("png header");
            writer.write_image_data(&self.rgba).expect("png data");
        }
        out
    }
}

/// A caption's image and where `overlay` should put it on the frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptionBox {
    pub raster: Raster,
    pub x: u32,
    pub y: u32,
}

/// Width of one line of text at `size`, including kerning.
fn line_width(text: &str, font: &FontRef, size: f32) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    let mut width = 0.0;
    let mut prev = None;
    for ch in text.chars() {
        let id = scaled.glyph_id(ch);
        if let Some(prev) = prev {
            width += scaled.kern(prev, id);
        }
        width += scaled.h_advance(id);
        prev = Some(id);
    }
    width
}

/// Distance between the baselines of consecutive lines.
fn line_advance(font: &FontRef, size: f32) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    scaled.height() + scaled.line_gap()
}

/// Greedy word wrapping at `max_width`. A word wider than the line keeps its
/// own line rather than being broken mid-word.
pub fn layout_lines(text: &str, font: &FontRef, size: f32, max_width: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
            continue;
        }
        let candidate = format!("{current} {word}");
        if line_width(&candidate, font, size) <= max_width {
            current = candidate;
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Draw `lines` centred horizontally, the block's vertical centre at `centre_y`.
fn draw_block(
    raster: &mut Raster,
    lines: &[String],
    font: &FontRef,
    size: f32,
    centre_y: f32,
    color: Rgba,
) {
    let scaled = font.as_scaled(PxScale::from(size));
    let advance = line_advance(font, size);
    let top = centre_y - advance * lines.len() as f32 / 2.0;
    for (i, line) in lines.iter().enumerate() {
        let baseline = top + advance * i as f32 + scaled.ascent();
        let x = (raster.width as f32 - line_width(line, font, size)) / 2.0;
        draw_line(raster, line, font, size, x, baseline, color);
    }
}

/// Draw one line with its left edge at `x` and its baseline at `baseline`.
fn draw_line(
    raster: &mut Raster,
    text: &str,
    font: &FontRef,
    size: f32,
    x: f32,
    baseline: f32,
    color: Rgba,
) {
    let scaled = font.as_scaled(PxScale::from(size));
    let mut pen = x;
    let mut prev = None;
    for ch in text.chars() {
        let id = scaled.glyph_id(ch);
        if let Some(prev) = prev {
            pen += scaled.kern(prev, id);
        }
        let glyph = id.with_scale_and_position(PxScale::from(size), point(pen, baseline));
        if let Some(outline) = font.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                let px = bounds.min.x as i32 + gx as i32;
                let py = bounds.min.y as i32 + gy as i32;
                if px >= 0 && py >= 0 {
                    raster.blend(px as u32, py as u32, color, coverage);
                }
            });
        }
        pen += scaled.h_advance(id);
        prev = Some(id);
    }
}

/// Background and foreground for a title card.
fn title_colors(style: TitleStyle) -> (Rgba, Rgba) {
    match style {
        TitleStyle::Dark => (Rgba([0x11, 0x11, 0x11, 255]), WHITE),
        TitleStyle::Light => (Rgba([0xf6, 0xf6, 0xf7, 255]), Rgba([0x17, 0x18, 0x1a, 255])),
        TitleStyle::Accent => (Rgba([0x25, 0x63, 0xeb, 255]), WHITE),
    }
}

/// A full-frame title card: the style's background, the title centred in
/// semibold at `height/12` and, when given, a subtitle in regular at
/// `height/24`. With a subtitle the title block centres at 42% of the height
/// and the subtitle at 58%; alone the title centres on the frame.
pub fn render_title(
    text: &str,
    subtitle: Option<&str>,
    style: TitleStyle,
    video: VideoInfo,
) -> Raster {
    let (bg, fg) = title_colors(style);
    let mut raster = Raster::filled(video.width, video.height, bg);
    let semibold = FontRef::try_from_slice(FONT_SEMIBOLD).expect("bundled semibold font parses");
    let regular = FontRef::try_from_slice(FONT_REGULAR).expect("bundled regular font parses");
    let max_width = video.width as f32 * WRAP;
    let big = video.height as f32 / 12.0;
    let small = video.height as f32 / 24.0;
    let h = video.height as f32;

    let sub = subtitle.filter(|s| !s.trim().is_empty());
    let title_centre = if sub.is_some() { h * 0.42 } else { h / 2.0 };
    let lines = layout_lines(text, &semibold, big, max_width);
    draw_block(&mut raster, &lines, &semibold, big, title_centre, fg);
    if let Some(sub) = sub {
        let lines = layout_lines(sub, &regular, small, max_width);
        draw_block(&mut raster, &lines, &regular, small, h * 0.58, fg);
    }
    raster
}

/// A caption box: white regular text at `height/28` on rgba(0,0,0,140) with
/// 12 px of padding, plus where `overlay` should place it.
pub fn render_caption(text: &str, position: CaptionPos, video: VideoInfo) -> CaptionBox {
    let font = FontRef::try_from_slice(FONT_REGULAR).expect("bundled regular font parses");
    let size = video.height as f32 / 28.0;
    let max_width = video.width as f32 * WRAP;
    let lines = layout_lines(text, &font, size, max_width);
    let widest = lines
        .iter()
        .map(|l| line_width(l, &font, size))
        .fold(0.0_f32, f32::max);
    let advance = line_advance(&font, size);
    let pad = CAPTION_PAD;
    let width = widest.ceil() as u32 + 2 * pad;
    let height = (advance * lines.len() as f32).ceil() as u32 + 2 * pad;

    let mut raster = Raster::filled(width, height, CAPTION_BG);
    let scaled = font.as_scaled(PxScale::from(size));
    for (i, line) in lines.iter().enumerate() {
        let baseline = pad as f32 + advance * i as f32 + scaled.ascent();
        draw_line(&mut raster, line, &font, size, pad as f32, baseline, WHITE);
    }

    // Integer percentages of the frame, so the numbers do not wobble with
    // floating-point rounding.
    let left = video.width * 5 / 100;
    let bottom = video.height * 85 / 100;
    let (x, y) = match position {
        CaptionPos::BottomLeft => (left, bottom.saturating_sub(height)),
        CaptionPos::BottomCenter => (
            video.width.saturating_sub(width) / 2,
            bottom.saturating_sub(height),
        ),
        CaptionPos::TopLeft => (left, video.height * 8 / 100),
    };
    CaptionBox { raster, x, y }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: VideoInfo = VideoInfo {
        width: 1280,
        height: 720,
        fps: 30.0,
    };

    fn px(r: &Raster, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * r.width + x) * 4) as usize;
        [r.rgba[i], r.rgba[i + 1], r.rgba[i + 2], r.rgba[i + 3]]
    }

    #[test]
    fn title_fills_the_frame_with_the_style_colour_and_draws_text() {
        let r = render_title("Hello", None, TitleStyle::Accent, V);
        assert_eq!((r.width, r.height), (1280, 720));
        assert_eq!(r.rgba.len(), 1280 * 720 * 4);
        let bg = px(&r, 0, 0);
        assert_eq!(bg, [0x25, 0x63, 0xeb, 255]);
        // Somewhere across the middle band the lettering differs from the card.
        let band = (300..420).any(|y| (0..1280).any(|x| px(&r, x, y) != bg));
        assert!(band, "no text pixels near the centre");
    }

    #[test]
    fn title_styles_use_their_own_backgrounds() {
        assert_eq!(
            px(&render_title("T", None, TitleStyle::Dark, V), 0, 0),
            [0x11, 0x11, 0x11, 255]
        );
        assert_eq!(
            px(&render_title("T", None, TitleStyle::Light, V), 0, 0),
            [0xf6, 0xf6, 0xf7, 255]
        );
    }

    #[test]
    fn a_subtitle_puts_ink_in_both_bands() {
        let r = render_title("Big", Some("small"), TitleStyle::Dark, V);
        let bg = px(&r, 0, 0);
        let ink = |mut ys: std::ops::Range<u32>| ys.any(|y| (0..1280).any(|x| px(&r, x, y) != bg));
        // Title block around 42% of 720 = 302, subtitle around 58% = 418.
        assert!(ink(270..330), "no title ink");
        assert!(ink(395..445), "no subtitle ink");
    }

    #[test]
    fn long_titles_wrap_onto_several_lines() {
        let font = FontRef::try_from_slice(FONT_SEMIBOLD).unwrap();
        let text = "word ".repeat(40);
        let lines = layout_lines(text.trim(), &font, 60.0, 1024.0);
        assert!(lines.len() > 1, "expected wrapping, got {lines:?}");
        assert_eq!(lines.join(" ").split_whitespace().count(), 40);
        assert_eq!(layout_lines("short", &font, 60.0, 1024.0), vec!["short"]);
        // A single word longer than the line still yields one line.
        assert_eq!(
            layout_lines("x".repeat(200).as_str(), &font, 60.0, 100.0).len(),
            1
        );
    }

    #[test]
    fn caption_box_is_smaller_than_the_frame_and_sits_inside_it() {
        for pos in [
            CaptionPos::BottomLeft,
            CaptionPos::BottomCenter,
            CaptionPos::TopLeft,
        ] {
            let b = render_caption("Ada Lovelace", pos, V);
            assert!(
                b.raster.width < V.width && b.raster.height < V.height,
                "{pos:?}"
            );
            assert!(b.x + b.raster.width <= V.width, "{pos:?} overflows right");
            assert!(
                b.y + b.raster.height <= V.height,
                "{pos:?} overflows bottom"
            );
        }
        let left = render_caption("Ada", CaptionPos::BottomLeft, V);
        let centre = render_caption("Ada", CaptionPos::BottomCenter, V);
        let top = render_caption("Ada", CaptionPos::TopLeft, V);
        assert_eq!(left.x, 64, "5% of 1280");
        assert_eq!(top.x, 64);
        assert_eq!(top.y, 57, "8% of 720");
        assert_eq!(centre.x, (V.width - centre.raster.width) / 2);
        // 85% of 720 = 612, minus the box height, which depends on font metrics.
        assert_eq!(left.y, 612 - left.raster.height);
        assert!(left.y < 612);
        assert_eq!(centre.y, left.y);
    }

    #[test]
    fn caption_box_is_translucent_black_with_white_text() {
        let b = render_caption("Ada", CaptionPos::BottomLeft, V);
        let r = &b.raster;
        assert_eq!(px(r, 0, 0), [0, 0, 0, 140], "padding is the box colour");
        let mid = r.height / 2;
        let white = (0..r.width).any(|x| px(r, x, mid) == [255, 255, 255, 255]);
        assert!(white, "no white lettering across the middle row");
    }

    #[test]
    fn to_png_round_trips_the_dimensions() {
        let b = render_caption("Ada", CaptionPos::TopLeft, V);
        let bytes = b.raster.to_png();
        let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (b.raster.width, b.raster.height));
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(&buf[..info.buffer_size()], &b.raster.rgba[..]);
    }
}
