//! How the day plan of a gate build ends a session (owner, 20 Nov 2026).
//!
//! The first full day simulation stopped each session a fixed time after the start call returned, but the player of the
//! test rig starts when the script has seen the start line and has set up its own processes, up to 46 s later, so the app
//! stopped before the player had finished and the reference capture held audio the app had no recording of. A session
//! now ends when its stop file appears (the script writes it 15 s after its player has finished); the plan's duration is
//! only a safety net for a player that died, and the evidence fails when the net was used.

use std::path::Path;
use std::time::{Duration, Instant};

/// Why a session's wait ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The session's stop file appeared.
    File,
    /// The deadline passed first (no stop file at all, or a safety net that was needed).
    Timer,
}

/// Waits until `file` exists or `deadline` passes, looking every `poll`. A missing `file` means "no stop file": the
/// wait runs to the deadline (the smoke and kill runs of the day script plan their sessions the old way).
pub fn wait_for_stop(file: Option<&Path>, deadline: Instant, poll: Duration) -> StopReason {
    loop {
        if file.is_some_and(Path::exists) {
            return StopReason::File;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return StopReason::Timer;
        }
        std::thread::sleep(poll.min(left));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("wb-day-stop-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&dir);
        dir
    }

    #[test]
    fn an_existing_stop_file_ends_the_wait_at_once() {
        let f = temp_file("exists");
        std::fs::write(&f, "stop").unwrap();
        let t = Instant::now();
        let r = wait_for_stop(
            Some(&f),
            Instant::now() + Duration::from_secs(60),
            Duration::from_millis(20),
        );
        assert_eq!(r, StopReason::File);
        assert!(t.elapsed() < Duration::from_secs(2));
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn a_session_is_not_stopped_by_a_timer_while_the_stop_file_may_still_come() {
        // The old behaviour stopped at the deadline counted from the start call; with a stop file the deadline is
        // only the safety net, and a file that appears before it ends the wait as a file, not as a timer.
        let f = temp_file("later");
        let f2 = f.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            std::fs::write(&f2, "stop").unwrap();
        });
        let t = Instant::now();
        let r = wait_for_stop(
            Some(&f),
            Instant::now() + Duration::from_secs(30),
            Duration::from_millis(20),
        );
        writer.join().unwrap();
        assert_eq!(r, StopReason::File);
        assert!(t.elapsed() >= Duration::from_millis(250), "it waited for the file");
        assert!(t.elapsed() < Duration::from_secs(5));
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn the_safety_net_ends_a_wait_whose_stop_file_never_comes() {
        let f = temp_file("never");
        let t = Instant::now();
        let r = wait_for_stop(
            Some(&f),
            Instant::now() + Duration::from_millis(200),
            Duration::from_millis(20),
        );
        assert_eq!(r, StopReason::Timer);
        assert!(t.elapsed() >= Duration::from_millis(190));
    }

    #[test]
    fn without_a_stop_file_the_wait_runs_to_the_deadline() {
        let t = Instant::now();
        let r = wait_for_stop(
            None,
            Instant::now() + Duration::from_millis(150),
            Duration::from_millis(20),
        );
        assert_eq!(r, StopReason::Timer);
        assert!(t.elapsed() >= Duration::from_millis(140));
    }
}
