//! Circuit Breaker for AI calls.
//!
//! # State Machine
//! - **Closed**: Normal operation, failures are tracked.
//! - **Open**: After 3 failures within 60s, all calls are rejected for 60s.
//! - **Half-Open**: After 60s, one test call is allowed. Success → Closed, Failure → Open again.

use std::time::{Duration, Instant};
use parking_lot::RwLock;

#[derive(Debug, Clone, Copy)]
enum CircuitState {
    Closed,
    Open(Instant), // timestamp when circuit opened
    HalfOpen,
}

pub struct CircuitBreaker {
    inner: RwLock<CircuitBreakerInner>,
}

struct CircuitBreakerInner {
    state: CircuitState,
    failures: Vec<Instant>, // failure timestamps within the window
    failure_threshold: u32,
    failure_window: Duration,
    reset_timeout: Duration,
    half_open_permitted: bool, // ensures only one probe in half-open state
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self {
            inner: RwLock::new(CircuitBreakerInner {
                state: CircuitState::Closed,
                failures: Vec::new(),
                failure_threshold: 3,
                failure_window: Duration::from_secs(60),
                reset_timeout: Duration::from_secs(60),
                half_open_permitted: false,
            }),
        }
    }
}

impl CircuitBreaker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if a call is allowed. Returns `Err` if circuit is open.
    pub fn allow_request(&self) -> Result<(), String> {
        let mut inner = self.inner.write();

        match inner.state {
            CircuitState::Closed => Ok(()),
            CircuitState::Open(opened_at) => {
                if Instant::now().duration_since(opened_at) >= inner.reset_timeout {
                    // Transition to half-open — only one probe allowed
                    inner.state = CircuitState::HalfOpen;
                    inner.half_open_permitted = true;
                    Ok(())
                } else {
                    let remaining = inner.reset_timeout - Instant::now().duration_since(opened_at);
                    Err(format!(
                        "KI-System temporär nicht verfügbar (Circuit Breaker offen, {}s verbleibend)",
                        remaining.as_secs()
                    ))
                }
            }
            CircuitState::HalfOpen => {
                if inner.half_open_permitted {
                    inner.half_open_permitted = false;
                    Ok(())
                } else {
                    Err("KI-System temporär nicht verfügbar (Testcall läuft)".to_string())
                }
            }
        }
    }

    /// Record a successful call. Resets the circuit to closed.
    pub fn record_success(&self) {
        let mut inner = self.inner.write();
        inner.state = CircuitState::Closed;
        inner.failures.clear();
        inner.half_open_permitted = false;
    }

    /// Explicitly reset the circuit breaker to closed state (user-triggered).
    pub fn reset(&self) {
        let mut inner = self.inner.write();
        inner.state = CircuitState::Closed;
        inner.failures.clear();
        inner.half_open_permitted = false;
        tracing::info!("Circuit Breaker: manuell zurückgesetzt");
    }

    /// Record a failed call. May open the circuit if threshold is exceeded.
    pub fn record_failure(&self) {
        let mut inner = self.inner.write();
        let now = Instant::now();

        // Add failure timestamp
        inner.failures.push(now);

        // Remove failures outside the window
        let window = inner.failure_window;
        inner.failures.retain(|t| now.duration_since(*t) < window);

        // Check if threshold is exceeded
        if (inner.failures.len() as u32) >= inner.failure_threshold {
            inner.state = CircuitState::Open(now);
            inner.half_open_permitted = false;
            tracing::warn!(
                "Circuit Breaker: OFFEN ({} Fehler in {:?}s)",
                inner.failures.len(),
                inner.failure_window.as_secs()
            );
        }
    }

    /// Get current state for logging/debugging.
    pub fn state(&self) -> String {
        let inner = self.inner.read();
        match inner.state {
            CircuitState::Closed => format!("geschlossen ({} Fehler im Fenster)", inner.failures.len()),
            CircuitState::Open(opened_at) => {
                let elapsed = Instant::now().duration_since(opened_at);
                let remaining = if elapsed >= inner.reset_timeout {
                    0
                } else {
                    (inner.reset_timeout - elapsed).as_secs()
                };
                format!("offen ({}s verbleibend)", remaining)
            }
            CircuitState::HalfOpen => "halb-offen (Testcall erlaubt)".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_closed_allows_requests() {
        let cb = CircuitBreaker::new();
        assert!(cb.allow_request().is_ok());
    }

    #[test]
    fn test_success_resets_failures() {
        let cb = CircuitBreaker::new();
        cb.record_failure();
        cb.record_failure();
        cb.record_success();
        assert!(cb.allow_request().is_ok());
        assert_eq!(cb.state(), "geschlossen (0 Fehler im Fenster)");
    }

    #[test]
    fn test_opens_after_threshold() {
        let cb = CircuitBreaker::new();
        cb.record_failure();
        cb.record_failure();
        assert!(cb.allow_request().is_ok()); // 2 < 3, still closed
        cb.record_failure();
        assert!(cb.allow_request().is_err()); // 3 >= 3, now open
    }

    #[test]
    fn test_half_open_after_timeout() {
        let cb = CircuitBreaker::new();
        // Override reset_timeout for testing
        {
            let mut inner = cb.inner.write();
            inner.reset_timeout = Duration::from_millis(100);
        }
        cb.record_failure();
        cb.record_failure();
        cb.record_failure(); // Open
        assert!(cb.allow_request().is_err());

        tokio_test::block_on(async {
            tokio::time::sleep(Duration::from_millis(150)).await;
        });

        assert!(cb.allow_request().is_ok()); // Half-open
    }

    #[test]
    fn test_success_closes_half_open() {
        let cb = CircuitBreaker::new();
        cb.record_success();
        assert_eq!(cb.state(), "geschlossen (0 Fehler im Fenster)");
    }
}
