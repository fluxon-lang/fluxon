// Fluxon queue battery — background jobs.
//
// Language API (docs):
//   queue.push "send" {ph:p body:t}            # enqueues a job (name + payload map)
//   queue.on "send" \job -> tools.send job.ph job.body   # handler for jobs of this name
//
// Philosophy (spec): so a webhook can respond fast, you hand heavy work off to the
// background. `queue.push` returns immediately (does not block); the job runs on the
// background worker thread.
//
// Model (user decision):
//   - ONE worker thread, FIFO order. Jobs run sequentially — order is guaranteed,
//     and the thread does not blow up under bursty load. (A background thread like
//     `cron`, but it wakes on the queue rather than on time.)
//   - If `queue.push` is called for a job whose handler is not yet registered, the
//     job WAITS in the queue. In Fluxon top-level code, `queue.push` may be written
//     before `queue.on` — the worker sleeps on the Condvar and wakes to run the job
//     when `queue.on` arrives (NO busy-loop, issue #105). A job without a handler does
//     not block other jobs that DO have one.
//   - When top-level code finishes, `queue_wait_drain` waits for the queue to empty —
//     jobs are not silently lost (issue #105). Jobs whose handler was never registered
//     are dropped here with a warning (they cannot be run — waiting would be forever).
//
// Worker — a plain `std::thread` + `Condvar` (NOT tokio): handlers are synchronous
// tree-walking, so async is not needed. The handler is called via `apply` with a single
// `job` map argument; an error does not kill the worker (stderr diagnostics like cron/ws fire).

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::OnceLock;

use parking_lot::{Condvar, Mutex};

use crate::interp::{Flow, Interp};
use crate::value::Value;

// A single queued job: which handler (name) and what data (payload).
struct Job {
    name: String,
    payload: Value,
}

// queue battery state — one per process (an Arc inside Interp). Top-level code
// (`queue.push`/`queue.on`) fills it; the background worker thread reads it.
pub struct QueueState {
    // FIFO queue + name->handler registry. Under one Mutex — so the worker sees both
    // together (it takes a job and checks whether a handler exists for its name under
    // a single lock). The Condvar tells the worker a new job/handler has arrived.
    inner: Mutex<QueueInner>,
    not_empty: Condvar,
    // Signals that the worker has finished one job — `queue_wait_drain` uses this to
    // wait for the queue to empty (issue #105: jobs must not be silently lost).
    idle: Condvar,
    // Marker so the worker thread starts ONCE (idempotent start).
    started: OnceLock<()>,
}

struct QueueInner {
    queue: VecDeque<Job>,
    handlers: HashMap<String, Value>,
    // Whether the worker is currently running a job — drain must wait for the running
    // job to finish even when the queue is empty.
    busy: bool,
    // So the stderr warning about a handler-less name is emitted ONCE.
    warned: HashSet<String>,
}

impl QueueState {
    pub fn new() -> Self {
        QueueState {
            inner: Mutex::new(QueueInner {
                queue: VecDeque::new(),
                handlers: HashMap::new(),
                busy: false,
                warned: HashSet::new(),
            }),
            not_empty: Condvar::new(),
            idle: Condvar::new(),
            started: OnceLock::new(),
        }
    }
}

impl Default for QueueState {
    fn default() -> Self {
        Self::new()
    }
}

