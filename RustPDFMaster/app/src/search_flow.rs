//! A debounce revision is not a native request slot. Cancelling observation
//! must never release an unfinished native operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revision(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket(u64, u64);

#[derive(Clone, Debug, Default)]
pub struct Flow {
    generation: u64,
    serial: u64,
    pending: bool,
    active: Option<Ticket>,
    suspended: bool,
    unobserved: bool,
}

impl Flow {
    pub fn input_blocked(&self, loaded: bool, busy: bool, editor: bool) -> bool {
        !loaded || editor || self.suspended || (busy && !self.active())
    }
    pub fn revision(&self) -> Option<Revision> {
        (!self.suspended).then_some(Revision(self.generation))
    }
    pub fn changed(&mut self) -> Option<Revision> {
        self.pending = false;
        if self.suspended {
            return None;
        }
        let Some(generation) = self.generation.checked_add(1) else {
            self.suspended = true;
            return None;
        };
        self.generation = generation;
        Some(Revision(generation))
    }
    pub fn queue(&mut self, revision: Revision) -> bool {
        if self.suspended || revision.0 != self.generation {
            return false;
        }
        self.pending = true;
        true
    }
    pub fn request(&mut self) {
        self.queue(Revision(self.generation));
    }
    pub fn can_begin(&self) -> bool {
        self.pending && self.active.is_none() && !self.suspended
    }
    pub fn active(&self) -> bool {
        self.active.is_some()
    }
    pub fn suspended(&self) -> bool {
        self.suspended
    }
    pub fn begin(&mut self) -> Option<Ticket> {
        if !self.can_begin() {
            return None;
        }
        let Some(serial) = self.serial.checked_add(1) else {
            self.suspended = true;
            self.pending = false;
            return None;
        };
        self.serial = serial;
        self.pending = false;
        let ticket = Ticket(self.generation, serial);
        self.active = Some(ticket);
        Some(ticket)
    }
    /// None means an unrelated/duplicate reply, not permission to clear busy.
    pub fn finish(&mut self, ticket: Ticket) -> Option<bool> {
        if self.active != Some(ticket) || self.unobserved {
            return None;
        }
        self.active = None;
        Some(!self.suspended && ticket.0 == self.generation)
    }
    pub fn suspend(&mut self, ticket: Ticket) -> bool {
        if self.active != Some(ticket) {
            return false;
        }
        self.suspended = true;
        self.unobserved = true;
        self.pending = false;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausted_generation_invalidates_reply_but_still_drains_observed_owner() {
        let mut flow = Flow {
            generation: u64::MAX,
            ..Flow::default()
        };
        flow.request();
        let active = flow.begin().unwrap();
        assert!(flow.changed().is_none());
        assert!(flow.suspended());
        assert_eq!(flow.finish(active), Some(false));
        assert!(!flow.active());
        assert!(flow.revision().is_none());
    }

    #[test]
    fn serial_exhaustion_never_wraps_or_starts_a_native_operation() {
        let mut flow = Flow {
            serial: u64::MAX,
            ..Flow::default()
        };
        flow.request();
        assert!(flow.begin().is_none());
        assert!(flow.suspended());
        assert!(!flow.active());
        assert!(!flow.can_begin());
    }
}
