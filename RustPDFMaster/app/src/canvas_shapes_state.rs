//! Protected pure shape transaction state, separate from controller effects.
use crate::canvas_shapes::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Idle,
    BeginPending,
    Preparing,
    Editing,
    Committing,
    Uncertain,
    Cancelling,
    Handoff,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub generation: u64,
    pub document_id: u32,
    pub page: usize,
    pub request: String,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Frozen {
    Commit(Payload),
    Delete { id: u32, request: String },
}
#[derive(Clone, Debug)]
pub struct Session<I> {
    pub phase: Phase,
    pub ticket: Option<Ticket>,
    pub owner: Option<Owner>,
    pub candidate: Option<Draft>,
    pub frozen: Option<Frozen>,
    pub intent: Option<I>,
    pub finish_requested: bool,
    pub error: Option<String>,
    target: Option<Target>,
    generation: u64,
}
impl<I> Default for Session<I> {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            ticket: None,
            owner: None,
            candidate: None,
            frozen: None,
            intent: None,
            finish_requested: false,
            error: None,
            target: None,
            generation: 0,
        }
    }
}
impl<I> Session<I> {
    pub fn active(&self) -> bool {
        self.phase != Phase::Idle
    }
    pub fn begin(
        &mut self,
        document_id: u32,
        page: usize,
        request: String,
        target: Target,
    ) -> Option<Ticket> {
        if self.active() {
            return None;
        }
        let candidate = match &target {
            Target::New { draft, .. } => draft.clone(),
            Target::Existing { original } => {
                if original.page_index != page {
                    return None;
                }
                original.draft().ok()?
            }
        };
        self.generation = self.generation.checked_add(1)?;
        let ticket = Ticket {
            generation: self.generation,
            document_id,
            page,
            request,
        };
        self.phase = Phase::BeginPending;
        self.ticket = Some(ticket.clone());
        self.target = Some(target);
        self.candidate = Some(candidate);
        self.owner = None;
        self.frozen = None;
        self.intent = None;
        self.finish_requested = false;
        self.error = None;
        Some(ticket)
    }
    pub fn owns(&self, ticket: &Ticket) -> bool {
        self.active() && self.ticket.as_ref() == Some(ticket)
    }
    pub fn accept(&mut self, ticket: &Ticket, owner: Owner) -> bool {
        if !self.owns(ticket)
            || self.phase != Phase::BeginPending
            || owner.id == 0
            || owner.page != ticket.page
            || owner.request != ticket.request
            || Some(&owner.target) != self.target.as_ref()
        {
            return false;
        }
        self.owner = Some(owner);
        self.phase = Phase::Preparing;
        self.error = None;
        true
    }
    pub fn ready(&mut self, ticket: &Ticket) -> bool {
        if !self.owns(ticket) || self.phase != Phase::Preparing {
            return false;
        }
        self.phase = Phase::Editing;
        true
    }
    pub fn set_candidate(&mut self, ticket: &Ticket, draft: Draft) -> bool {
        if !self.owns(ticket)
            || !matches!(
                self.phase,
                Phase::BeginPending | Phase::Preparing | Phase::Editing
            )
        {
            return false;
        }
        self.candidate = Some(draft);
        true
    }
    pub fn matches_owner(&self, document_id: u32, owner: &Owner) -> bool {
        self.active()
            && self.ticket.as_ref().is_some_and(|t| {
                t.document_id == document_id && t.page == owner.page && t.request == owner.request
            })
            && self.owner.as_ref() == Some(owner)
            && Some(&owner.target) == self.target.as_ref()
    }
    pub fn queue(&mut self, intent: Option<I>) {
        if self.active() {
            self.finish_requested = true;
            if self.intent.is_none() {
                self.intent = intent;
            }
        }
    }
    fn valid_owner(&self) -> Result<&Owner, String> {
        let t = self.ticket.as_ref().ok_or("shape ticket absent")?;
        let owner = self.owner.as_ref().ok_or("shape owner absent")?;
        if owner.id == 0 || !self.matches_owner(t.document_id, owner) {
            return Err("shape ownership mismatch".into());
        }
        if let Target::Existing { original } = &owner.target
            && (original.page_index != t.page || original.flags & (64 | 128 | 512) != 0)
        {
            return Err("shape occurrence locked or on another page".into());
        }
        Ok(owner)
    }
    fn failed<T>(&mut self, error: String) -> Result<T, String> {
        self.reject_local(error.clone());
        Err(error)
    }
    pub fn freeze(&mut self, page: Page) -> Result<Option<Payload>, String> {
        if self.phase != Phase::Editing {
            return Ok(None);
        }
        let result = (|| {
            let owner = self.valid_owner()?;
            if owner.page != page.index {
                return Err("commit page mismatch".into());
            }
            let draft = self.candidate.clone().ok_or("shape candidate absent")?;
            draft.validate(page)?;
            Ok(Payload {
                id: owner.id,
                request: owner.request.clone(),
                draft,
            })
        })();
        match result {
            Ok(payload) => {
                self.frozen = Some(Frozen::Commit(payload.clone()));
                self.phase = Phase::Committing;
                self.error = None;
                Ok(Some(payload))
            }
            Err(e) => self.failed(e),
        }
    }
    pub fn freeze_delete(&mut self) -> Result<Option<Frozen>, String> {
        if self.phase != Phase::Editing {
            return Ok(None);
        }
        let result = (|| {
            let owner = self.valid_owner()?;
            if !matches!(owner.target, Target::Existing { .. }) {
                return Err("new shape cannot be deleted".into());
            }
            Ok(Frozen::Delete {
                id: owner.id,
                request: owner.request.clone(),
            })
        })();
        match result {
            Ok(frozen) => {
                self.frozen = Some(frozen.clone());
                self.phase = Phase::Committing;
                self.error = None;
                Ok(Some(frozen))
            }
            Err(e) => self.failed(e),
        }
    }
    pub fn retry(&mut self) -> Option<Frozen> {
        if self.phase != Phase::Uncertain {
            return None;
        }
        let action = self.frozen.clone()?;
        self.phase = Phase::Committing;
        self.error = None;
        Some(action)
    }
    pub fn uncertain(&mut self, ticket: &Ticket, error: String) {
        if self.owns(ticket)
            && matches!(self.phase, Phase::Committing | Phase::Uncertain)
            && self.frozen.is_some()
        {
            self.phase = Phase::Uncertain;
            self.error = Some(error);
        }
    }
    pub fn reject(&mut self, ticket: &Ticket, error: String) {
        if self.owns(ticket)
            && self.valid_owner().is_ok()
            && (self.phase == Phase::Editing
                || (matches!(self.phase, Phase::Committing | Phase::Uncertain)
                    && self.frozen.is_some()))
        {
            self.reset_editable(error);
        }
    }
    pub fn reject_local(&mut self, error: String) {
        if self.phase != Phase::Editing {
            return;
        }
        self.reset_editable(error);
    }
    fn reset_editable(&mut self, error: String) {
        self.phase = Phase::Editing;
        self.frozen = None;
        self.intent = None;
        self.finish_requested = false;
        self.error = Some(error);
    }
    pub fn delete_cancelled(&mut self, ticket: &Ticket) -> bool {
        if !self.owns(ticket)
            || !matches!(self.phase, Phase::Committing | Phase::Uncertain)
            || !matches!(self.frozen, Some(Frozen::Delete { .. }))
        {
            return false;
        }
        self.reset_editable("shape deletion cancelled".into());
        true
    }
    pub fn finish(&mut self, ticket: &Ticket, document_id: u32, owner_ended: bool) -> Option<I> {
        if !self.owns(ticket) || ticket.document_id != document_id || !owner_ended {
            return None;
        }
        let intent = self.intent.take();
        let generation = self.generation;
        *self = Self::default();
        self.generation = generation;
        intent
    }
}