impl Interp {
    // queue.<func> calls.
    pub fn queue_dispatch(self: &Arc<Self>, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            "push" => self.queue_push(args),
            "on" => self.queue_on(args),
            _ => Err(Flow::err(format!("queue module has no function '{func}'"))),
        }
    }

    // queue.push <name> <payload> — enqueues a job and starts the worker.
    // Does not block: the job is enqueued immediately and the worker runs it in the
    // background. Payload is optional (Nil if omitted) — the handler takes a single
    // `job` argument.
    fn queue_push(self: &Arc<Self>, args: Vec<Value>) -> Result<Value, Flow> {
        let name = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => {
                return Err(Flow::err(
                    "queue.push: 1st argument must be the job name (str)",
                ));
            }
        };
        // Payload is optional. If omitted, Nil rather than an empty map — the handler
        // decides for itself.
        let payload = args.get(1).cloned().unwrap_or(Value::Nil);
        {
            let mut inner = self.queue.inner.lock();
            inner.queue.push_back(Job { name, payload });
        }
        // Start the background worker thread (once) and wake it if it is sleeping.
        self.start_worker();
        self.queue.not_empty.notify_one();
        Ok(Value::Nil)
    }

    // queue.on <name> <handler> — registers a handler for jobs of this name.
    // Does not block. There may be jobs waiting for this handler (push written before
    // on) — we wake the worker, which can now find the handler and run them.
    fn queue_on(self: &Arc<Self>, args: Vec<Value>) -> Result<Value, Flow> {
        let name = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => {
                return Err(Flow::err(
                    "queue.on: 1st argument must be the job name (str)",
                ));
            }
        };
        let handler = match args.get(1) {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => {
                return Err(Flow::err("queue.on: 2nd argument must be a handler (fn)"));
            }
        };
        {
            let mut inner = self.queue.inner.lock();
            inner.handlers.insert(name, handler);
        }
        // Make sure the worker is running (also for a queue-only script) and have it
        // re-examine the waiting jobs now that a handler exists.
        self.start_worker();
        self.queue.not_empty.notify_one();
        Ok(Value::Nil)
    }

    // Starts the background worker thread once.
    //
    // IMPORTANT (same as cron): `freeze_globals` is NOT called here.
    // `queue.push`/`queue.on` are called in the MIDDLE of top-level code — if we froze
    // at that point, later global variables would not make it into the snapshot and
    // referencing them would give "unknown name". The worker reads globals through the
    // RwLock. If `http.serve`/`ws.serve` is called later, THEY freeze, and the worker
    // then reads from the frozen snapshot too.
    fn start_worker(self: &Arc<Self>) {
        // OnceLock::set succeeds only the first time — subsequent calls pass silently.
        if self.queue.started.set(()).is_err() {
            return;
        }
        let interp = self.clone();
        std::thread::spawn(move || run_worker(interp));
    }

    // Called when top-level code finishes (interp.rs `run`): blocks until the runnable
    // jobs in the queue and the running job are done — so background jobs are not
    // silently lost on script exit (issue #105). Jobs whose handler was never registered
    // cannot be run — we drop them with a warning (otherwise the wait would be forever).
    // New jobs pushed from inside a handler are also waited on here (the predicate is
    // re-checked on each wakeup).
    pub fn queue_wait_drain(&self) {
        let mut inner = self.queue.inner.lock();
        loop {
            let runnable = inner
                .queue
                .iter()
                .any(|j| inner.handlers.contains_key(&j.name));
            if !runnable && !inner.busy {
                break;
            }
            self.queue.idle.wait(&mut inner);
        }
        for job in inner.queue.drain(..) {
            eprintln!(
                "queue: job '{}' not run — handler not registered (process ended)",
                job.name
            );
        }
    }
}

