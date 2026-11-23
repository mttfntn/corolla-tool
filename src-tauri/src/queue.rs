//! Runs conversions on worker threads. One failed file never stops the others.

use crate::engines::{self, Done, Job, Reporter, Tools};
use crate::formats;
use crate::tools::Action;
use serde::Serialize;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub percent: u32,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct BatchReport {
    pub converted: usize,
    pub skipped: usize,
    pub cancelled: usize,
    pub not_smaller: usize,
    pub failures: Vec<(PathBuf, String)>,
    pub last_output: Option<PathBuf>,
    pub action: Action,
}

impl Default for BatchReport {
    fn default() -> Self {
        Self {
            converted: 0,
            skipped: 0,
            cancelled: 0,
            not_smaller: 0,
            failures: Vec::new(),
            last_output: None,
            action: Action::Convert(String::new()),
        }
    }
}

/// A job a worker is on, with how far it says it has got, in 1/1000s.
struct Running {
    units: usize,
    fraction: Arc<AtomicU32>,
}

#[derive(Default)]
struct State {
    pending: VecDeque<Job>,
    running: Vec<Running>,
    total: usize,
    done: usize,
    report: BatchReport,
}

/// Finished work, counting what's been done of the running jobs. `running` is
/// (units, fraction in 1/1000s). 100% is only ever shown by the wheel closing.
fn measure(done: usize, total: usize, running: &[(usize, u32)]) -> (usize, u32) {
    let partial: f64 = running.iter().map(|(units, fraction)| *units as f64 * f64::from(*fraction) / 1000.0).sum();
    let whole: usize = running.iter().map(|(units, fraction)| units * *fraction as usize / 1000).sum();
    let percent = if total == 0 { 0.0 } else { (done as f64 + partial) * 100.0 / total as f64 };
    ((done + whole).min(total), (percent as u32).min(99))
}

type Listener = Box<dyn Fn() + Send + Sync>;
type Finisher = Box<dyn Fn(BatchReport) + Send + Sync>;

pub struct Queue {
    state: Mutex<State>,
    wake: Condvar,
    cancel: AtomicBool,
    on_change: Mutex<Option<Listener>>,
    on_finish: Mutex<Option<Finisher>>,
}

impl Queue {
    /// Starts one worker thread per CPU core minus one (1 to 8), all waiting
    /// for jobs.
    pub fn start(tools: Arc<Tools>, quality: Arc<engines::QualitySetting>) -> Arc<Self> {
        let queue = Arc::new(Self {
            state: Mutex::default(),
            wake: Condvar::new(),
            cancel: AtomicBool::new(false),
            on_change: Mutex::new(None),
            on_finish: Mutex::new(None),
        });
        let workers = std::thread::available_parallelism()
            .map(|n| n.get().saturating_sub(1))
            .unwrap_or(1)
            .clamp(1, 8);
        for _ in 0..workers {
            let queue = queue.clone();
            let tools = tools.clone();
            let quality = quality.clone();
            std::thread::spawn(move || queue.work(&tools, &quality));
        }
        queue
    }

    /// `on_change` runs after every change, for the wheel's progress;
    /// `on_finish` runs once when a batch is done, with its report.
    pub fn set_listeners(&self, on_change: Listener, on_finish: Finisher) {
        *self.on_change.lock().unwrap() = Some(on_change);
        *self.on_finish.lock().unwrap() = Some(on_finish);
    }

    /// Adds the dropped files. Files already in the target format are counted
    /// as skipped, not converted; a combined PDF is one job for all its files.
    pub fn enqueue(&self, paths: Vec<PathBuf>, action: Action) {
        {
            let mut state = self.state.lock().unwrap();
            if action == Action::CombinePdf {
                state.total += paths.len();
                state.pending.push_back(Job { sources: paths, action: action.clone() });
            } else {
                for source in paths {
                    // A file already in the target format is not part of the work.
                    if let Action::Convert(target) = &action {
                        if formats::format_of(&source).as_deref() == Some(target.as_str()) {
                            state.report.skipped += 1;
                            continue;
                        }
                    }
                    state.total += 1;
                    state.pending.push_back(Job::one(source, action.clone()));
                }
            }
            state.report.action = action;
        }
        self.wake.notify_all();
        self.after_change();
    }

