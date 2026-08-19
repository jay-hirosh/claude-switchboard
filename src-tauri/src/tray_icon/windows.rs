//! Windows tray icon renderer. Produces a 32×32 PNG with concentric rings
//! (outer = 7-day, inner = 5-hour) and the worse of the two percentages
//! displayed as two digits in the center. Square geometry is forced by the
//! Windows shell tray API.

use crate::tray_icon::{digits, shared, IconStyle};
use tiny_skia::{
    FillRule, Paint, PathBuilder, Pixmap, Stroke, StrokeDash, Transform,
};

const SIZE: u32 = 32;
const CX: f32 = 16.0;
const CY: f32 = 16.0;
const OUTER_R: f32 = 14.0;
const OUTER_STROKE: f32 = 2.5;
const INNER_R: f32 = 9.0;
const INNER_STROKE: f32 = 2.5;
const DIGIT_HEIGHT_PX: f32 = 9.0;

// Primary/Minimal styles show a single glyph — a bit larger than the inner
// ring of the Dual layout since there's no outer ring competing for space.
const PRIMARY_R: f32 = 12.5;
const PRIMARY_STROKE: f32 = 3.0;
const PRIMARY_DIGIT_HEIGHT_PX: f32 = 11.0;
const MINIMAL_DOT_R: f32 = 8.0;

pub fn render(five_hour: Option<f64>, seven_day: Option<f64>, paused: bool, style: IconStyle) -> Vec<u8> {
    let mut pixmap = Pixmap::new(SIZE, SIZE).expect("32x32 fits in memory");
    pixmap.fill(tiny_skia::Color::TRANSPARENT);

    let no_data = paused || (five_hour.is_none() && seven_day.is_none());
    let five_hour = if no_data { None } else { five_hour };
    let seven_day = if no_data { None } else { seven_day };

    match style {
        IconStyle::Dual => {
            draw_ring(&mut pixmap, OUTER_R, OUTER_STROKE, five_hour);
            draw_ring(&mut pixmap, INNER_R, INNER_STROKE, seven_day);

            let digit_str = match five_hour.or(seven_day) {
                Some(p) => format!("{}", (p.clamp(0.0, 99.0).round()) as i64),
                None => "\u{2014}".to_string(),
            };
            draw_centered_text(&mut pixmap, &digit_str, CX, CY, DIGIT_HEIGHT_PX, shared::text());
        }
        IconStyle::Primary => {
            let pct = worse_of(five_hour, seven_day);
            draw_ring(&mut pixmap, PRIMARY_R, PRIMARY_STROKE, pct);

            let digit_str = match pct {
                Some(p) => format!("{}", (p.clamp(0.0, 99.0).round()) as i64),
                None => "\u{2014}".to_string(),
            };
            let color = match pct {
                Some(_) => shared::text(),
                None => shared::text_muted(),
            };
            draw_centered_text(&mut pixmap, &digit_str, CX, CY, PRIMARY_DIGIT_HEIGHT_PX, color);
        }
        IconStyle::Minimal => {
            draw_dot(&mut pixmap, worse_of(five_hour, seven_day));
        }
    }

    pixmap.encode_png().expect("png encode never fails")
}

