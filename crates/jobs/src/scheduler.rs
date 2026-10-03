//! The background job scheduler (§15).
//!
//! > Never spawn unlimited threads. Use a bounded worker pool.
//! >
//! > ```text
//! > CPU cores <= 4:  1 heavy job
//! > CPU cores >= 8:  2-4 heavy jobs
//! > ```
//!
//! §74 lists "create unlimited background threads" among the prohibited things,
//! and §15.1 explains why capping job *count* is not enough on its own — the
//! threads must also run below normal priority, and FFmpeg's own thread count
//! must be capped per job.
//!
//! # What this does not schedule
//!
//! Playback, decode, render, and the audio callback (§69 levels 0–1). Those run
//! on their own threads at normal priority. Putting them behind this queue would
//! let a proxy job delay a frame, which inverts §80's ordering.

use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

use crate::priority::Priority;
use crate::thread_priority;

/// Identifies a queued job, for progress reporting and cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(pub u64);

/// Cooperative cancellation (§48).
///
/// §48: *"Cancellation must be checked frequently enough to be responsive —
/// inside decode loops, not only at job boundaries."* A token that is only
/// consulted between jobs makes a large export uncancellable.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// What a running job can see and say.
pub struct JobContext {
    pub id: JobId,
    cancel: CancelToken,
    events: Sender<JobEvent>,
}

impl JobContext {
    /// For running a task on the caller's own thread, outside any scheduler —
    /// for a caller that has to wait for the answer anyway. Never cancelled;
    /// its progress goes nowhere.
    pub fn detached() -> Self {
        let (events, _) = channel();
        Self {
            id: JobId(0),
            cancel: CancelToken::new(),
            events,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Report progress, 0.0 to 1.0 (§42).
    ///
    /// Dropped if the receiver is gone; a job must not fail because nobody is
    /// listening.
    pub fn progress(&self, fraction: f32) {
        let _ = self.events.send(JobEvent::Progress {
            id: self.id,
            fraction: fraction.clamp(0.0, 1.0),
        });
    }
}

/// The unit of work.
pub trait Task: Send + 'static {
    /// Human-readable, for the status bar.
    fn label(&self) -> String;
    fn priority(&self) -> Priority;
    /// Do the work. Return `Err` with a message the user could act on.
    fn run(&mut self, ctx: &JobContext) -> Result<(), String>;
}

/// What the scheduler reports back (§56).
#[derive(Debug, Clone, PartialEq)]
pub enum JobEvent {
    Started { id: JobId, label: String },
    Progress { id: JobId, fraction: f32 },
    Finished { id: JobId },
    Failed { id: JobId, message: String },
    Cancelled { id: JobId },
}

struct Queued {
    id: JobId,
    priority: Priority,
    /// Insertion order, so equal priorities run first-in-first-out rather than
    /// in whatever order the heap happens to produce.
    sequence: u64,
    task: Box<dyn Task>,
    cancel: CancelToken,
}

impl PartialEq for Queued {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.sequence == other.sequence
    }
}
impl Eq for Queued {}

impl Ord for Queued {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // `BinaryHeap` is a max-heap and lower `Priority` is more urgent, so
        // the comparison is reversed. Getting this backwards would run
        // background analysis ahead of what the user asked for.
        other
            .priority
            .cmp(&self.priority)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}