    /// Current progress, or None when nothing is running.
    pub fn snapshot(&self) -> Option<Progress> {
        let state = self.state.lock().unwrap();
        (state.total > state.done).then(|| {
            let running: Vec<(usize, u32)> =
                state.running.iter().map(|r| (r.units, r.fraction.load(Ordering::Relaxed))).collect();
            let (done, percent) = measure(state.done, state.total, &running);
            Progress { done, total: state.total, percent, label: state.report.action.label() }
        })
    }

    /// Drops everything queued and stops whatever is running.
    pub fn cancel(&self) {
        let dropped = {
            let mut state = self.state.lock().unwrap();
            let dropped: usize = state.pending.iter().map(Job::units).sum();
            state.pending.clear();
            state.done += dropped;
            state.report.cancelled += dropped;
            dropped
        };
        self.cancel.store(true, Ordering::SeqCst);
        if dropped > 0 {
            self.after_change();
        }
    }

    fn work(&self, tools: &Tools, quality: &engines::QualitySetting) {
        loop {
            let fraction = Arc::new(AtomicU32::new(0));
            let job = {
                let mut state = self.state.lock().unwrap();
                loop {
                    if let Some(job) = state.pending.pop_front() {
                        state.running.push(Running { units: job.units(), fraction: fraction.clone() });
                        break job;
                    }
                    state = self.wake.wait(state).unwrap();
                }
            };
            let notify = || self.notify();
            let reporter = Reporter { fraction: &fraction, notify: &notify };
            // An engine crash must not take a worker, and its counters, down with it.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                engines::convert(&job, tools, quality.get(), &self.cancel, &reporter)
            }))
            .unwrap_or_else(|_| Err("The converter crashed on this file".to_string()));
            {
                let mut state = self.state.lock().unwrap();
                state.running.retain(|r| !Arc::ptr_eq(&r.fraction, &fraction));
                let units = job.units();
                state.done += units;
                match result {
                    Ok(Done::Output { path, skipped }) => {
                        state.report.converted += units - skipped.len();
                        state.report.last_output = Some(path);
                        state.report.failures.extend(skipped);
                    }
                    Ok(Done::NotSmaller) => state.report.not_smaller += units,
                    Err(_) if self.cancel.load(Ordering::SeqCst) => state.report.cancelled += units,
                    Err(error) => {
                        let source = job.sources.into_iter().next().unwrap_or_default();
                        state.report.failures.push((source, error));
                    }
                }
            }
            self.after_change();
        }
    }

    fn notify(&self) {
        if let Some(listener) = &*self.on_change.lock().unwrap() {
            listener();
        }
    }

    fn after_change(&self) {
        let finished = {
            let mut state = self.state.lock().unwrap();
            let idle = state.done >= state.total;
            let anything = state.total > 0 || state.report.skipped > 0;
            if idle && anything {
                let report = std::mem::take(&mut state.report);
                state.total = 0;
                state.done = 0;
                self.cancel.store(false, Ordering::SeqCst);
                Some(report)
            } else {
                None
            }
        };
        self.notify();
        if let Some(report) = finished {
            if let Some(listener) = &*self.on_finish.lock().unwrap() {
                listener(report);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_counts_finished_and_running_work() {
        assert_eq!(measure(0, 4, &[]), (0, 0));
        assert_eq!(measure(1, 4, &[]), (1, 25));
        assert_eq!(measure(1, 4, &[(1, 500)]), (1, 37));
    }

    #[test]
    fn a_combined_pdf_counts_as_all_its_files() {
        assert_eq!(measure(0, 10, &[(10, 300)]), (3, 30));
    }

    #[test]
    fn it_never_claims_to_be_finished_early() {
        assert_eq!(measure(3, 4, &[(1, 1000)]).1, 99);
        assert_eq!(measure(0, 0, &[]), (0, 0));
    }
}
