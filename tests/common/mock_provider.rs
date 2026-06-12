//! Scriptable [`Provider`] for integration tests (no network).

use std::cell::RefCell;

use strata::{
    CompletionRequest, CompletionResponse, Provider, ProviderError,
};

/// One scripted provider response.
pub enum MockStep {
    Ok(CompletionResponse),
    Err(ProviderError),
}

enum MockMode {
    Sequential,
    RepeatLast,
}

/// Returns preset completion results in order, or repeats the last step.
pub struct MockProvider {
    script: RefCell<Vec<MockStep>>,
    mode: MockMode,
    call_count: RefCell<u32>,
    recorded_requests: RefCell<Vec<CompletionRequest>>,
}

impl MockProvider {
    pub fn new(steps: Vec<MockStep>) -> Self {
        Self {
            script: RefCell::new(steps),
            mode: MockMode::Sequential,
            call_count: RefCell::new(0),
            recorded_requests: RefCell::new(Vec::new()),
        }
    }

    pub fn repeating(step: MockStep) -> Self {
        Self {
            script: RefCell::new(vec![step]),
            mode: MockMode::RepeatLast,
            call_count: RefCell::new(0),
            recorded_requests: RefCell::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> u32 {
        *self.call_count.borrow()
    }

    pub fn recorded_requests(&self) -> std::cell::Ref<'_, Vec<CompletionRequest>> {
        self.recorded_requests.borrow()
    }
}

impl Provider for MockProvider {
    fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        *self.call_count.borrow_mut() += 1;
        self.recorded_requests.borrow_mut().push(req);

        match self.mode {
            MockMode::Sequential => {
                let mut script = self.script.borrow_mut();
                let step = script
                    .first()
                    .ok_or_else(|| {
                        ProviderError::InvalidResponse("mock script exhausted".into())
                    })?
                    .clone_step();
                script.remove(0);
                match step {
                    MockStep::Ok(resp) => Ok(resp),
                    MockStep::Err(err) => Err(err),
                }
            }
            MockMode::RepeatLast => {
                let script = self.script.borrow();
                let step = script
                    .last()
                    .ok_or_else(|| ProviderError::InvalidResponse("mock script empty".into()))?
                    .clone_step();
                match step {
                    MockStep::Ok(resp) => Ok(resp),
                    MockStep::Err(err) => Err(err),
                }
            }
        }
    }
}

impl MockStep {
    fn clone_step(&self) -> MockStep {
        match self {
            MockStep::Ok(resp) => MockStep::Ok(resp.clone()),
            MockStep::Err(ProviderError::Network(m)) => {
                MockStep::Err(ProviderError::Network(m.clone()))
            }
            MockStep::Err(ProviderError::Auth(m)) => MockStep::Err(ProviderError::Auth(m.clone())),
            MockStep::Err(ProviderError::RateLimit(m)) => {
                MockStep::Err(ProviderError::RateLimit(m.clone()))
            }
            MockStep::Err(ProviderError::InvalidResponse(m)) => {
                MockStep::Err(ProviderError::InvalidResponse(m.clone()))
            }
        }
    }
}