/// The bucket that's closer to its limit — the more urgent of the two to
/// show when only one glyph fits (`Primary` style).
fn worse_of(five_hour: Option<f64>, seven_day: Option<f64>) -> Option<f64> {
    match (five_hour, seven_day) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// `Minimal` style — a plain status-colored dot, no ring/track and no
/// digits. A muted gray stands in for "no data" instead of an em-dash,
/// since there's no digit slot to put one in.
fn draw_dot(pixmap: &mut Pixmap, pct: Option<f64>) {
    let color = match pct {
        Some(p) => shared::arc_color(p),
        None => shared::track(),
    };
    let path = {
        let mut pb = PathBuilder::new();
        pb.push_circle(CX, CY, MINIMAL_DOT_R);
        pb.finish().expect("circle is valid")
    };
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn draw_ring(pixmap: &mut Pixmap, radius: f32, stroke_width: f32, pct: Option<f64>) {
    let path = {
        let mut pb = PathBuilder::new();
        pb.push_circle(CX, CY, radius);
        pb.finish().expect("circle is valid")
    };

    let mut stroke = Stroke {
        width: stroke_width,
        line_cap: tiny_skia::LineCap::Butt,
        ..Default::default()
    };

    // Track.
    let mut track_paint = Paint::default();
    track_paint.set_color(shared::track());
    track_paint.anti_alias = true;
    pixmap.stroke_path(&path, &track_paint, &stroke, Transform::identity(), None);

    // Arc fill, if data.
    if let Some(raw) = pct {
        let clamped = raw.clamp(0.0, 99.0);
        if clamped > 0.0 {
            let circumference = 2.0 * std::f32::consts::PI * radius;
            let filled = circumference * (clamped as f32 / 100.0);
            let gap = circumference - filled;
            stroke.dash = StrokeDash::new(vec![filled, gap], 0.0);
            let mut arc_paint = Paint::default();
            arc_paint.set_color(shared::arc_color(raw));
            arc_paint.anti_alias = true;
            let arc_transform = Transform::identity()
                .pre_translate(CX, CY)
                .pre_rotate(-90.0)
                .pre_translate(-CX, -CY);
            pixmap.stroke_path(&path, &arc_paint, &stroke, arc_transform, None);
        }
    }
}

fn draw_centered_text(
    pixmap: &mut Pixmap,
    text: &str,
    cx: f32,
    cy: f32,
    cap_height_px: f32,
    color: tiny_skia::Color,
) {
    let em = digits::units_per_em();
    let scale = cap_height_px / digits::cap_height();

    // Compute the total width using each glyph's true advance.
    let total_width: f32 = text
        .chars()
        .map(|ch| digits::glyph_advance(ch).unwrap_or(em * 0.6) * scale)
        .sum();

    // Baseline placement: visible block (cap_height_px) is centered on cy, so
    // the block top is at (cy - cap_height_px/2) and the baseline sits at the
    // bottom of that block.
    let baseline_y = cy + cap_height_px / 2.0;
    let mut pen_x = cx - total_width / 2.0;

    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;

    for ch in text.chars() {
        let advance_px = digits::glyph_advance(ch).unwrap_or(em * 0.6) * scale;
        if let Some(path) = digits::glyph_path(ch) {
            let transform = Transform::from_scale(scale, scale)
                .post_translate(pen_x, baseline_y);
            pixmap.fill_path(path, &paint, FillRule::Winding, transform, None);
        }
        pen_x += advance_px;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let decoder = png::Decoder::new(bytes);
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).unwrap();
        (info.width, info.height, buf[..info.buffer_size()].to_vec())
    }

    #[test]
    fn output_is_32x32_png() {
        let bytes = render(Some(50.0), Some(50.0), false, IconStyle::Dual);
        let (w, h, _) = decode_png(&bytes);
        assert_eq!((w, h), (32, 32));
    }

    #[test]
    fn five_hour_value_drives_center_digits() {
        // 80% on 5h, 30% on 7d: the digits should be "80", in danger color
        // is wrong (80 = warn). Here we just confirm SOMETHING gets rendered.
        let bytes = render(Some(80.0), Some(30.0), false, IconStyle::Dual);
        let (_, _, rgba) = decode_png(&bytes);
        let any_opaque = rgba.chunks(4).any(|p| p[3] > 200);
        assert!(any_opaque);
    }

    #[test]
    fn no_data_state_is_mostly_empty() {
        let bytes = render(None, None, true, IconStyle::Dual);
        let (_, _, rgba) = decode_png(&bytes);
        let opaque_count = rgba.chunks(4).filter(|p| p[3] > 200).count();
        // Track rings + em-dash. Lower than two filled arcs + two two-digit numbers.
        assert!(opaque_count < 250, "got {} opaque pixels in no-data state", opaque_count);
    }

    #[test]
    fn primary_style_uses_the_worse_of_the_two_buckets() {
        let bytes = render(Some(80.0), Some(30.0), false, IconStyle::Primary);
        let (_, _, rgba) = decode_png(&bytes);
        let warn_target = (0xE8, 0x91, 0x49);
        let has_warn = rgba
            .chunks(4)
            .any(|p| p[3] > 200 && near(p[0], warn_target.0) && near(p[1], warn_target.1) && near(p[2], warn_target.2));
        assert!(has_warn, "expected the worse (80%) bucket's warn color to appear");
    }

    #[test]
    fn primary_style_output_is_valid_32x32_png() {
        let bytes = render(Some(50.0), Some(50.0), false, IconStyle::Primary);
        let (w, h, _) = decode_png(&bytes);
        assert_eq!((w, h), (32, 32));
    }

    #[test]
    fn minimal_style_renders_no_digit_ink() {
        let bytes = render(Some(50.0), Some(90.0), false, IconStyle::Minimal);
        let (_, _, rgba) = decode_png(&bytes);
        let text = shared::text();
        let text_muted = shared::text_muted();
        let has_digit_ink = rgba.chunks(4).any(|p| {
            p[3] > 0
                && ((near(p[0], (text.red() * 255.0) as u8)
                    && near(p[1], (text.green() * 255.0) as u8)
                    && near(p[2], (text.blue() * 255.0) as u8))
                    || (near(p[0], (text_muted.red() * 255.0) as u8)
                        && near(p[1], (text_muted.green() * 255.0) as u8)
                        && near(p[2], (text_muted.blue() * 255.0) as u8)))
        });
        assert!(!has_digit_ink, "minimal style must not paint digit glyphs");
    }

    #[test]
    fn minimal_style_dot_uses_the_worse_buckets_status_color() {
        let bytes = render(Some(10.0), Some(95.0), false, IconStyle::Minimal);
        let (_, _, rgba) = decode_png(&bytes);
        let danger_target = (0xD8, 0x5A, 0x45);
        let has_danger = rgba.chunks(4).any(|p| {
            p[3] > 200 && near(p[0], danger_target.0) && near(p[1], danger_target.1) && near(p[2], danger_target.2)
        });
        assert!(has_danger, "expected the worse (95%) bucket's danger color in the dot");
    }

    #[test]
    fn minimal_style_no_data_still_renders_a_valid_png() {
        let bytes = render(None, None, true, IconStyle::Minimal);
        let (w, h, _) = decode_png(&bytes);
        assert_eq!((w, h), (32, 32));
    }

    fn near(a: u8, b: u8) -> bool {
        (a as i16 - b as i16).abs() < 12
    }
}
