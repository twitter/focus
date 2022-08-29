use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::spawn;
use std::time::Duration;

use log::debug;

enum InvocationKind {
    Timed,
    Direct,
}

trait PeriodicInvocationTarget: Send + Sync {
    fn run(&self, invocation_kind: InvocationKind);
}

struct Periodic {
    #[allow(dead_code)]
    interval: Duration,
    #[allow(dead_code)]
    target: Arc<dyn PeriodicInvocationTarget>,
    stop_tx: mpsc::Sender<()>,
    invoke_tx: mpsc::Sender<bool>,
    stopped: Arc<AtomicBool>,
}

impl Periodic {
    #[allow(dead_code)]
    pub fn new(
        interval: Duration,
        target: Arc<dyn PeriodicInvocationTarget>,
    ) -> (Periodic, std::thread::JoinHandle<()>) {
        let (stop_tx, stop_rx) = mpsc::channel();
        let (invoke_tx, invoke_rx) = mpsc::channel::<bool>();
        let cloned_target = target.clone();
        let stopped = Arc::new(AtomicBool::new(false));

        let stopped_clone = stopped.clone();
        let handle = spawn(move || {
            while stop_rx.try_recv().is_err() {
                match invoke_rx.recv_timeout(interval) {
                    Ok(true) => {
                        debug!("Direct invocation");
                        cloned_target.run(InvocationKind::Direct)
                    }
                    Ok(false) => {
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        debug!("Timed invocation");
                        cloned_target.run(InvocationKind::Timed);
                    }
                    Err(_) => panic!("Timed out"),
                }
            }
            stopped_clone.store(true, Ordering::SeqCst);
        });

        (
            Periodic {
                interval,
                target,
                stop_tx,
                invoke_tx,
                stopped,
            },
            handle,
        )
    }

    #[allow(dead_code)]
    pub fn invoke(&self) {
        self.invoke_tx.send(true).unwrap();
    }

    #[allow(dead_code)]
    pub fn cancel(&self) {
        let _yolo = self.invoke_tx.send(false);
        let _ = self.stop_tx.send(());
    }

    #[allow(dead_code)]
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct PeriodicTest {
        timed_calls: AtomicUsize,
        direct_calls: AtomicUsize,
    }

    impl PeriodicTest {
        fn new() -> Self {
            Self {
                timed_calls: AtomicUsize::new(0),
                direct_calls: AtomicUsize::new(0),
            }
        }
    }

    impl PeriodicInvocationTarget for PeriodicTest {
        fn run(&self, invocation_kind: InvocationKind) {
            match invocation_kind {
                InvocationKind::Timed => {
                    self.timed_calls.fetch_add(1, Ordering::SeqCst);
                }
                InvocationKind::Direct => {
                    self.direct_calls.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
    }

    #[test]
    fn test_periodic_immediate_cancellation() {
        let t = Arc::new(PeriodicTest::new());
        let (p, h) = Periodic::new(Duration::from_secs(300), t.clone());
        p.cancel();
        h.join().unwrap();
        assert_eq!(p.stopped(), true);
        assert_eq!(t.timed_calls.load(Ordering::SeqCst), 0);
        assert_eq!(t.direct_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_periodic_invocation_time_elapses() {
        let t = Arc::new(PeriodicTest::new());
        let (p, h) = Periodic::new(Duration::from_millis(5), t.clone());

        std::thread::sleep(Duration::from_millis(50));
        p.cancel();
        h.join().unwrap();
        assert_eq!(p.stopped(), true);
        assert_ne!(t.timed_calls.load(Ordering::SeqCst), 0);
        assert_eq!(t.direct_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_periodic_invocation_direct() {
        let t = Arc::new(PeriodicTest::new());
        let (p, h) = Periodic::new(Duration::from_secs(300), t.clone());
        p.invoke();
        std::thread::sleep(Duration::from_millis(50));
        p.cancel();
        h.join().unwrap();
        assert_eq!(p.stopped(), true);
        assert_eq!(t.timed_calls.load(Ordering::SeqCst), 0);
        assert_eq!(t.direct_calls.load(Ordering::SeqCst), 1);
    }
}
