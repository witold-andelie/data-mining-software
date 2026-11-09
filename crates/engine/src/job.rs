//! Toolkit-independent worker jobs. Cancelling detaches the result, never kills the thread.

use std::collections::HashMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(pub u64);

/// Called whenever a job has news (for example, to request a repaint).
pub type Waker = Arc<dyn Fn() + Send + Sync>;
type SharedWaker = Arc<Mutex<Option<Waker>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub current: Option<String>,
}

const RUNNING: u8 = 0;
const OVER: u8 = 1;
const CANCELLED: u8 = 2;

/// Handed to the work closure. The sender erases only the manager's output type; progress
/// and final events still travel through the same FIFO channel.
pub struct JobContext {
    id: JobId,
    sender: Box<dyn Fn(Progress) + Send + Sync>,
    waker: SharedWaker,
    state: Arc<AtomicU8>,
}

impl JobContext {
    pub fn id(&self) -> JobId {
        self.id
    }

    pub fn progress(&self, progress: Progress) {
        if !self.is_cancelled() {
            (self.sender)(progress);
            wake(&self.waker);
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::Acquire) == CANCELLED
    }
}

#[derive(Debug)]
pub enum JobEvent<T> {
    Progress { job: JobId, progress: Progress },
    Finished { job: JobId, output: T },
    Panicked { job: JobId, message: String },
    Cancelled { job: JobId },
}

struct Job {
    state: Arc<AtomicU8>,
    progress: Option<Progress>,
}

pub struct JobManager<T: Send + 'static> {
    next_id: u64,
    sender: mpsc::Sender<JobEvent<T>>,
    receiver: mpsc::Receiver<JobEvent<T>>,
    jobs: HashMap<JobId, Job>,
    waker: SharedWaker,
}

impl<T: Send + 'static> fmt::Debug for JobManager<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobManager")
            .field("jobs", &self.jobs.keys())
            .finish_non_exhaustive()
    }
}

impl<T: Send + 'static> Default for JobManager<T> {
    fn default() -> Self {
        Self::new(None)
    }
}

