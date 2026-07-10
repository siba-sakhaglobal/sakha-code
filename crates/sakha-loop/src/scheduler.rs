//! Cron/interval schedule engine. See spec "Required Capabilities" ->
//! "Cron/interval schedules" and `modules/04-loop-engineering.md`
//! "Implementation Tasks" step 2.

use async_trait::async_trait;

use sakha_core::{LoopId, SakhaError, SakhaResult};

use crate::spec::CronSpec;

/// Computes the next fire time for a `CronSpec`.
#[async_trait]
pub trait Scheduler: Send + Sync {
    async fn next_fire_time(&self, loop_id: LoopId, cron: &CronSpec) -> SakhaResult<chrono::DateTime<chrono::Utc>>;
}

/// A scheduler that always fires "now" — usable as a manual/immediate
/// trigger or as a safe fallback when no real schedule is configured.
#[derive(Debug, Default)]
pub struct ImmediateScheduler;

#[async_trait]
impl Scheduler for ImmediateScheduler {
    async fn next_fire_time(&self, _loop_id: LoopId, _cron: &CronSpec) -> SakhaResult<chrono::DateTime<chrono::Utc>> {
        Ok(sakha_core::time::now_utc())
    }
}

/// A parsed schedule expression: either a fixed interval (`@every 5m`) or a
/// cron-lite 5-field expression (`minute hour day month weekday`, each field
/// `*` or a single non-negative integer — no ranges/lists/steps). This
/// covers the common "run every N / run at fixed time" loop schedules
/// without pulling in a cron dependency outside the pre-declared workspace
/// set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedSchedule {
    /// Fire every `Duration` starting from "now" at parse time.
    Interval(std::time::Duration),
    /// Cron-lite fields: (minute, hour, day_of_month, month, day_of_week).
    /// `None` means "any" (i.e. `*`).
    CronLite { minute: Option<u32>, hour: Option<u32>, day_of_month: Option<u32>, month: Option<u32>, day_of_week: Option<u32> },
}

/// Parses a `CronSpec` string. Supported forms:
/// - `@every <n><unit>` where unit is one of `s`, `m`, `h` (e.g. `@every 30s`, `@every 5m`, `@every 1h`).
/// - A 5-field cron-lite expression: `min hour dom month dow`, each field
///   either `*` or a non-negative integer.
pub fn parse_schedule(cron: &CronSpec) -> SakhaResult<ParsedSchedule> {
    let raw = cron.0.trim();
    if let Some(rest) = raw.strip_prefix("@every ") {
        return parse_interval(rest.trim());
    }

    let fields: Vec<&str> = raw.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(SakhaError::invalid_input(
            "sakha-loop",
            format!("unsupported cron expression (expected '@every <n><unit>' or 5 whitespace-separated fields): {raw}"),
        ));
    }
    let parse_field = |s: &str| -> SakhaResult<Option<u32>> {
        if s == "*" {
            Ok(None)
        } else {
            s.parse::<u32>()
                .map(Some)
                .map_err(|e| SakhaError::invalid_input("sakha-loop", format!("bad cron field '{s}': {e}")))
        }
    };
    Ok(ParsedSchedule::CronLite {
        minute: parse_field(fields[0])?,
        hour: parse_field(fields[1])?,
        day_of_month: parse_field(fields[2])?,
        month: parse_field(fields[3])?,
        day_of_week: parse_field(fields[4])?,
    })
}

fn parse_interval(rest: &str) -> SakhaResult<ParsedSchedule> {
    let (num_str, unit) = rest.split_at(rest.len().saturating_sub(1));
    let n: u64 = num_str
        .parse()
        .map_err(|e| SakhaError::invalid_input("sakha-loop", format!("bad interval '{rest}': {e}")))?;
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        other => {
            return Err(SakhaError::invalid_input("sakha-loop", format!("unsupported interval unit '{other}' (use s/m/h)")))
        }
    };
    Ok(ParsedSchedule::Interval(std::time::Duration::from_secs(secs)))
}

/// Given a parsed cron-lite expression and a starting time, finds the next
/// minute-resolution time (searched up to ~4 years ahead) whose fields match.
/// Minute resolution is sufficient for loop scheduling (loops are not
/// expected to fire sub-minute via cron-lite; use `@every Ns` for that).
fn next_cron_lite_fire(
    from: chrono::DateTime<chrono::Utc>,
    minute: Option<u32>,
    hour: Option<u32>,
    day_of_month: Option<u32>,
    month: Option<u32>,
    day_of_week: Option<u32>,
) -> SakhaResult<chrono::DateTime<chrono::Utc>> {
    use chrono::{Datelike, Duration as ChronoDuration, Timelike};

    // Start searching from the next whole minute so we never return a time
    // in the past relative to `from`.
    let mut candidate = (from + ChronoDuration::minutes(1))
        .with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .ok_or_else(|| SakhaError::integrity("sakha-loop", "failed to normalize candidate time"))?;

    const MAX_STEPS: u32 = 60 * 24 * 366 * 4; // ~4 years of minutes.
    for _ in 0..MAX_STEPS {
        let matches = minute.is_none_or(|m| candidate.minute() == m)
            && hour.is_none_or(|h| candidate.hour() == h)
            && day_of_month.is_none_or(|d| candidate.day() == d)
            && month.is_none_or(|mo| candidate.month() == mo)
            && day_of_week.is_none_or(|dow| candidate.weekday().num_days_from_sunday() == dow);
        if matches {
            return Ok(candidate);
        }
        candidate += ChronoDuration::minutes(1);
    }
    Err(SakhaError::invalid_input("sakha-loop", "cron-lite expression never matches within search horizon"))
}

