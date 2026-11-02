//! A cap on how many polls one caller holds open at once.
//!
//! Every waiting long poll pins a connection and a task, so a client that
//! forgets to close them (or a loop that opens a poll per iteration) can
//! starve the hub. The cap is per caller: one runaway app cannot spend the
//! allowance of the others.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::http::StatusCode;

use crate::http::error::ApiError;

#[derive(Debug, Default)]
pub struct Polls {
    open: Mutex<HashMap<String, usize>>,
}

impl Polls {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes one slot for `caller`, or refuses with a 429 naming the cap.
    pub fn acquire(self: &Arc<Self>, caller: &str, cap: usize) -> Result<PollSlot, ApiError> {
        let mut open = self.open.lock().expect("the poll lock is never poisoned");
        let held = open.entry(caller.to_string()).or_insert(0);
        if *held >= cap {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                format!("{caller:?} already has {cap} polls open"),
                format!(
                    "close the polls you no longer need, or raise KYU_MAX_CONCURRENT_POLLS \
                     (currently {cap}) and restart the hub. One consumer rarely needs more \
                     than one poll per subscription."
                ),
            ));
        }
        *held += 1;
        Ok(PollSlot {
            polls: self.clone(),
            caller: caller.to_string(),
        })
    }
}

/// One open poll; the slot is returned when it drops, on every exit path.
pub struct PollSlot {
    polls: Arc<Polls>,
    caller: String,
}

impl Drop for PollSlot {
    fn drop(&mut self) {
        let mut open = self
            .polls
            .open
            .lock()
            .expect("the poll lock is never poisoned");
        if let Some(held) = open.get_mut(&self.caller) {
            *held -= 1;
            if *held == 0 {
                open.remove(&self.caller);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cap_is_per_caller_and_slots_come_back_on_drop() {
        let polls = Arc::new(Polls::new());
        let first = polls.acquire("a", 2).expect("one");
        let _second = polls.acquire("a", 2).expect("two");
        let refused = polls.acquire("a", 2).err().expect("a third is refused");
        assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
        assert!(refused.remedy.contains("KYU_MAX_CONCURRENT_POLLS"));
        let _other = polls.acquire("b", 2).expect("another caller is unaffected");
        drop(first);
        let _again = polls.acquire("a", 2).expect("a slot came back");
    }
}
