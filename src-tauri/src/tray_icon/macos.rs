//! macOS dual-pie tray icon renderer. Produces an 88×44 PNG showing two
//! side-by-side pies — left for 5-hour, right for 7-day — each with its own
//! arc fill, threshold-keyed color, and inscribed two-digit number.

use crate::tray_icon::{digits, shared, IconStyle};
use tiny_skia::{
    FillRule, Paint, PathBuilder, Pixmap, Stroke, StrokeDash, Transform,
};

const CANVAS_W: u32 = 88;
const CANVAS_H: u32 = 44;
const CELL_W: f32 = 44.0;

// All measurements below are in pixels (the Pixmap pixel space).
const RING_CX: f32 = 22.0;       // cell center x
const RING_CY: f32 = 22.0;       // cell center y — middle of the cell now that
                                  // the bucket-position labels are gone (left=5h, right=7d)
const RING_R: f32 = 17.0;        // ring radius — bigger now that the label slot is reclaimed
const RING_STROKE: f32 = 4.0;    // ring stroke width
const DIGIT_HEIGHT_PX: f32 = 17.0; // cap height of the digits in pixels

// Primary/Minimal styles use the full 88×44 canvas for a single glyph,
// centered rather than split into two cells.
const SINGLE_CX: f32 = CANVAS_W as f32 / 2.0;
const SINGLE_CY: f32 = CANVAS_H as f32 / 2.0;
const PRIMARY_R: f32 = 19.0;
const PRIMARY_STROKE: f32 = 5.0;
const PRIMARY_DIGIT_HEIGHT_PX: f32 = 19.0;
const MINIMAL_DOT_R: f32 = 11.0;

pub fn render(five_hour: Option<f64>, seven_day: Option<f64>, paused: bool, style: IconStyle) -> Vec<u8> {
    let mut pixmap = Pixmap::new(CANVAS_W, CANVAS_H).expect("88x44 fits in memory");
    pixmap.fill(tiny_skia::Color::TRANSPARENT);

    let no_data = paused || (five_hour.is_none() && seven_day.is_none());
    let five_hour = if no_data { None } else { five_hour };
    let seven_day = if no_data { None } else { seven_day };

    match style {
        IconStyle::Dual => {
            draw_pie(&mut pixmap, RING_CX, RING_CY, RING_R, RING_STROKE, DIGIT_HEIGHT_PX, five_hour);
            draw_pie(&mut pixmap, CELL_W + RING_CX, RING_CY, RING_R, RING_STROKE, DIGIT_HEIGHT_PX, seven_day);
        }
        IconStyle::Primary => {
            draw_pie(&mut pixmap, SINGLE_CX, SINGLE_CY, PRIMARY_R, PRIMARY_STROKE, PRIMARY_DIGIT_HEIGHT_PX, worse_of(five_hour, seven_day));
        }
        IconStyle::Minimal => {
            draw_dot(&mut pixmap, worse_of(five_hour, seven_day));
        }
    }

    pixmap.encode_png().expect("png encode never fails for valid pixmap")
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

fn draw_pie(pixmap: &mut Pixmap, cx: f32, cy: f32, radius: f32, stroke_width: f32, digit_height_px: f32, pct: Option<f64>) {
    // Track ring (always drawn).
    let track_path = circle_path(cx, cy, radius);
    let stroke = Stroke {
        width: stroke_width,
        ..Default::default()
    };
    let mut paint = Paint::default();
    paint.set_color(shared::track());
    paint.anti_alias = true;
    pixmap.stroke_path(&track_path, &paint, &stroke, Transform::identity(), None);

    // Arc fill (only when we have data).
    if let Some(raw) = pct {
        let clamped = raw.clamp(0.0, 99.0);
        if clamped > 0.0 {
            let circumference = 2.0 * std::f32::consts::PI * radius;
            let filled = circumference * (clamped as f32 / 100.0);
            let gap = circumference - filled;

            let arc_path = circle_path(cx, cy, radius);
            let arc_stroke = Stroke {
                width: stroke_width,
                line_cap: tiny_skia::LineCap::Butt,
                dash: StrokeDash::new(vec![filled, gap], 0.0),
                ..Default::default()
            };
            let mut arc_paint = Paint::default();
            arc_paint.set_color(shared::arc_color(raw));
            arc_paint.anti_alias = true;
            // Rotate so the dash pattern starts at 12 o'clock.
            let arc_transform = Transform::identity()
                .pre_translate(cx, cy)
                .pre_rotate(-90.0)
                .pre_translate(-cx, -cy);
            pixmap.stroke_path(&arc_path, &arc_paint, &arc_stroke, arc_transform, None);
        }
    }

    // Inscribed digits, centered both horizontally and vertically inside the ring.
    let digit_str: String = match pct {
        Some(p) => format!("{}", (p.clamp(0.0, 99.0).round()) as i64),
        None => "\u{2014}".to_string(),
    };
    let color = match pct {
        Some(_) => shared::text(),
        None => shared::text_muted(),
    };
    draw_centered_text(pixmap, &digit_str, cx, cy, digit_height_px, color);
}

/// `Minimal` style — a plain status-colored dot, no ring/track and no
/// digits. A muted gray stands in for "no data" instead of an em-dash,
/// since there's no digit slot to put one in.
fn draw_dot(pixmap: &mut Pixmap, pct: Option<f64>) {
    let color = match pct {
        Some(p) => shared::arc_color(p),
        None => shared::track(),
    };
    let path = circle_path(SINGLE_CX, SINGLE_CY, MINIMAL_DOT_R);
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
}

/// Draws `text` centered horizontally and vertically at (`cx`, `cy`).
/// `cap_height_px` is the target height of capital letters / digit tops in pixels;
/// the renderer uses the font's reported cap-height to compute the baseline so
/// the visible glyph block (cap-top → baseline) is centered on `cy`.
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
            // The em-dash and digits in JetBrains Mono are centered within
            // their advance width. Position pen_x as the LEFT edge of the
            // character's advance box; the path renders relative to its own
            // origin (which for monospace digits coincides with the box).
            let transform = Transform::from_scale(scale, scale)
                .post_translate(pen_x, baseline_y);
            pixmap.fill_path(path, &paint, FillRule::Winding, transform, None);
        }
        pen_x += advance_px;
    }
}

