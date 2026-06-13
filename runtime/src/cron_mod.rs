// Fluxon cron battery — scheduled background tasks.
//
// Language API (docs):
//   cron.on 0 * * * * check_prices     # minute hour day month weekday; named function
//   cron.on 30 9 * * * \-> ...          # inline lambda (no parameters)
//   cron.run                            # if there is NO server: keep the process alive
//
// Syntax: a standard 5-field Unix cron expression. The parser reads it UNQUOTED
// (`*` is not multiplication) — when the `cron.on` callee is seen, parser.rs collects
// the special 5 fields into a str. The quoted form (`cron.on "0 * * * *" f`) also works.
//
// Model (user decision): `cron.on` NEVER blocks — like `http.on`/`ws.on` it only
// registers and starts the scheduler background thread (once). If there is another
// blocking process (`http.serve`/`ws.serve`), cron keeps running in the background. For
// a cron-only script, `cron.run` takes over the process.
//
// Scheduler — a plain `std::thread` + sleep loop (NOT tokio): cron handlers are
// synchronous tree-walking, so async is not needed. Each handler is called via `apply`
// with no arguments; an error does not kill the server (stderr diagnostics like ws fire_handler).

use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use chrono::{Timelike, Utc};
use cron::Schedule;
use parking_lot::Mutex;

use crate::interp::{Flow, Interp};
use crate::value::Value;

// A single registered task: the parsed schedule + the handler to call.
struct CronJob {
    schedule: Schedule,
    handler: Value,
}

// cron battery state — one per process (an Arc inside Interp). Top-level code
// (`cron.on`) fills `jobs`; the scheduler background thread reads it.
pub struct CronState {
    // Registered tasks. `cron.on` pushes, the scheduler iterates.
    jobs: Mutex<Vec<CronJob>>,
    // Marker so the scheduler thread starts ONCE (idempotent start).
    started: OnceLock<()>,
}

impl CronState {
    pub fn new() -> Self {
        CronState {
            jobs: Mutex::new(Vec::new()),
            started: OnceLock::new(),
        }
    }
}

impl Default for CronState {
    fn default() -> Self {
        Self::new()
    }
}

// `cron` expression (str) -> Schedule.
//
// Fluxon uses the standard 5-field Unix format (minute hour day month weekday), while
// the `cron` crate expects 6-7 fields (with seconds) — so we prepend "0 " (seconds=0).
// Another mismatch: in Unix cron the weekday `0`=Sunday (and `7` is also Sunday), but the
// `cron` crate only accepts `1-7`/`SUN-SAT` (`0` is an error). So we turn each lone `0` in
// the weekday field into `7` — that way `0 18 * * 0` (Sunday 18:00) works like standard
// Unix.
fn parse_schedule(expr: &str) -> Result<Schedule, Flow> {
    let trimmed = expr.trim();
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    // For 5 fields (standard Unix) prepend the seconds field and normalize the weekday.
    // For any other field count we leave it unchanged (the crate validates it itself).
    let normalized = if fields.len() == 5 {
        let weekday = normalize_weekday(fields[4]);
        format!(
            "0 {} {} {} {} {weekday}",
            fields[0], fields[1], fields[2], fields[3]
        )
    } else {
        trimmed.to_string()
    };
    Schedule::from_str(&normalized)
        .map_err(|e| Flow::err(format!("cron.on: invalid cron expression '{expr}': {e}")))
}

// Unix weekday `0`(Sunday) -> cron crate `7`(Sunday). Splits the field by `,` (list) and
// turns members that are exactly `0` into `7` (common cases like `* * * * 0` and
// `* * * * 0,6`). A `0` at the START of a range (`0-2`) would become an inverted `7-2` in
// the crate, so we expand the range as `7` + the remaining part (`0-2` -> `7,1-2`;
// `0-0` -> `7`). Numbers like `10`, `20` are untouched.
fn normalize_weekday(field: &str) -> String {
    field
        .split(',')
        .map(normalize_weekday_member)
        .collect::<Vec<_>>()
        .join(",")
}

// Normalizes a single list member (a lone value or a range).
fn normalize_weekday_member(part: &str) -> String {
    // Lone `0` -> `7`.
    if part == "0" {
        return "7".to_string();
    }
    // Range `A-B`: we only specially expand the A==0 case.
    if let Some((a, b)) = part.split_once('-')
        && a == "0"
    {
        // `0-0` -> Sunday; `0-N` -> Sunday (7) + Monday..N (1-N).
        return if b == "0" {
            "7".to_string()
        } else {
            format!("7,1-{b}")
        };
    }
    part.to_string()
}

