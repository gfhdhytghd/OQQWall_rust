use crate::config::TimeWindow;
use crate::ids::TimestampMs;

pub fn compute_not_before(
    now_ms: TimestampMs,
    delay_until_ms: Option<TimestampMs>,
    send_windows: &[TimeWindow],
    min_interval_ms: TimestampMs,
    last_send_ms: Option<TimestampMs>,
    queue_depth: usize,
    max_queue: usize,
    tz_offset_minutes: i32,
) -> TimestampMs {
    let mut candidate = delay_until_ms.unwrap_or(now_ms).max(now_ms);

    candidate = apply_send_window(candidate, send_windows, tz_offset_minutes);

    if let Some(last_send) = last_send_ms {
        let next_allowed = last_send.saturating_add(min_interval_ms);
        if candidate < next_allowed {
            candidate = next_allowed;
        }
    }

    candidate = apply_send_window(candidate, send_windows, tz_offset_minutes);

    if queue_depth >= max_queue && max_queue > 0 {
        let overflow_backoff = min_interval_ms.max(0);
        let next_window = next_window_start(candidate, send_windows, tz_offset_minutes);
        candidate = next_window.saturating_add(overflow_backoff);
    }

    apply_send_window(candidate, send_windows, tz_offset_minutes)
}

pub fn minute_of_day(now_ms: TimestampMs, tz_offset_minutes: i32) -> u16 {
    let adjusted = now_ms.saturating_add((tz_offset_minutes as i64) * 60_000);
    let minutes = ((adjusted / 60_000) % 1440 + 1440) % 1440;
    minutes as u16
}

pub fn day_index(now_ms: TimestampMs, tz_offset_minutes: i32) -> i64 {
    let adjusted = now_ms.saturating_add((tz_offset_minutes as i64) * 60_000);
    adjusted.div_euclid(86_400_000)
}

fn apply_send_window(
    candidate_ms: TimestampMs,
    send_windows: &[TimeWindow],
    tz_offset_minutes: i32,
) -> TimestampMs {
    if send_windows.is_empty() {
        return candidate_ms;
    }
    if is_within_window(candidate_ms, send_windows, tz_offset_minutes) {
        return candidate_ms;
    }
    next_window_start(candidate_ms, send_windows, tz_offset_minutes)
}

fn is_within_window(
    candidate_ms: TimestampMs,
    send_windows: &[TimeWindow],
    tz_offset_minutes: i32,
) -> bool {
    let minute = minute_of_day(candidate_ms, tz_offset_minutes);
    send_windows
        .iter()
        .any(|win| win.start_minute <= minute && minute <= win.end_minute)
}

pub fn next_window_start(
    candidate_ms: TimestampMs,
    send_windows: &[TimeWindow],
    tz_offset_minutes: i32,
) -> TimestampMs {
    if send_windows.is_empty() {
        return candidate_ms;
    }
    let minute = minute_of_day(candidate_ms, tz_offset_minutes);
    let day_start = candidate_ms
        .saturating_sub((minute as i64) * 60_000)
        .saturating_sub((tz_offset_minutes as i64) * 60_000);

    let mut sorted = send_windows.to_vec();
    sorted.sort_by_key(|win| win.start_minute);

    for win in &sorted {
        if minute <= win.start_minute {
            let delta = (win.start_minute - minute) as i64 * 60_000;
            return candidate_ms.saturating_add(delta);
        }
    }

    let first = sorted[0].start_minute as i64 * 60_000;
    day_start.saturating_add(86_400_000 + first)
}
