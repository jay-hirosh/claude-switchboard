use crate::tray_icon::{self, IconStyle};
use chrono::{DateTime, Utc};
use tauri::image::Image;
use tauri::AppHandle;

/// Updates the tray icon. The icon is synthesized on every call from the
/// live percentages — no static asset paths are involved.
///
/// `five_hour_resets_at` / `seven_day_resets_at` are the absolute timestamps
/// at which each rolling window resets; the tooltip formats them as
/// "resets in Xh Ym" so the user can answer "can I keep coding?" at a glance.
pub fn set_level(
    app: &AppHandle,
    five_hour: Option<f64>,
    seven_day: Option<f64>,
    five_hour_resets_at: Option<DateTime<Utc>>,
    seven_day_resets_at: Option<DateTime<Utc>>,
    paused: bool,
    icon_style: IconStyle,
) {
    let Some(tray) = app.tray_by_id("main") else { return };

    let bytes = tray_icon::render(five_hour, seven_day, paused, icon_style);
    match Image::from_bytes(&bytes) {
        Ok(img) => { let _ = tray.set_icon(Some(img)); }
        Err(e) => tracing::warn!("tray icon decode failed, keeping previous icon: {e}"),
    }
    let _ = tray.set_icon_as_template(false);

    // Numbers live in the icon now — no separate title text alongside.
    let _ = tray.set_title(None::<&str>);
    let tooltip = tooltip(
        five_hour,
        seven_day,
        five_hour_resets_at,
        seven_day_resets_at,
        paused,
        Utc::now(),
    );

    // A global provider default means the usage bars no longer describe where
    // the user's work is going. Name it rather than let the bars mislead.
    // Sessions launched from the Providers tab get no marker — those are
    // explicit acts in visible windows, and several may run at once.
    let tooltip = match default_provider_name(app) {
        Some(name) => format!("{tooltip}\nDefault provider: {name}"),
        None => tooltip,
    };

    let _ = tray.set_tooltip(Some(tooltip));
}

/// Name of the provider currently set as the global default, if any.
fn default_provider_name(app: &AppHandle) -> Option<String> {
    use tauri::Manager;
    let state = app.try_state::<std::sync::Arc<crate::app_state::AppState>>()?;
    let default = state.db.get_default_provider().ok().flatten()?;
    Some(
        state
            .db
            .get_provider(&default.provider_id)
            .ok()
            .flatten()
            .map(|p| p.name)
            .unwrap_or_else(|| "a custom provider".to_string()),
    )
}

fn tooltip(
    five_hour: Option<f64>,
    seven_day: Option<f64>,
    five_hour_resets_at: Option<DateTime<Utc>>,
    seven_day_resets_at: Option<DateTime<Utc>>,
    paused: bool,
    now: DateTime<Utc>,
) -> String {
    if paused && five_hour.is_none() && seven_day.is_none() {
        return "Claude usage — sign-in required".into();
    }

    if five_hour.is_none() && seven_day.is_none() {
        return "Claude usage".into();
    }

    let fmt_reset = |reset: Option<DateTime<Utc>>| -> String {
        let Some(r) = reset else { return String::new() };
        let minutes = (r - now).num_minutes().max(0);
        let d = minutes / (60 * 24);
        let h = (minutes % (60 * 24)) / 60;
        let m = minutes % 60;
        let reset_str = if d > 0 {
            format!("{}d {}h", d, h)
        } else if h > 0 {
            format!("{}h {}m", h, m)
        } else {
            format!("{}m", m)
        };
        format!(" — resets in {}", reset_str)
    };

    let mut lines: Vec<String> = Vec::new();
    if let Some(p) = five_hour {
        lines.push(format!("5h: {}%{}", p.round() as i64, fmt_reset(five_hour_resets_at)));
    }
    if let Some(p) = seven_day {
        lines.push(format!("7d: {}%{}", p.round() as i64, fmt_reset(seven_day_resets_at)));
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(min_from_now: i64) -> DateTime<Utc> {
        let now: DateTime<Utc> = "2026-04-27T12:00:00Z".parse().unwrap();
        now + chrono::Duration::minutes(min_from_now)
    }
    fn now() -> DateTime<Utc> {
        "2026-04-27T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn tooltip_full_with_reset_in_hours_and_minutes() {
        let s = tooltip(Some(63.0), Some(28.0), Some(t(134)), Some(t(134 + 5 * 60)), false, now());
        assert_eq!(s, "5h: 63% — resets in 2h 14m\n7d: 28% — resets in 7h 14m");
    }

    #[test]
    fn tooltip_full_with_reset_under_an_hour() {
        let s = tooltip(Some(63.0), Some(28.0), Some(t(45)), None, false, now());
        assert_eq!(s, "5h: 63% — resets in 45m\n7d: 28%");
    }

    #[test]
    fn tooltip_clamps_negative_reset_to_zero() {
        let s = tooltip(Some(99.0), Some(50.0), Some(t(-10)), None, false, now());
        assert_eq!(s, "5h: 99% — resets in 0m\n7d: 50%");
    }

    #[test]
    fn tooltip_paused_with_no_data_shows_signin_prompt() {
        let s = tooltip(None, None, None, None, true, now());
        assert_eq!(s, "Claude usage — sign-in required");
    }

    #[test]
    fn tooltip_drops_missing_pieces() {
        // Partial data — only 7d known, no 5h reading and no reset time.
        let s = tooltip(None, Some(28.0), None, None, false, now());
        assert_eq!(s, "7d: 28%");
    }

    #[test]
    fn tooltip_unpaused_with_nothing_falls_back_to_generic() {
        let s = tooltip(None, None, None, None, false, now());
        assert_eq!(s, "Claude usage");
    }

    #[test]
    fn tooltip_both_resets_shown() {
        // 135 min → 2h 15m; 2880 min (48h) → 2d 0h
        let s = tooltip(Some(21.0), Some(50.0), Some(t(135)), Some(t(2880)), false, now());
        assert_eq!(s, "5h: 21% — resets in 2h 15m\n7d: 50% — resets in 2d 0h");
    }
}