impl Interp {
    // cron.<func> calls.
    pub fn cron_dispatch(self: &Arc<Self>, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            "on" => self.cron_on(args),
            "run" => self.cron_run(args),
            _ => Err(Flow::err(format!("cron module has no function '{func}'"))),
        }
    }

    // cron.on <expr> <handler> — registers a task and starts the scheduler.
    // Does not block (like http.on). The background thread starts on the first `cron.on`.
    fn cron_on(self: &Arc<Self>, args: Vec<Value>) -> Result<Value, Flow> {
        let expr = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => {
                return Err(Flow::err(
                    "cron.on: 1st argument must be a cron expression (e.g. 0 * * * *)",
                ));
            }
        };
        let handler = match args.get(1) {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => return Err(Flow::err("cron.on: 2nd argument must be a handler (fn)")),
        };
        let schedule = parse_schedule(&expr)?;
        self.cron.jobs.lock().push(CronJob { schedule, handler });
        // Start the scheduler background thread (once). cron.on does not block.
        self.start_scheduler();
        Ok(Value::Nil)
    }

    // cron.run — signals that the process should be kept alive (does NOT block immediately).
    // The scheduler already runs on a background thread; cron.run only sets the "do not
    // exit the program once top-level finishes" flag. This is DEFERRED like http.serve/
    // ws.serve: Cron is added to `pending_servers`, and once top-level finishes
    // `run_pending` keeps it alive. That way `cron.run` + `http.serve` work together in any
    // order — previously cron.run's `loop { sleep }` blocked any serve that followed it.
    fn cron_run(self: &Arc<Self>, _args: Vec<Value>) -> Result<Value, Flow> {
        self.start_scheduler(); // even if there was no cron.on at all (no-op jobs)
        self.pending_servers
            .lock()
            .unwrap()
            .push(crate::serve_mod::PendingServer::Cron);
        Ok(Value::Nil)
    }

    // Starts the scheduler background thread once.
    //
    // IMPORTANT: `freeze_globals` is NOT called here. `cron.on` is called in the MIDDLE
    // of top-level code (more global bindings may follow it) — if we froze at that point,
    // later global variables would not make it into the snapshot and referencing them
    // would give "unknown name". The scheduler thread reads globals through the RwLock
    // (lookup supports the unfrozen state); since cron runs once a minute, that slowness
    // is negligible. If `http.serve`/`ws.serve` is called later, THEY freeze, and cron
    // then reads from the frozen snapshot too.
    fn start_scheduler(self: &Arc<Self>) {
        // OnceLock::set succeeds only the first time — a second cron.on passes silently.
        if self.cron.started.set(()).is_err() {
            return;
        }
        let interp = self.clone();
        std::thread::spawn(move || run_scheduler(interp));
    }
}

// Scheduler loop: wakes at each minute boundary and fires the tasks that match the
// current minute. Since 5-field cron has minute granularity, a per-minute granularity is
// enough.
fn run_scheduler(interp: Arc<Interp>) {
    // The last minute fired (epoch minute) — tracked per job to avoid firing twice within
    // the same minute.
    loop {
        // Sleep until the start of the next minute (we check when the current second is 0).
        let now = Utc::now();
        let secs_into_minute = now.second();
        let sleep_secs = 60u64.saturating_sub(secs_into_minute as u64).max(1);
        std::thread::sleep(Duration::from_secs(sleep_secs));

        // Take the start of the current minute (second=0) as the reference point.
        let tick = Utc::now().with_second(0).and_then(|t| t.with_nanosecond(0));
        let Some(tick) = tick else { continue };

        // Per job: if the next run after the previous minute lands exactly on this minute,
        // fire it.
        let jobs = interp.cron.jobs.lock();
        for job in jobs.iter() {
            // `after(tick - 1min)` gives the next run >= tick, and it matches when == tick.
            let prev = tick - chrono::Duration::minutes(1);
            if let Some(next) = job.schedule.after(&prev).next()
                && next == tick
            {
                let interp = interp.clone();
                let handler = job.handler.clone();
                // Call the handler on a separate thread — if it runs long it does not block
                // the scheduler loop (the next minute is checked on time).
                std::thread::spawn(move || {
                    if let Err(flow) = interp.apply(handler, vec![]) {
                        eprintln!("cron handler error: {}", flow_msg(&flow));
                    }
                });
            }
        }
    }
}

fn flow_msg(flow: &Flow) -> String {
    match flow {
        Flow::Fail { message, .. } => message.clone(),
        Flow::Error(e) => e.clone(),
        _ => "skip/stop/return".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standart_5_maydon_parse() {
        // At the top of every hour.
        assert!(parse_schedule("0 * * * *").is_ok());
        // Every 15 minutes.
        assert!(parse_schedule("*/15 * * * *").is_ok());
        // Weekdays 09:30.
        assert!(parse_schedule("30 9 * * 1-5").is_ok());
        // List.
        assert!(parse_schedule("0 0,12 * * *").is_ok());
        // Unix weekday 0 = Sunday (the cron crate rejects 0, we turn it into 7).
        assert!(parse_schedule("0 18 * * 0").is_ok());
        // Sunday as a range/list member.
        assert!(parse_schedule("0 0 * * 0,3").is_ok());
        assert!(parse_schedule("0 0 * * 0-2").is_ok());
    }

    #[test]
    fn hafta_kuni_0_yakshanba() {
        // `* * * * 0` (Unix Sunday) and `* * * * 7` (crate Sunday) must fall on the same
        // day — asserts that normalize_weekday 0 -> 7 works correctly.
        // Flow does not derive Debug -> match instead of expect().
        let (Ok(s0), Ok(s7)) = (parse_schedule("0 12 * * 0"), parse_schedule("0 12 * * 7")) else {
            panic!("sunday 0/7 should have parsed");
        };
        let now = Utc::now();
        let next0 = s0.after(&now).next();
        let next7 = s7.after(&now).next();
        assert_eq!(next0, next7, "0 and 7 should fall on the same sunday");
    }

    #[test]
    fn notogri_ifoda_xato() {
        // There is no minute 99.
        assert!(parse_schedule("99 * * * *").is_err());
        // Empty.
        assert!(parse_schedule("").is_err());
        // Missing fields.
        assert!(parse_schedule("* *").is_err());
    }

    #[test]
    fn keyingi_run_hisoblanadi() {
        // A schedule that runs every minute — the next run must be after now.
        // (Flow does not derive Debug, so ok_or instead of unwrap().)
        let sched = match parse_schedule("* * * * *") {
            Ok(s) => s,
            Err(_) => panic!("'* * * * *' should have parsed"),
        };
        let now = Utc::now();
        let next = sched.after(&now).next();
        assert!(next.is_some());
        assert!(next.unwrap() > now);
    }
}