/// A real scheduler supporting `@every` intervals and 5-field cron-lite
/// expressions, per spec "Cron/interval schedules". State is per-loop so
/// repeated calls to `next_fire_time` for an interval schedule advance from
/// the last computed fire time rather than always "now + interval".
pub struct IntervalCronScheduler {
    last_fire: std::sync::Mutex<std::collections::HashMap<LoopId, chrono::DateTime<chrono::Utc>>>,
}

impl IntervalCronScheduler {
    pub fn new() -> Self {
        Self { last_fire: std::sync::Mutex::new(std::collections::HashMap::new()) }
    }
}

impl Default for IntervalCronScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Scheduler for IntervalCronScheduler {
    async fn next_fire_time(&self, loop_id: LoopId, cron: &CronSpec) -> SakhaResult<chrono::DateTime<chrono::Utc>> {
        let parsed = parse_schedule(cron)?;
        let now = sakha_core::time::now_utc();
        let base = {
            let guard = self.last_fire.lock().unwrap();
            guard.get(&loop_id).copied().unwrap_or(now)
        };
        let next = match parsed {
            ParsedSchedule::Interval(duration) => {
                let candidate = base + chrono::Duration::from_std(duration).map_err(|e| SakhaError::invalid_input("sakha-loop", e.to_string()))?;
                if candidate < now { now } else { candidate }
            }
            ParsedSchedule::CronLite { minute, hour, day_of_month, month, day_of_week } => {
                next_cron_lite_fire(base.max(now), minute, hour, day_of_month, month, day_of_week)?
            }
        };
        self.last_fire.lock().unwrap().insert(loop_id, next);
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_seconds() {
        let parsed = parse_schedule(&CronSpec("@every 30s".into())).unwrap();
        assert_eq!(parsed, ParsedSchedule::Interval(std::time::Duration::from_secs(30)));
    }

    #[test]
    fn parses_every_minutes() {
        let parsed = parse_schedule(&CronSpec("@every 5m".into())).unwrap();
        assert_eq!(parsed, ParsedSchedule::Interval(std::time::Duration::from_secs(300)));
    }

    #[test]
    fn parses_cron_lite_wildcard_fields() {
        let parsed = parse_schedule(&CronSpec("* * * * *".into())).unwrap();
        assert_eq!(
            parsed,
            ParsedSchedule::CronLite { minute: None, hour: None, day_of_month: None, month: None, day_of_week: None }
        );
    }

    #[test]
    fn parses_cron_lite_fixed_fields() {
        let parsed = parse_schedule(&CronSpec("0 9 * * 1".into())).unwrap();
        assert_eq!(
            parsed,
            ParsedSchedule::CronLite { minute: Some(0), hour: Some(9), day_of_month: None, month: None, day_of_week: Some(1) }
        );
    }

    #[test]
    fn rejects_malformed_expression() {
        assert!(parse_schedule(&CronSpec("not a schedule".into())).is_err());
    }

    #[tokio::test]
    async fn immediate_scheduler_fires_now() {
        let scheduler = ImmediateScheduler;
        let before = sakha_core::time::now_utc();
        let fire = scheduler.next_fire_time(LoopId::new(), &CronSpec("ignored".into())).await.unwrap();
        assert!(fire >= before);
    }

    #[tokio::test]
    async fn interval_scheduler_computes_future_fire_time() {
        let scheduler = IntervalCronScheduler::new();
        let loop_id = LoopId::new();
        let before = sakha_core::time::now_utc();
        let fire = scheduler.next_fire_time(loop_id, &CronSpec("@every 1h".into())).await.unwrap();
        assert!(fire >= before + chrono::Duration::minutes(59));
    }

    #[tokio::test]
    async fn interval_scheduler_advances_from_previous_fire_on_repeat_calls() {
        let scheduler = IntervalCronScheduler::new();
        let loop_id = LoopId::new();
        let cron = CronSpec("@every 1h".into());
        let first = scheduler.next_fire_time(loop_id, &cron).await.unwrap();
        let second = scheduler.next_fire_time(loop_id, &cron).await.unwrap();
        assert!(second >= first + chrono::Duration::minutes(59));
    }

    #[tokio::test]
    async fn cron_lite_scheduler_finds_next_matching_minute() {
        let scheduler = IntervalCronScheduler::new();
        let loop_id = LoopId::new();
        // Every minute (`* * * * *`) should fire within the next 2 minutes.
        let before = sakha_core::time::now_utc();
        let fire = scheduler.next_fire_time(loop_id, &CronSpec("* * * * *".into())).await.unwrap();
        assert!(fire > before);
        assert!(fire <= before + chrono::Duration::minutes(2));
    }
}
