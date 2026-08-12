//! The dashboard (K10): topics, subscriptions, backlogs, recent
//! messages, dead letters — and the copy-paste curl examples rendered
//! from a real recent payload, which is the mechanism behind the
//! five-minute re-entry test (S1).
//!
//! Rendering is minijinja + htmx (T4). Payloads are untrusted input:
//! autoescape stays on, display is capped and the cap is always visible
//! (AR11).
//!
//! Built in L7.