fn circle_path(cx: f32, cy: f32, r: f32) -> tiny_skia::Path {
    let mut pb = PathBuilder::new();
    pb.push_circle(cx, cy, r);
    pb.finish().expect("circle is a valid path")
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
    fn output_is_a_valid_88x44_png() {
        let bytes = render(Some(50.0), Some(50.0), false, IconStyle::Dual);
        assert_eq!(&bytes[..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        let (w, h, _) = decode_png(&bytes);
        assert_eq!((w, h), (88, 44));
    }

    #[test]
    fn output_has_visible_pixels_when_data_present() {
        let bytes = render(Some(50.0), Some(50.0), false, IconStyle::Dual);
        let (_, _, rgba) = decode_png(&bytes);
        let any_opaque = rgba.chunks(4).any(|p| p[3] > 0);
        assert!(any_opaque, "expected at least one opaque pixel");
    }

    #[test]
    fn output_is_almost_blank_when_no_data() {
        let bytes = render(None, None, true, IconStyle::Dual);
        let (_, _, rgba) = decode_png(&bytes);
        let opaque_count = rgba.chunks(4).filter(|p| p[3] > 200).count();
        // The track rings + em-dashes still show up; just confirm the
        // colored arcs and full-strength digits are absent.
        // Lower-bound: even fully empty has some track pixels (~80-200).
        // Upper-bound: 5h+7d filled would have ~600+ opaque pixels.
        assert!(opaque_count < 400, "no-data state should have <400 opaque pixels, got {}", opaque_count);
    }

    #[test]
    fn warn_threshold_renders_warn_color_in_left_pie() {
        // 75% on left, 0% on right. The left pie should have at least one pixel
        // matching the warn arc color (with anti-aliasing tolerance).
        let bytes = render(Some(80.0), Some(0.0), false, IconStyle::Dual);
        let (w, _, rgba) = decode_png(&bytes);
        let warn_target = (0xE8, 0x91, 0x49);
        let mut hit = false;
        for (i, px) in rgba.chunks(4).enumerate() {
            let x = (i as u32) % w;
            // Only check the left pie (x < 44).
            if x >= 44 {
                continue;
            }
            let (r, g, b, a) = (px[0], px[1], px[2], px[3]);
            if a > 200 && near(r, warn_target.0) && near(g, warn_target.1) && near(b, warn_target.2)
            {
                hit = true;
                break;
            }
        }
        assert!(hit, "expected at least one warn-colored pixel in left pie");
    }

    #[test]
    fn primary_style_uses_the_worse_of_the_two_buckets() {
        // 5h at 80% (warn), 7d at 30% (accent) — primary shows the worse one,
        // so a warn-colored pixel should appear; danger should not.
        let bytes = render(Some(80.0), Some(30.0), false, IconStyle::Primary);
        let (_, _, rgba) = decode_png(&bytes);
        let warn_target = (0xE8, 0x91, 0x49);
        let has_warn = rgba
            .chunks(4)
            .any(|p| p[3] > 200 && near(p[0], warn_target.0) && near(p[1], warn_target.1) && near(p[2], warn_target.2));
        assert!(has_warn, "expected the worse (80%) bucket's warn color to appear");
    }

    #[test]
    fn primary_style_falls_back_to_the_only_available_bucket() {
        let with_seven_day_only = render(None, Some(40.0), false, IconStyle::Primary);
        let without_data = render(None, None, true, IconStyle::Primary);
        let (_, _, a) = decode_png(&with_seven_day_only);
        let (_, _, b) = decode_png(&without_data);
        assert_ne!(a, b, "a single available bucket must still render distinctly from no-data");
    }

    #[test]
    fn primary_style_output_is_valid_88x44_png() {
        let bytes = render(Some(50.0), Some(50.0), false, IconStyle::Primary);
        let (w, h, _) = decode_png(&bytes);
        assert_eq!((w, h), (88, 44));
    }

    #[test]
    fn minimal_style_renders_no_digit_ink() {
        // Digit glyphs are painted in shared::text()/shared::text_muted().
        // Minimal must never paint either — it's a dot only.
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
        // 7d at 95% is danger; the dot should be danger-colored somewhere.
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
        assert_eq!((w, h), (88, 44));
    }

    fn near(a: u8, b: u8) -> bool {
        (a as i16 - b as i16).abs() < 12
    }
}
