//! In-memory [`Tracer`] for integration test assertions.

use std::cell::RefCell;

use strata::{TraceEvent, Tracer};

pub struct RecordingTracer(pub RefCell<Vec<TraceEvent>>);

impl Tracer for RecordingTracer {
    fn on_event(&self, event: TraceEvent) {
        self.0.borrow_mut().push(event);
    }
}
