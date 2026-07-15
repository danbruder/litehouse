use chrono::Timelike;

/// True if, given the current Eastern time and the Eastern date
/// (`YYYY-MM-DD`) the nightly restart last completed (`None` if it has
/// never run), tonight's pass should run now. Pure and independent of
/// Docker/DB so it's directly unit-testable; the scheduler loop in
/// `commands::server::execute` calls this once per hourly tick.
pub fn should_run_now(now_eastern: chrono::DateTime<chrono_tz::Tz>, last_run_date: Option<&str>) -> bool {
    if now_eastern.hour() != 3 {
        return false;
    }
    let today = now_eastern.format("%Y-%m-%d").to_string();
    last_run_date != Some(today.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn eastern(y: i32, m: u32, d: u32, h: u32) -> chrono::DateTime<chrono_tz::Tz> {
        chrono_tz::America::New_York
            .with_ymd_and_hms(y, m, d, h, 0, 0)
            .unwrap()
    }

    #[test]
    fn should_run_at_3am_when_never_run_before() {
        assert!(should_run_now(eastern(2026, 7, 15, 3), None));
    }

    #[test]
    fn should_not_run_outside_the_3am_hour() {
        assert!(!should_run_now(eastern(2026, 7, 15, 2), None));
        assert!(!should_run_now(eastern(2026, 7, 15, 4), None));
        assert!(!should_run_now(eastern(2026, 7, 15, 0), None));
    }

    #[test]
    fn should_not_run_twice_in_the_same_eastern_day() {
        assert!(!should_run_now(eastern(2026, 7, 15, 3), Some("2026-07-15")));
    }

    #[test]
    fn should_run_again_the_next_day() {
        assert!(should_run_now(eastern(2026, 7, 16, 3), Some("2026-07-15")));
    }
}