impl PartialOrd for Queued {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

struct Shared {
    queue: Mutex<State>,
    wakeup: Condvar,
}

struct State {
    pending: BinaryHeap<Queued>,
    /// Heavy jobs currently running, against the §15 cap.
    heavy_running: usize,
    shutting_down: bool,
}

pub struct JobScheduler {
    shared: Arc<Shared>,
    workers: Vec<std::thread::JoinHandle<()>>,
    events: Sender<JobEvent>,
    next_id: AtomicU64,
    next_sequence: AtomicU64,
    max_heavy: usize,
    /// Live tokens, so a caller can cancel by id.
    tokens: Mutex<std::collections::HashMap<JobId, CancelToken>>,
}

impl JobScheduler {
    /// Start a pool.
    ///
    /// `max_heavy` comes from `HardwareProfile::max_heavy_jobs` (§15), and
    /// `workers` is deliberately a little larger so cheap background work
    /// (thumbnails, waveforms) is not blocked behind a long proxy encode.
    pub fn new(max_heavy: usize) -> (Self, Receiver<JobEvent>) {
        let max_heavy = max_heavy.max(1);
        // One spare thread for light work, capped so a many-core machine does
        // not spawn a thread per core for jobs that are mostly I/O.
        let worker_count = (max_heavy + 1).min(4);

        let (events, receiver) = channel();
        let shared = Arc::new(Shared {
            queue: Mutex::new(State {
                pending: BinaryHeap::new(),
                heavy_running: 0,
                shutting_down: false,
            }),
            wakeup: Condvar::new(),
        });

        let mut workers = Vec::with_capacity(worker_count);
        for index in 0..worker_count {
            let shared = Arc::clone(&shared);
            let events = events.clone();
            match std::thread::Builder::new()
                .name(format!("bettercut-job-{index}"))
                .spawn(move || worker_loop(&shared, &events, max_heavy))
            {
                Ok(handle) => workers.push(handle),
                Err(err) => {
                    // A machine that cannot spawn a thread is in trouble, but
                    // refusing to start the editor over it would be worse
                    // (§50). Carry on with however many workers we got; the
                    // check below guarantees at least one.
                    tracing::error!(%err, index, "could not spawn a job worker");
                    break;
                }
            }
        }

        assert!(
            !workers.is_empty(),
            "the job scheduler needs at least one worker thread"
        );

        tracing::info!(workers = worker_count, max_heavy, "job scheduler started");

        (
            Self {
                shared,
                workers,
                events,
                next_id: AtomicU64::new(1),
                next_sequence: AtomicU64::new(0),
                max_heavy,
                tokens: Mutex::new(std::collections::HashMap::new()),
            },
            receiver,
        )
    }

    pub fn max_heavy(&self) -> usize {
        self.max_heavy
    }

    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    /// Queue a job. Returns its id, for progress and cancellation.
    pub fn submit(&self, task: Box<dyn Task>) -> JobId {
        let id = JobId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        let cancel = CancelToken::new();

        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.insert(id, cancel.clone());
        }

        let queued = Queued {
            id,
            priority: task.priority(),
            sequence,
            task,
            cancel,
        };

        if let Ok(mut state) = self.shared.queue.lock() {
            state.pending.push(queued);
        }
        self.shared.wakeup.notify_one();

        id
    }

    /// Cancel a job, whether queued or running (§48).
    pub fn cancel(&self, id: JobId) {
        if let Ok(tokens) = self.tokens.lock()
            && let Some(token) = tokens.get(&id)
        {
            token.cancel();
        }
        // A queued job stays in the heap; the worker checks the token before
        // starting it. Removing it from a `BinaryHeap` would mean rebuilding
        // the heap, and cancelled jobs are rare.
        self.shared.wakeup.notify_all();
    }

    /// Cancel everything queued or running.
    pub fn cancel_all(&self) {
        if let Ok(tokens) = self.tokens.lock() {
            for token in tokens.values() {
                token.cancel();
            }
        }
        self.shared.wakeup.notify_all();
    }

    pub fn queued_count(&self) -> usize {
        self.shared
            .queue
            .lock()
            .map_or(0, |state| state.pending.len())
    }

    pub fn heavy_running(&self) -> usize {
        self.shared
            .queue
            .lock()
            .map_or(0, |state| state.heavy_running)
    }

    /// Emit an event as if from a job. Used by callers that do work inline but
    /// want it reported the same way.
    pub fn report(&self, event: JobEvent) {
        let _ = self.events.send(event);
    }
}

impl Drop for JobScheduler {
    fn drop(&mut self) {
        // Cancel first: a worker part-way through a long encode would otherwise
        // hold the process open until it finished.
        self.cancel_all();
        if let Ok(mut state) = self.shared.queue.lock() {
            state.shutting_down = true;
        }
        self.shared.wakeup.notify_all();

        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        tracing::debug!("job scheduler stopped");
    }
}

