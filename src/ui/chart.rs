//! Minimal server-rendered SVG line/band charts for the admin UI. No
//! client-side JS beyond the HTMX already vendored alongside this module —
//! every chart is a plain inline `<svg>` string built from a series of
//! optional values (a `None` is a tick where sampling failed or hadn't
//! started yet, and breaks the line rather than interpolating across it).

const WIDTH: f64 = 640.0;
const HEIGHT: f64 = 120.0;
const PAD: f64 = 4.0;

#[derive(Debug, Clone, Copy)]
pub enum ChartUnit {
    Percent,
    Bytes,
}

fn format_label(value: f64, unit: ChartUnit) -> String {
    match unit {
        ChartUnit::Percent => format!("{:.1}%", value),
        ChartUnit::Bytes => format_bytes(value as i64),
    }
}

/// Render a single line chart. Returns a placeholder message instead of an
/// empty `<svg>` when the series has no numeric points at all.
pub fn line_chart(points: &[Option<f64>], unit: ChartUnit) -> String {
    let values: Vec<f64> = points.iter().filter_map(|p| *p).collect();
    if values.is_empty() {
        return "<p class=\"muted chart-empty\">no data yet</p>".to_string();
    }

    let max_v = values.iter().cloned().fold(f64::MIN, f64::max).max(1.0);
    let path = build_path(points, 0.0, max_v);
    let current = points.iter().rev().find_map(|p| *p);
    let label = current.map(|v| format_label(v, unit)).unwrap_or_else(|| "—".to_string());

    format!(
        r#"<div class="chart"><svg viewBox="0 0 {w} {h}" preserveAspectRatio="none" class="chart-svg">{path}</svg><span class="chart-label">{label}</span></div>"#,
        w = WIDTH,
        h = HEIGHT,
        path = path,
    )
}

/// Render an average line with a translucent min/max band behind it (used
/// for the 30-day hourly-rollup view).
pub fn band_chart(avg: &[Option<f64>], min: &[Option<f64>], max: &[Option<f64>], unit: ChartUnit) -> String {
    let all_values: Vec<f64> = max.iter().chain(avg.iter()).filter_map(|p| *p).collect();
    if all_values.is_empty() {
        return "<p class=\"muted chart-empty\">no data yet</p>".to_string();
    }
    let max_v = all_values.iter().cloned().fold(f64::MIN, f64::max).max(1.0);

    let band = build_band_path(min, max, 0.0, max_v);
    let avg_path = build_path(avg, 0.0, max_v);
    let current = avg.iter().rev().find_map(|p| *p);
    let label = current
        .map(|v| format!("{} avg", format_label(v, unit)))
        .unwrap_or_else(|| "—".to_string());

    format!(
        r#"<div class="chart"><svg viewBox="0 0 {w} {h}" preserveAspectRatio="none" class="chart-svg">{band}{avg_path}</svg><span class="chart-label">{label}</span></div>"#,
        w = WIDTH,
        h = HEIGHT,
        band = band,
        avg_path = avg_path,
    )
}

fn x_for(i: usize, len: usize) -> f64 {
    if len <= 1 {
        return PAD;
    }
    PAD + (i as f64 / (len - 1) as f64) * (WIDTH - 2.0 * PAD)
}

fn y_for(v: f64, min_v: f64, max_v: f64) -> f64 {
    let span = (max_v - min_v).max(f64::EPSILON);
    let frac = ((v - min_v) / span).clamp(0.0, 1.0);
    HEIGHT - PAD - frac * (HEIGHT - 2.0 * PAD)
}

/// One `<polyline>` per contiguous run of `Some` values — a run of length 1
/// (a lone point surrounded by gaps) is dropped, since a polyline needs at
/// least two points to draw anything.
fn build_path(points: &[Option<f64>], min_v: f64, max_v: f64) -> String {
    let len = points.len();
    let mut segments = String::new();
    let mut current_segment: Vec<String> = Vec::new();

    for (i, p) in points.iter().enumerate() {
        match p {
            Some(v) => {
                let x = x_for(i, len);
                let y = y_for(*v, min_v, max_v);
                current_segment.push(format!("{:.1},{:.1}", x, y));
            }
            None => {
                flush_segment(&mut current_segment, &mut segments);
            }
        }
    }
    flush_segment(&mut current_segment, &mut segments);
    segments
}

fn flush_segment(segment: &mut Vec<String>, out: &mut String) {
    if segment.len() >= 2 {
        out.push_str(&format!(
            r#"<polyline points="{}" fill="none" stroke="currentColor" stroke-width="1.5" />"#,
            segment.join(" ")
        ));
    }
    segment.clear();
}

/// Filled polygon between `min` and `max`, only over indices where both are
/// present.
fn build_band_path(min: &[Option<f64>], max: &[Option<f64>], min_v: f64, max_v: f64) -> String {
    let len = min.len().min(max.len());
    let mut top: Vec<String> = Vec::new();
    let mut bottom: Vec<String> = Vec::new();

    for i in 0..len {
        if let (Some(lo), Some(hi)) = (min[i], max[i]) {
            let x = x_for(i, len);
            top.push(format!("{:.1},{:.1}", x, y_for(hi, min_v, max_v)));
            bottom.push(format!("{:.1},{:.1}", x, y_for(lo, min_v, max_v)));
        }
    }
    if top.len() < 2 {
        return String::new();
    }
    bottom.reverse();
    let mut all_points = top;
    all_points.extend(bottom);
    format!(
        r#"<polygon points="{}" fill="currentColor" opacity="0.15" />"#,
        all_points.join(" ")
    )
}

/// Humanize a byte count (`1.2 GB`, `340 MB`, `512 B`).
pub fn format_bytes(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes <= 0 {
        return "0 B".to_string();
    }
    let mut value = bytes as f64;
    let mut unit_idx = 0;
    while value >= 1024.0 && unit_idx < UNITS.len() - 1 {
        value /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.1} {}", value, UNITS[unit_idx])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_scales_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(1_500_000_000), "1.4 GB");
    }

    #[test]
    fn line_chart_empty_series_shows_placeholder() {
        let out = line_chart(&[None, None], ChartUnit::Percent);
        assert!(out.contains("no data yet"));
    }

    #[test]
    fn line_chart_renders_polyline_and_current_label() {
        let out = line_chart(&[Some(10.0), Some(20.0), Some(15.0)], ChartUnit::Percent);
        assert!(out.contains("<polyline"));
        assert!(out.contains("15.0%"));
    }

    #[test]
    fn line_chart_drops_isolated_single_point_segments() {
        let out = line_chart(&[Some(10.0), None, Some(20.0)], ChartUnit::Percent);
        assert!(!out.contains("<polyline"));
    }

    #[test]
    fn band_chart_empty_shows_placeholder() {
        let out = band_chart(&[None], &[None], &[None], ChartUnit::Bytes);
        assert!(out.contains("no data yet"));
    }

    #[test]
    fn band_chart_renders_polygon_and_avg_line() {
        let avg = vec![Some(50.0), Some(60.0)];
        let min = vec![Some(40.0), Some(45.0)];
        let max = vec![Some(60.0), Some(70.0)];
        let out = band_chart(&avg, &min, &max, ChartUnit::Percent);
        assert!(out.contains("<polygon"));
        assert!(out.contains("<polyline"));
        assert!(out.contains("60.0% avg"));
    }
}
