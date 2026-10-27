use std::{
    io,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub(crate) struct ImageExportControl {
    state: Arc<Mutex<State>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Active,
    Cancelled,
    Published,
}

pub(super) enum PublicationGateError {
    Cancelled,
    Io(io::Error),
}

impl ImageExportControl {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State::Active)),
        }
    }

    pub(crate) fn cancel(&self) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match *state {
            State::Active => {
                *state = State::Cancelled;
                true
            }
            State::Cancelled => true,
            State::Published => false,
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        *self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            == State::Cancelled
    }

    pub(crate) fn is_published(&self) -> bool {
        *self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            == State::Published
    }

    pub(super) fn publish(
        &self,
        rename: impl FnOnce() -> io::Result<()>,
    ) -> Result<(), PublicationGateError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match *state {
            State::Cancelled => Err(PublicationGateError::Cancelled),
            State::Published => Err(PublicationGateError::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "image directory was already published",
            ))),
            State::Active => match rename() {
                Ok(()) => {
                    *state = State::Published;
                    Ok(())
                }
                Err(error) => Err(PublicationGateError::Io(error)),
            },
        }
    }
}

impl Default for ImageExportControl {
    fn default() -> Self {
        Self::new()
    }
}