fn worker_loop(shared: &Arc<Shared>, events: &Sender<JobEvent>, max_heavy: usize) {
    // §15.1. Done once, on the worker's own thread.
    thread_priority::lower_current_thread();

    loop {
        let mut queued = {
            let Ok(mut state) = shared.queue.lock() else {
                return;
            };

            loop {
                if state.shutting_down {
                    return;
                }

                // Take the most urgent job we are allowed to start. A heavy job
                // waits when the §15 cap is reached, but a light one may still
                // go — otherwise a single proxy encode would block thumbnails
                // for its entire duration.
                let can_take_heavy = state.heavy_running < max_heavy;
                let next_is_runnable = state
                    .pending
                    .peek()
                    .is_some_and(|job| can_take_heavy || !job.priority.is_heavy());

                if next_is_runnable {
                    match state.pending.pop() {
                        Some(job) => {
                            if job.priority.is_heavy() {
                                state.heavy_running += 1;
                            }
                            break job;
                        }
                        None => continue,
                    }
                }

                let Ok(next) = shared.wakeup.wait(state) else {
                    return;
                };
                state = next;
            }
        };

        let heavy = queued.priority.is_heavy();

        // A job cancelled while queued must not run at all.
        if queued.cancel.is_cancelled() {
            let _ = events.send(JobEvent::Cancelled { id: queued.id });
        } else {
            let _ = events.send(JobEvent::Started {
                id: queued.id,
                label: queued.task.label(),
            });

            let ctx = JobContext {
                id: queued.id,
                cancel: queued.cancel.clone(),
                events: events.clone(),
            };

            let outcome = queued.task.run(&ctx);

            // A cancelled job reports as cancelled even if it returned an
            // error on its way out — a cancellation is not a failure, and
            // surfacing it as one would put a spurious error in front of the
            // user (§50).
            let event = if queued.cancel.is_cancelled() {
                JobEvent::Cancelled { id: queued.id }
            } else {
                match outcome {
                    Ok(()) => JobEvent::Finished { id: queued.id },
                    Err(message) => JobEvent::Failed {
                        id: queued.id,
                        message,
                    },
                }
            };
            let _ = events.send(event);
        }

        if heavy && let Ok(mut state) = shared.queue.lock() {
            state.heavy_running -= 1;
            // Someone may have been waiting on the heavy slot.
            shared.wakeup.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    struct Recording {
        label: String,
        priority: Priority,
        order: Arc<Mutex<Vec<String>>>,
        /// Held while running, to observe concurrency.
        running: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        work: Duration,
    }

    impl Task for Recording {
        fn label(&self) -> String {
            self.label.clone()
        }
        fn priority(&self) -> Priority {
            self.priority
        }
        fn run(&mut self, _ctx: &JobContext) -> Result<(), String> {
            let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);

            if let Ok(mut order) = self.order.lock() {
                order.push(self.label.clone());
            }
            std::thread::sleep(self.work);

            self.running.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn drain_until(
        events: &Receiver<JobEvent>,
        mut done: impl FnMut(&JobEvent) -> bool,
        timeout: Duration,
    ) -> Vec<JobEvent> {
        let deadline = Instant::now() + timeout;
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            match events.recv_timeout(Duration::from_millis(50)) {
                Ok(event) => {
                    let stop = done(&event);
                    seen.push(event);
                    if stop {
                        return seen;
                    }
                }
                Err(_) => continue,
            }
        }
        seen
    }

    #[test]
    fn a_submitted_job_runs_and_reports() {
        let (scheduler, events) = JobScheduler::new(1);

        struct Trivial;
        impl Task for Trivial {
            fn label(&self) -> String {
                "trivial".to_owned()
            }
            fn priority(&self) -> Priority {
                Priority::Background
            }
            fn run(&mut self, ctx: &JobContext) -> Result<(), String> {
                ctx.progress(0.5);
                Ok(())
            }
        }

        let id = scheduler.submit(Box::new(Trivial));
        let seen = drain_until(
            &events,
            |e| matches!(e, JobEvent::Finished { .. }),
            Duration::from_secs(5),
        );

        assert!(seen.contains(&JobEvent::Started {
            id,
            label: "trivial".to_owned()
        }));
        assert!(seen.contains(&JobEvent::Progress { id, fraction: 0.5 }));
        assert!(seen.contains(&JobEvent::Finished { id }));
    }

    /// §15's cap, which is the whole reason this type exists: a 4-core machine
    /// runs **one** heavy job, however many are queued.
    #[test]
    fn heavy_jobs_never_exceed_the_cap() {
        let (scheduler, events) = JobScheduler::new(1);
        let peak = Arc::new(AtomicUsize::new(0));
        let running = Arc::new(AtomicUsize::new(0));
        let order = Arc::new(Mutex::new(Vec::new()));

        for i in 0..6 {
            scheduler.submit(Box::new(Recording {
                label: format!("proxy{i}"),
                priority: Priority::Proxy,
                order: Arc::clone(&order),
                running: Arc::clone(&running),
                peak: Arc::clone(&peak),
                work: Duration::from_millis(30),
            }));
        }

        let mut finished = 0;
        drain_until(
            &events,
            |e| {
                if matches!(e, JobEvent::Finished { .. }) {
                    finished += 1;
                }
                finished == 6
            },
            Duration::from_secs(10),
        );

        assert_eq!(
            peak.load(Ordering::SeqCst),
            1,
            "more than one heavy job ran at once"
        );
    }

    /// Light work must not queue behind a long encode — that is what makes a
    /// machine feel frozen while a proxy runs.
    #[test]
    fn light_jobs_run_while_a_heavy_job_holds_the_slot() {
        let (scheduler, events) = JobScheduler::new(1);
        let order = Arc::new(Mutex::new(Vec::new()));
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        scheduler.submit(Box::new(Recording {
            label: "heavy".to_owned(),
            priority: Priority::Proxy,
            order: Arc::clone(&order),
            running: Arc::clone(&running),
            peak: Arc::clone(&peak),
            work: Duration::from_millis(250),
        }));
        // Give the heavy job a moment to claim the slot.
        std::thread::sleep(Duration::from_millis(40));
        scheduler.submit(Box::new(Recording {
            label: "light".to_owned(),
            priority: Priority::Background,
            order: Arc::clone(&order),
            running: Arc::clone(&running),
            peak: Arc::clone(&peak),
            work: Duration::from_millis(10),
        }));

        let mut finished = 0;
        drain_until(
            &events,
            |e| {
                if matches!(e, JobEvent::Finished { .. }) {
                    finished += 1;
                }
                finished == 2
            },
            Duration::from_secs(10),
        );

        assert!(
            peak.load(Ordering::SeqCst) >= 2,
            "the light job waited for the heavy one to finish"
        );
    }

    /// §69's ordering: what the user asked for goes first.
    #[test]
    fn more_urgent_jobs_run_first() {
        // One worker, so ordering is unambiguous.
        let (scheduler, events) = JobScheduler::new(1);
        let order = Arc::new(Mutex::new(Vec::new()));
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        // Block the pool, then queue out of priority order behind it.
        let make = |label: &str, priority| {
            Box::new(Recording {
                label: label.to_owned(),
                priority,
                order: Arc::clone(&order),
                running: Arc::clone(&running),
                peak: Arc::clone(&peak),
                work: Duration::from_millis(5),
            })
        };

        scheduler.submit(make("blocker", Priority::Export));
        scheduler.submit(make("background", Priority::Background));
        scheduler.submit(make("proxy", Priority::Proxy));
        scheduler.submit(make("user", Priority::UserRequested));

        let mut finished = 0;
        drain_until(
            &events,
            |e| {
                if matches!(e, JobEvent::Finished { .. }) {
                    finished += 1;
                }
                finished == 4
            },
            Duration::from_secs(10),
        );

        let order = order.lock().expect("lock").clone();
        let user = order.iter().position(|l| l == "user").expect("user ran");
        let background = order
            .iter()
            .position(|l| l == "background")
            .expect("background ran");
        assert!(
            user < background,
            "background work ran before the user's request: {order:?}"
        );
    }

    /// §48: a job cancelled before it starts must not run at all.
    #[test]
    fn a_job_cancelled_while_queued_never_runs() {
        let (scheduler, events) = JobScheduler::new(1);
        let ran = Arc::new(AtomicBool::new(false));

        struct Blocker(Duration);
        impl Task for Blocker {
            fn label(&self) -> String {
                "blocker".to_owned()
            }
            fn priority(&self) -> Priority {
                Priority::Proxy
            }
            fn run(&mut self, _ctx: &JobContext) -> Result<(), String> {
                std::thread::sleep(self.0);
                Ok(())
            }
        }

        struct Marker(Arc<AtomicBool>);
        impl Task for Marker {
            fn label(&self) -> String {
                "marker".to_owned()
            }
            fn priority(&self) -> Priority {
                Priority::Proxy
            }
            fn run(&mut self, _ctx: &JobContext) -> Result<(), String> {
                self.0.store(true, Ordering::SeqCst);
                Ok(())
            }
        }

        scheduler.submit(Box::new(Blocker(Duration::from_millis(200))));
        let victim = scheduler.submit(Box::new(Marker(Arc::clone(&ran))));
        scheduler.cancel(victim);

        drain_until(
            &events,
            |e| matches!(e, JobEvent::Cancelled { id } if *id == victim),
            Duration::from_secs(10),
        );

        assert!(!ran.load(Ordering::SeqCst), "a cancelled job still ran");
    }

    /// §48 again: a running job sees the token and stops early.
    #[test]
    fn a_running_job_observes_cancellation() {
        let (scheduler, events) = JobScheduler::new(1);
        let completed_fully = Arc::new(AtomicBool::new(false));

        struct Looping(Arc<AtomicBool>);
        impl Task for Looping {
            fn label(&self) -> String {
                "looping".to_owned()
            }
            fn priority(&self) -> Priority {
                Priority::Proxy
            }
            fn run(&mut self, ctx: &JobContext) -> Result<(), String> {
                for _ in 0..200 {
                    if ctx.is_cancelled() {
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                self.0.store(true, Ordering::SeqCst);
                Ok(())
            }
        }

        let id = scheduler.submit(Box::new(Looping(Arc::clone(&completed_fully))));
        std::thread::sleep(Duration::from_millis(60));
        scheduler.cancel(id);

        drain_until(
            &events,
            |e| matches!(e, JobEvent::Cancelled { .. }),
            Duration::from_secs(10),
        );

        assert!(
            !completed_fully.load(Ordering::SeqCst),
            "the job ran to completion despite being cancelled"
        );
    }

    #[test]
    fn a_failing_job_reports_its_message() {
        let (scheduler, events) = JobScheduler::new(1);

        struct Failing;
        impl Task for Failing {
            fn label(&self) -> String {
                "failing".to_owned()
            }
            fn priority(&self) -> Priority {
                Priority::Background
            }
            fn run(&mut self, _ctx: &JobContext) -> Result<(), String> {
                Err("disk is full".to_owned())
            }
        }

        let id = scheduler.submit(Box::new(Failing));
        let seen = drain_until(
            &events,
            |e| matches!(e, JobEvent::Failed { .. }),
            Duration::from_secs(5),
        );

        assert!(seen.contains(&JobEvent::Failed {
            id,
            message: "disk is full".to_owned()
        }));
    }

    /// §74: never unlimited threads, whatever the machine claims.
    #[test]
    fn the_worker_count_is_bounded() {
        for max_heavy in [1_usize, 2, 4, 64] {
            let (scheduler, _events) = JobScheduler::new(max_heavy);
            assert!(
                scheduler.worker_count() <= 4,
                "{max_heavy} heavy jobs spawned {} workers",
                scheduler.worker_count()
            );
            assert!(scheduler.worker_count() >= 1);
        }
    }

    /// Dropping the scheduler must not hang on a long-running job.
    #[test]
    fn dropping_the_scheduler_stops_promptly() {
        struct Patient;
        impl Task for Patient {
            fn label(&self) -> String {
                "patient".to_owned()
            }
            fn priority(&self) -> Priority {
                Priority::Proxy
            }
            fn run(&mut self, ctx: &JobContext) -> Result<(), String> {
                for _ in 0..1000 {
                    if ctx.is_cancelled() {
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Ok(())
            }
        }

        let started = Instant::now();
        {
            let (scheduler, _events) = JobScheduler::new(2);
            scheduler.submit(Box::new(Patient));
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "shutdown waited for the job to finish naturally"
        );
    }
}