// Worker loop: takes and calls the FIRST job whose handler is registered. A job whose
// handler is not yet present stays in the queue (not the old "re-enqueue + sleep 50ms"
// busy-loop — issue #105): the worker sleeps on the Condvar and `queue.on`/`queue.push`
// wake it. A handler-less job does not block jobs that have a handler; FIFO is preserved
// within a name (jobs of one name share the same handler state — selecting does not break
// the order).
fn run_worker(interp: Arc<Interp>) {
    loop {
        let (job, handler) = {
            let mut inner = interp.queue.inner.lock();
            loop {
                // To borrow the fields separately through the MutexGuard.
                let st = &mut *inner;
                if let Some(pos) = st
                    .queue
                    .iter()
                    .position(|j| st.handlers.contains_key(&j.name))
                {
                    // pos was just found — remove is guaranteed to return Some.
                    let Some(job) = st.queue.remove(pos) else {
                        continue;
                    };
                    let handler = st.handlers[&job.name].clone();
                    // Make drain wait for this job even if the queue is empty.
                    st.busy = true;
                    break (job, handler);
                }
                // No runnable job. Emit a diagnostic ONCE for the names waiting without
                // a handler (issue #105).
                let unknown: Vec<String> = st
                    .queue
                    .iter()
                    .map(|j| j.name.clone())
                    .filter(|n| !st.warned.contains(n))
                    .collect();
                for name in unknown {
                    eprintln!(
                        "queue: no handler for '{}' (queue.on not called) — job waiting in queue",
                        name
                    );
                    st.warned.insert(name);
                }
                interp.queue.not_empty.wait(&mut inner);
            }
        };

        // Call the handler on THIS thread with a single `job` (payload) argument — this
        // is what guarantees FIFO and sequential execution. An error does not kill the
        // worker.
        if let Err(flow) = interp.apply(handler, vec![job.payload]) {
            eprintln!("queue handler '{}' error: {}", job.name, flow_msg(&flow));
        }
        // The job is done — wake drain if it is waiting.
        interp.queue.inner.lock().busy = false;
        interp.queue.idle.notify_all();
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
    use std::time::Duration;

    use super::*;

    // queue.push must work without a payload too (payload becomes Nil).
    #[test]
    fn push_payloadsiz_nil() {
        let interp = Arc::new(Interp::new());
        let r = interp.queue_push(vec![Value::Str("ish".into())]);
        assert!(r.is_ok());
        let inner = interp.queue.inner.lock();
        assert_eq!(inner.queue.len(), 1);
        match &inner.queue.front() {
            Some(job) => assert!(matches!(job.payload, Value::Nil)),
            None => panic!("the job should have been enqueued"),
        }
    }

    // queue.push errors if the 1st argument is not a str.
    #[test]
    fn push_nom_str_bolmasa_xato() {
        let interp = Arc::new(Interp::new());
        assert!(interp.queue_push(vec![Value::Int(5)]).is_err());
        assert!(interp.queue_push(vec![]).is_err());
    }

    // queue.on errors if the handler is not an fn; errors if the name is not a str.
    #[test]
    fn on_argument_tekshiruvi() {
        let interp = Arc::new(Interp::new());
        // No handler.
        assert!(interp.queue_on(vec![Value::Str("ish".into())]).is_err());
        // Handler is not an fn.
        assert!(
            interp
                .queue_on(vec![Value::Str("ish".into()), Value::Int(1)])
                .is_err()
        );
        // Name is not a str.
        assert!(interp.queue_on(vec![Value::Int(1)]).is_err());
    }

    // queue.on adds the handler to the registry.
    #[test]
    fn on_handler_royxatga_oladi() {
        let interp = Arc::new(Interp::new());
        let handler = Value::Native(Arc::new(crate::value::NativeFn {
            name: "noop".into(),
            func: Box::new(|_| Ok(Value::Nil)),
        }));
        let r = interp.queue_on(vec![Value::Str("ish".into()), handler]);
        assert!(r.is_ok());
        let inner = interp.queue.inner.lock();
        assert!(inner.handlers.contains_key("ish"));
    }

    // An unknown function in the queue module returns an error.
    #[test]
    fn nomalum_funksiya_xato() {
        let interp = Arc::new(Interp::new());
        assert!(interp.queue_dispatch("yoq", vec![]).is_err());
    }

    // End-to-end: queue.on registers a handler, queue.push enqueues a job, and the
    // background worker thread ACTUALLY calls the handler (with the payload). The native
    // handler increments an `AtomicUsize` — the main thread observes the job through that
    // counter. This directly exercises the Condvar/worker flow.
    #[test]
    fn worker_handlerni_haqiqatan_chaqiradi() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let interp = Arc::new(Interp::new());
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let handler = Value::Native(Arc::new(crate::value::NativeFn {
            name: "inc".into(),
            func: Box::new(move |args| {
                // Also assert the payload reached us (we expect Int(7)).
                if matches!(args.first(), Some(Value::Int(7))) {
                    c.fetch_add(1, Ordering::SeqCst);
                }
                Ok(Value::Nil)
            }),
        }));
        // The handler is registered first, then 3 jobs are pushed.
        // (Flow does not derive Debug -> we assert with is_ok() instead of expect().)
        assert!(
            interp
                .queue_on(vec![Value::Str("ish".into()), handler])
                .is_ok()
        );
        for _ in 0..3 {
            assert!(
                interp
                    .queue_push(vec![Value::Str("ish".into()), Value::Int(7)])
                    .is_ok()
            );
        }

        // Wait for the background worker thread to run all 3 jobs (poll, max ~2s).
        for _ in 0..200 {
            if counter.load(Ordering::SeqCst) == 3 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            counter.load(Ordering::SeqCst),
            3,
            "worker should have run all 3 jobs"
        );
    }

    // Case where the handler is registered AFTER the push: the job waits in the queue,
    // and the worker runs it when queue.on arrives (order-independent).
    #[test]
    fn push_oldin_on_keyin_kutadi() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let interp = Arc::new(Interp::new());
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        // First push — no handler yet, the job waits in the queue.
        assert!(
            interp
                .queue_push(vec![Value::Str("kech".into()), Value::Nil])
                .is_ok()
        );

        // The worker must still not have run it after a short wait (no handler).
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "no handler — should not run"
        );

        // Now the handler arrives — the waiting job should run.
        let handler = Value::Native(Arc::new(crate::value::NativeFn {
            name: "inc".into(),
            func: Box::new(move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(Value::Nil)
            }),
        }));
        assert!(
            interp
                .queue_on(vec![Value::Str("kech".into()), handler])
                .is_ok()
        );

        for _ in 0..200 {
            if counter.load(Ordering::SeqCst) == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "the waiting job should run once the handler arrives"
        );
    }

    // Issue #105: drain waits for the queued jobs to FINISH — by the time it returns the
    // handler has already run (we verify without polling/races).
    #[test]
    fn drain_ishlar_tugashini_kutadi() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let interp = Arc::new(Interp::new());
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let handler = Value::Native(Arc::new(crate::value::NativeFn {
            name: "sekin".into(),
            func: Box::new(move |_| {
                // Deliberately slow handler — without drain waiting, the counter would not
                // reach 3.
                std::thread::sleep(Duration::from_millis(20));
                c.fetch_add(1, Ordering::SeqCst);
                Ok(Value::Nil)
            }),
        }));
        assert!(
            interp
                .queue_on(vec![Value::Str("ish".into()), handler])
                .is_ok()
        );
        for _ in 0..3 {
            assert!(interp.queue_push(vec![Value::Str("ish".into())]).is_ok());
        }

        interp.queue_wait_drain();
        assert_eq!(
            counter.load(Ordering::SeqCst),
            3,
            "drain should wait for all jobs to finish"
        );
    }

    // Issue #105: a job whose handler was never registered does not hold drain forever —
    // it is dropped with a warning and the queue empties.
    #[test]
    fn drain_handlersiz_ishni_tashlab_yuboradi() {
        let interp = Arc::new(Interp::new());
        assert!(interp.queue_push(vec![Value::Str("yetim".into())]).is_ok());

        // Call drain on a separate thread — so the test itself does not hang on a
        // regression (an infinite wait).
        let i2 = interp.clone();
        let h = std::thread::spawn(move || i2.queue_wait_drain());
        for _ in 0..200 {
            if h.is_finished() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            h.is_finished(),
            "drain should not hang on a handler-less job"
        );
        h.join().unwrap();
        assert!(
            interp.queue.inner.lock().queue.is_empty(),
            "the dropped job should leave the queue"
        );
    }

    // Issue #105: even if a handler-less job sits at the FRONT of the queue, a job that
    // has a handler is not blocked (the old busy-loop delayed everything 50ms per cycle).
    #[test]
    fn handlersiz_ish_boshqalarni_tosmaydi() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let interp = Arc::new(Interp::new());
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        // First a handler-less job — it takes the front of the queue.
        assert!(interp.queue_push(vec![Value::Str("yoq".into())]).is_ok());

        let handler = Value::Native(Arc::new(crate::value::NativeFn {
            name: "inc".into(),
            func: Box::new(move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(Value::Nil)
            }),
        }));
        assert!(
            interp
                .queue_on(vec![Value::Str("bor".into()), handler])
                .is_ok()
        );
        assert!(interp.queue_push(vec![Value::Str("bor".into())]).is_ok());

        // Drain waits for "bor" to finish and drops "yoq".
        interp.queue_wait_drain();
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "a job with a handler should not be stuck behind a handler-less job"
        );
    }
}
