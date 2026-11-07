//! Bounded document acquisition, preserving the shared checksum verifier.
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

type Totals = (usize, usize, u64);

/// Download independent documents with one connection per worker, using the original verifier.
pub(super) fn fetch_verified(
    entries: &[(&str, &str, &str)],
    directory: &Path,
    jobs: usize,
    force: bool,
) -> Result<Totals, String> {
    run_jobs(entries, jobs, |entry| {
        super::fetch_verified(std::slice::from_ref(entry), directory, 1, force)
            .map_err(|error| format!("{}: {error}", entry.0))
    })
}

/// Attempt every entry, then aggregate results after joining all bounded workers.
fn run_jobs<T: Sync, F>(entries: &[T], jobs: usize, run: F) -> Result<Totals, String>
where
    F: Fn(&T) -> Result<Totals, String> + Sync,
{
    run_jobs_with_spawn_check(entries, jobs, run, |_| Ok(()))
}

// The pre-spawn check permits deterministic testing of resource exhaustion without exhausting
// the host's threads. Its errors take exactly the same path as an OS spawn failure.
fn run_jobs_with_spawn_check<T: Sync, F, S>(
    entries: &[T],
    jobs: usize,
    run: F,
    spawn_check: S,
) -> Result<Totals, String>
where
    F: Fn(&T) -> Result<Totals, String> + Sync,
    S: Fn(usize) -> Result<(), String>,
{
    let workers = jobs.clamp(1, super::MAX_DOWNLOAD_JOBS).min(entries.len());
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers);
        let mut errors = Vec::new();
        for worker_index in 0..workers {
            let run = &run;
            let next = &next;
            let worker = spawn_check(worker_index).and_then(|()| {
                std::thread::Builder::new()
                    .spawn_scoped(scope, move || {
                        let mut totals: Totals = (0, 0, 0);
                        let mut errors = Vec::new();
                        loop {
                            // Saturation keeps the queue index bounded even for an exhausted queue.
                            let index = next
                                .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                                    Some(n.saturating_add(1))
                                })
                                .unwrap_or_else(|n| n);
                            let Some(entry) = entries.get(index) else {
                                break;
                            };
                            match run(entry) {
                                Ok(value) => add_totals(&mut totals, value),
                                Err(error) => errors.push(error),
                            }
                        }
                        (totals, errors)
                    })
                    .map_err(|error| error.to_string())
            });
            match worker {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    errors.push(format!("could not start document download worker: {error}"));
                }
            }
        }
        let mut totals = (0, 0, 0);
        for handle in handles {
            match handle.join() {
                Ok((value, failures)) => {
                    add_totals(&mut totals, value);
                    errors.extend(failures);
                }
                Err(_) => errors.push("document download worker failed unexpectedly".to_owned()),
            }
        }
        if errors.is_empty() {
            Ok(totals)
        } else {
            Err(errors.join("\n"))
        }
    })
}

fn add_totals(totals: &mut Totals, value: Totals) {
    totals.0 = totals.0.saturating_add(value.0);
    totals.1 = totals.1.saturating_add(value.1);
    totals.2 = totals.2.saturating_add(value.2);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Barrier,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn workers_overlap_and_execute_each_entry_once() {
        let barrier = Barrier::new(2);
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let visits: Vec<_> = (0..8).map(|_| AtomicUsize::new(0)).collect();
        let result = run_jobs(&[0, 1, 2, 3, 4, 5, 6, 7], 2, |index| {
            let count = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(count, Ordering::SeqCst);
            let visit = visits
                .get(*index)
                .ok_or_else(|| "missing visit counter".to_owned())?;
            visit.fetch_add(1, Ordering::SeqCst);
            barrier.wait();
            active.fetch_sub(1, Ordering::SeqCst);
            Ok((1, 0, 10))
        });
        assert_eq!(result, Ok((8, 0, 80)));
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        assert!(visits.iter().all(|n| n.load(Ordering::SeqCst) == 1));
    }

    #[test]
    fn failures_are_collected_without_skipping_later_entries() {
        let calls = AtomicUsize::new(0);
        let result = run_jobs(&[0, 1, 2, 3], 1, |index| {
            calls.fetch_add(1, Ordering::SeqCst);
            if *index % 2 == 0 {
                Err(format!("entry-{index}: failure"))
            } else {
                Ok((0, 1, 4))
            }
        });
        let error = result.err().unwrap_or_default();
        assert!(error.contains("entry-0: failure"));
        assert!(error.contains("entry-2: failure"));
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn empty_and_zero_job_inputs_finish() {
        assert_eq!(run_jobs::<u8, _>(&[], 2, |_| Ok((1, 0, 1))), Ok((0, 0, 0)));
        assert_eq!(run_jobs(&[1, 2], 0, |_| Ok((0, 1, 3))), Ok((0, 2, 6)));
    }
}

#[cfg(test)]
mod spawn_failure_tests {
    use super::*;

    #[test]
    fn rejected_worker_is_reported_after_started_workers_finish() {
        let completed = AtomicUsize::new(0);
        let result = run_jobs_with_spawn_check(
            &[0, 1, 2, 3],
            2,
            |_| {
                completed.fetch_add(1, Ordering::SeqCst);
                Ok((1, 0, 1))
            },
            |worker| {
                if worker == 0 {
                    Err("injected worker rejection".to_owned())
                } else {
                    Ok(())
                }
            },
        );
        assert_eq!(completed.load(Ordering::SeqCst), 4);
        assert!(matches!(result, Err(message) if message.contains("injected worker rejection")));
    }
}