impl<T: Send + 'static> JobManager<T> {
    pub fn new(waker: Option<Waker>) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            next_id: 1,
            sender,
            receiver,
            jobs: HashMap::new(),
            waker: Arc::new(Mutex::new(waker)),
        }
    }

    pub fn set_waker(&mut self, waker: Option<Waker>) {
        *self.waker.lock().unwrap_or_else(|e| e.into_inner()) = waker;
    }

    pub fn spawn(&mut self, work: impl FnOnce(&JobContext) -> T + Send + 'static) -> JobId {
        let id = loop {
            let id = JobId(self.next_id);
            self.next_id = self.next_id.wrapping_add(1);
            if !self.jobs.contains_key(&id) {
                break id;
            }
        };
        let state = Arc::new(AtomicU8::new(RUNNING));
        self.jobs.insert(
            id,
            Job {
                state: Arc::clone(&state),
                progress: None,
            },
        );
        let progress_sender = self.sender.clone();
        let context = JobContext {
            id,
            sender: Box::new(move |progress| {
                let _ = progress_sender.send(JobEvent::Progress { job: id, progress });
            }),
            waker: Arc::clone(&self.waker),
            state: Arc::clone(&state),
        };
        let sender = self.sender.clone();
        let waker = Arc::clone(&self.waker);
        let spawned = thread::Builder::new()
            .name(format!("dms-job-{}", id.0))
            .spawn(move || {
                let event = match catch_unwind(AssertUnwindSafe(|| work(&context))) {
                    Ok(output) => JobEvent::Finished { job: id, output },
                    Err(payload) => JobEvent::Panicked {
                        job: id,
                        message: panic_message(payload.as_ref()),
                    },
                };
                if context
                    .state
                    .compare_exchange(RUNNING, OVER, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    let _ = sender.send(event);
                    wake(&waker);
                }
            });
        if let Err(error) = spawned {
            state.store(OVER, Ordering::Release);
            let _ = self.sender.send(JobEvent::Panicked {
                job: id,
                message: error.to_string(),
            });
            wake(&self.waker);
        }
        id
    }

    pub fn cancel(&mut self, job: JobId) -> bool {
        let Some(entry) = self.jobs.get(&job) else {
            return false;
        };
        if entry
            .state
            .compare_exchange(RUNNING, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        let _ = self.sender.send(JobEvent::Cancelled { job });
        wake(&self.waker);
        true
    }

    pub fn poll(&mut self) -> Vec<JobEvent<T>> {
        let mut events = Vec::new();
        while let Ok(event) = self.receiver.try_recv() {
            let job = match &event {
                JobEvent::Progress { job, .. }
                | JobEvent::Finished { job, .. }
                | JobEvent::Panicked { job, .. }
                | JobEvent::Cancelled { job } => *job,
            };
            let Some(entry) = self.jobs.get_mut(&job) else {
                continue;
            };
            if entry.state.load(Ordering::Acquire) == CANCELLED
                && !matches!(event, JobEvent::Cancelled { .. })
            {
                continue;
            }
            match &event {
                JobEvent::Progress { progress, .. } => entry.progress = Some(progress.clone()),
                _ => {
                    self.jobs.remove(&job);
                }
            }
            events.push(event);
        }
        events
    }

    pub fn running(&self) -> Vec<JobId> {
        let mut running: Vec<_> = self
            .jobs
            .iter()
            .filter(|(_, job)| job.state.load(Ordering::Acquire) == RUNNING)
            .map(|(&id, _)| id)
            .collect();
        running.sort_unstable();
        running
    }

    pub fn progress(&self, job: JobId) -> Option<Progress> {
        self.jobs.get(&job).and_then(|job| job.progress.clone())
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else {
        "unknown panic".to_string()
    }
}

fn wake(waker: &SharedWaker) {
    let waker = waker.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(waker) = waker {
        // A host callback must not unwind out of the worker or the UI's poll/cancel path.
        let _ = catch_unwind(AssertUnwindSafe(|| waker()));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    pub(crate) fn wait_for_job<T: Send + 'static>(manager: &mut JobManager<T>) -> Vec<JobEvent<T>> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut events = Vec::new();
        loop {
            events.extend(manager.poll());
            if events
                .iter()
                .any(|event| !matches!(event, JobEvent::Progress { .. }))
            {
                return events;
            }
            assert!(
                Instant::now() < deadline,
                "job did not finish within 10 seconds"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn progress_precedes_finished_and_wakes_the_host() {
        fn assert_send<T: Send>() {}
        assert_send::<JobManager<String>>();
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        let mut jobs = JobManager::new(Some(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })));
        let job = jobs.spawn(|ctx| {
            ctx.progress(Progress {
                done: 0,
                total: 1,
                current: Some("n".into()),
            });
            ctx.progress(Progress {
                done: 1,
                total: 1,
                current: None,
            });
            ctx.id()
        });
        let events = wait_for_job(&mut jobs);
        assert!(matches!(events.as_slice(), [
            JobEvent::Progress { job: a, progress: Progress { done: 0, .. } },
            JobEvent::Progress { job: b, progress: Progress { done: 1, .. } },
            JobEvent::Finished { job: c, output },
        ] if *a == job && *b == job && *c == job && *output == job));
        assert!(wakes.load(Ordering::SeqCst) >= 2);
        assert!(jobs.running().is_empty());
        assert_eq!(jobs.progress(job), None);
        assert!(!jobs.cancel(job));
        assert!(jobs.poll().is_empty());
    }

    #[test]
    fn panics_are_events_with_their_payload() {
        let mut jobs = JobManager::<()>::new(None);
        let id = jobs.spawn(|_| panic!("worker boom"));
        assert!(
            matches!(wait_for_job(&mut jobs).as_slice(), [JobEvent::Panicked { job, message }] if *job == id && message == "worker boom")
        );
        let id = jobs.spawn(|_| std::panic::panic_any(String::from("owned boom")));
        assert!(
            matches!(wait_for_job(&mut jobs).as_slice(), [JobEvent::Panicked { job, message }] if *job == id && message == "owned boom")
        );
        let id = jobs.spawn(|_| std::panic::panic_any(42usize));
        assert!(
            matches!(wait_for_job(&mut jobs).as_slice(), [JobEvent::Panicked { job, message }] if *job == id && message == "unknown panic")
        );
    }

    #[test]
    fn cancellation_is_immediate_and_discards_old_and_later_events() {
        for should_panic in [false, true] {
            let (release, gate) = mpsc::channel();
            let (ready, arrived) = mpsc::channel();
            let (ended, ended_receiver) = mpsc::channel();
            let mut jobs = JobManager::new(None);
            let id = jobs.spawn(move |ctx| {
                ctx.progress(Progress {
                    done: 0,
                    total: 1,
                    current: Some("old".into()),
                });
                ready.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(10)).unwrap();
                assert!(ctx.is_cancelled());
                ctx.progress(Progress {
                    done: 1,
                    total: 1,
                    current: None,
                });
                ended.send(()).unwrap();
                assert!(!should_panic, "cancelled panic");
            });
            arrived.recv_timeout(Duration::from_secs(10)).unwrap();
            assert_eq!(jobs.running(), [id]);
            assert!(jobs.cancel(id));
            assert!(!jobs.cancel(id));
            assert!(!jobs.cancel(JobId(999)));
            assert!(jobs.running().is_empty());
            // Simulate messages racing with cancellation, including a completed work result.
            jobs.sender
                .send(JobEvent::Progress {
                    job: id,
                    progress: Progress {
                        done: 1,
                        total: 1,
                        current: None,
                    },
                })
                .unwrap();
            jobs.sender
                .send(JobEvent::Finished {
                    job: id,
                    output: (),
                })
                .unwrap();
            jobs.sender
                .send(JobEvent::Panicked {
                    job: id,
                    message: "late panic".into(),
                })
                .unwrap();
            assert!(matches!(jobs.poll().as_slice(), [JobEvent::Cancelled { job }] if *job == id));
            release.send(()).unwrap();
            ended_receiver
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
            assert!(jobs.poll().is_empty());
            assert_eq!(jobs.progress(id), None);
        }
    }

    #[test]
    fn polling_retains_progress_and_waker_changes_reach_existing_jobs() {
        let (release, gate) = mpsc::channel();
        let (ready, arrived) = mpsc::channel();
        let mut jobs = JobManager::new(None);
        let id = jobs.spawn(move |ctx| {
            ctx.progress(Progress {
                done: 0,
                total: 1,
                current: Some("n".into()),
            });
            ready.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(10)).unwrap();
        });
        arrived.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(
            jobs.poll().as_slice(),
            [JobEvent::Progress { .. }]
        ));
        assert_eq!(
            jobs.progress(id),
            Some(Progress {
                done: 0,
                total: 1,
                current: Some("n".into())
            })
        );
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        jobs.set_waker(Some(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })));
        release.send(()).unwrap();
        wait_for_job(&mut jobs);
        let deadline = Instant::now() + Duration::from_secs(10);
        while wakes.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn host_waker_panics_do_not_escape() {
        let mut jobs = JobManager::new(Some(Arc::new(|| panic!("host waker"))));
        jobs.spawn(|ctx| {
            ctx.progress(Progress {
                done: 1,
                total: 1,
                current: None,
            });
            7
        });
        assert!(matches!(
            wait_for_job(&mut jobs).last(),
            Some(JobEvent::Finished { output: 7, .. })
        ));
    }
}
