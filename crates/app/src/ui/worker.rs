//! Background analysis thread.
//!
//! Payback analysis and A/B comparisons fast-forward forks of the sim, which
//! takes a while in debug builds. Running them on a worker thread keeps the
//! UI at a steady frame rate. The engine is deterministic and `Send`, so a
//! cloned `Sim` can simply be shipped across.

use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use chainlab_engine::Sim;
use chainlab_engine::config::ChainConfig;
use chainlab_engine::engine::analysis::{self, Payback, RunSummary};

pub enum Job {
    Payback(Box<Sim>),
    Compare {
        chain: Box<ChainConfig>,
        a: (String, BTreeMap<String, f64>),
        b: (String, BTreeMap<String, f64>),
        seed: u64,
        ticks: u64,
    },
}

pub enum Done {
    Payback { tick: u64, list: Vec<Payback> },
    Compare(Box<Result<(RunSummary, RunSummary), String>>),
}

pub struct Worker {
    tx: Sender<Job>,
    rx: Receiver<Done>,
    pub payback_busy: bool,
    pub compare_busy: bool,
}

impl Worker {
    pub fn spawn() -> Worker {
        let (tx, jobs) = channel::<Job>();
        let (done, rx) = channel::<Done>();
        thread::spawn(move || {
            for job in jobs {
                let out = match job {
                    Job::Payback(sim) => Done::Payback {
                        tick: sim.state.tick,
                        list: analysis::payback(&sim, analysis::HORIZON),
                    },
                    Job::Compare {
                        chain,
                        a,
                        b,
                        seed,
                        ticks,
                    } => {
                        let ra = analysis::run_headless(&chain, &a.1, seed, ticks, &a.0);
                        let rb = analysis::run_headless(&chain, &b.1, seed, ticks, &b.0);
                        Done::Compare(Box::new(ra.and_then(|x| rb.map(|y| (x, y)))))
                    }
                };
                if done.send(out).is_err() {
                    break;
                }
            }
        });
        Worker {
            tx,
            rx,
            payback_busy: false,
            compare_busy: false,
        }
    }

    pub fn send(&mut self, job: Job) {
        match job {
            Job::Payback(_) => self.payback_busy = true,
            Job::Compare { .. } => self.compare_busy = true,
        }
        let _ = self.tx.send(job);
    }

    pub fn poll(&mut self) -> Vec<Done> {
        let out: Vec<Done> = self.rx.try_iter().collect();
        for d in &out {
            match d {
                Done::Payback { .. } => self.payback_busy = false,
                Done::Compare(_) => self.compare_busy = false,
            }
        }
        out
    }
}
