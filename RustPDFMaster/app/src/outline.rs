use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Row {
    pub title: String,
    pub depth: usize,
    pub path: Vec<usize>,
    pub page_index: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Thumbnails,
    Outline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket(u64, u64);

#[derive(Clone, Debug, Default)]
pub struct Cache {
    generation: u64,
    serial: u64,
    active: Option<Ticket>,
    result: Option<Result<Vec<Row>, String>>,
    suspended: bool,
}

impl Cache {
    pub fn invalidate(&mut self) {
        self.result = None;
        if let Some(generation) = self.generation.checked_add(1) {
            self.generation = generation;
        } else {
            self.suspended = true;
        }
        // Keep the preceding native request slot until its reply is observed.
    }
    pub fn begin(&mut self) -> Option<Ticket> {
        if !self.can_begin() {
            return None;
        }
        let Some(serial) = self.serial.checked_add(1) else {
            self.suspended = true;
            return None;
        };
        self.serial = serial;
        let ticket = Ticket(self.generation, serial);
        self.active = Some(ticket);
        Some(ticket)
    }
    pub fn can_begin(&self) -> bool {
        !self.suspended && self.active.is_none() && self.result.is_none()
    }
    pub fn finish(&mut self, ticket: Ticket, result: Result<Vec<Row>, String>) -> bool {
        if self.suspended || self.active != Some(ticket) {
            return false;
        }
        self.active = None;
        if ticket.0 != self.generation {
            return false;
        }
        self.result = Some(result);
        true
    }
    pub fn suspend(&mut self, ticket: Ticket) {
        if self.active == Some(ticket) {
            self.suspended = true;
        }
    }
    pub fn result(&self) -> Option<&Result<Vec<Row>, String>> {
        self.result.as_ref()
    }
    pub fn suspended(&self) -> bool {
        self.suspended
    }
    pub fn page(&self, index: usize, page_count: usize, blocked: bool) -> Option<usize> {
        if blocked || self.suspended {
            return None;
        }
        self.result
            .as_ref()?
            .as_ref()
            .ok()?
            .get(index)?
            .page_index
            .filter(|page| *page < page_count)
    }
}
