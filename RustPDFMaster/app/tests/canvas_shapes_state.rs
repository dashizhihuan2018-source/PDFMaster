use pdfmaster_ui::{canvas_shapes::*, canvas_shapes_state::*};
fn draft() -> Draft {
    Draft {
        geometry: Geometry::Rectangle {
            bounds: Rect {
                x: 10.,
                y: 10.,
                width: 30.,
                height: 20.,
            },
        },
        stroke: Style::default().stroke,
        fill: None,
        line_width: 2.,
        opacity: 1.,
    }
}
#[test]
fn pending_candidate_survives_only_exact_full_target_owner() {
    let mut s = Session::<String>::default();
    let target = Target::New {
        draft: draft(),
        revision: [3; 32],
    };
    let t = s.begin(7, 0, "r".into(), target.clone()).unwrap();
    let mut changed = draft();
    changed.opacity = 0.5;
    assert!(s.set_candidate(&t, changed.clone()));
    let mut wrong = target.clone();
    if let Target::New { revision, .. } = &mut wrong {
        revision[0] = 4;
    }
    assert!(!s.accept(
        &t,
        Owner {
            id: 1,
            request: "r".into(),
            page: 0,
            target: wrong
        }
    ));
    assert!(s.accept(
        &t,
        Owner {
            id: 1,
            request: "r".into(),
            page: 0,
            target
        }
    ));
    assert_eq!(s.candidate, Some(changed));
    assert_eq!(s.phase, Phase::Preparing);
    assert!(s.ready(&t));
}
fn page() -> Page {
    Page {
        index: 0,
        width: 200.,
        height: 120.,
        rotation: 0,
    }
}
fn target() -> Target {
    Target::New {
        draft: draft(),
        revision: [3; 32],
    }
}
fn existing() -> Target {
    Target::Existing {
        original: Selection {
            page_index: 0,
            page_id: (2, 0),
            object_id: Some((3, 0)),
            revision: [3; 32],
            flags: 0,
            opacity: 1.,
            original: Annotation {
                annotation_index: 0,
                shape: ShapeKind::Rectangle,
                bounds: Rect {
                    x: 10.,
                    y: 10.,
                    width: 30.,
                    height: 20.,
                },
                stroke: Style::default().stroke,
                fill: None,
                line_width: 2.,
                line: None,
            },
        },
    }
}
fn editing(target: Target) -> (Session<String>, Ticket, Owner) {
    let mut s = Session::default();
    let t = s.begin(7, 0, "r".into(), target.clone()).unwrap();
    let o = Owner {
        id: 1,
        request: "r".into(),
        page: 0,
        target,
    };
    assert!(s.accept(&t, o.clone()));
    assert!(s.ready(&t));
    (s, t, o)
}
#[test]
fn active_begin_ticket_full_owner_and_ready_gates() {
    let mut s = Session::<String>::default();
    let t = s.begin(7, 0, "r".into(), target()).unwrap();
    assert!(s.active());
    assert!(s.begin(8, 0, "q".into(), target()).is_none());
    assert!(!s.ready(&t));
    assert!(s.freeze(page()).unwrap().is_none());
    let o = Owner {
        id: 1,
        request: "r".into(),
        page: 0,
        target: target(),
    };
    for i in 0..4 {
        let mut wrong = o.clone();
        match i {
            0 => wrong.id = 0,
            1 => wrong.request = "x".into(),
            2 => wrong.page = 1,
            _ => {
                if let Target::New { draft, .. } = &mut wrong.target {
                    draft.opacity = 0.
                }
            }
        }
        assert!(!s.accept(&t, wrong));
    }
    for i in 0..4 {
        let mut wrong = t.clone();
        match i {
            0 => wrong.document_id += 1,
            1 => wrong.page += 1,
            2 => wrong.generation += 1,
            _ => wrong.request = "wrong".into(),
        }
        assert!(!s.owns(&wrong));
        assert!(!s.accept(&wrong, o.clone()));
        assert!(!s.set_candidate(&wrong, draft()));
    }
    assert!(s.accept(&t, o));
    assert!(s.freeze(page()).unwrap().is_none());
    assert!(s.ready(&t));
    assert!(!s.ready(&t));
}
#[test]
fn matching_sync_uses_immutable_owner_and_preserves_candidate() {
    let (mut s, t, o) = editing(existing());
    let mut d = draft();
    d.opacity = 0.;
    assert!(s.set_candidate(&t, d.clone()));
    assert!(s.matches_owner(7, &o));
    assert!(!s.matches_owner(8, &o));
    let mut wrong = o.clone();
    if let Target::Existing { original } = &mut wrong.target {
        original.page_id.0 += 1;
    }
    assert!(!s.matches_owner(7, &wrong));
    assert_eq!(s.candidate, Some(d));
}
#[test]
fn first_intent_validation_refusal_retains_input_and_owner() {
    let (mut s, t, o) = editing(target());
    s.queue(Some("first".into()));
    s.queue(None);
    s.queue(Some("second".into()));
    assert_eq!(s.intent.as_deref(), Some("first"));
    let mut invalid = draft();
    invalid.line_width = f32::NAN;
    assert!(s.set_candidate(&t, invalid));
    assert!(s.freeze(page()).is_err());
    assert!(s.candidate.as_ref().unwrap().line_width.is_nan());
    assert_eq!(s.intent, None);
    assert_eq!(s.phase, Phase::Editing);
    assert_eq!(s.owner, Some(o));
    s.reject_local("local".into());
    assert_eq!(s.error.as_deref(), Some("local"));
}
#[test]
fn immutable_commit_uncertainty_retries_and_explicit_refusal() {
    let (mut s, t, _) = editing(target());
    s.queue(Some("next".into()));
    let p = s.freeze(page()).unwrap().unwrap();
    assert_eq!(s.frozen, Some(Frozen::Commit(p.clone())));
    assert_eq!(s.phase, Phase::Committing);
    assert!(!s.set_candidate(&t, draft()));
    assert!(s.freeze(page()).unwrap().is_none());
    assert!(s.retry().is_none());
    s.uncertain(&t, "network".into());
    assert_eq!(s.phase, Phase::Uncertain);
    assert_eq!(s.intent.as_deref(), Some("next"));
    assert_eq!(s.retry(), Some(Frozen::Commit(p)));
    s.reject(&t, "refused".into());
    assert_eq!(s.phase, Phase::Editing);
    assert!(s.frozen.is_none());
    assert!(s.intent.is_none());
    assert!(s.set_candidate(&t, draft()));
}
#[test]
fn delete_is_distinct_exact_retry_new_refused_and_native_null_retained() {
    let (mut n, _, _) = editing(target());
    assert!(n.freeze_delete().is_err());
    let (mut s, t, o) = editing(existing());
    s.queue(Some("next".into()));
    let f = s.freeze_delete().unwrap().unwrap();
    assert_eq!(
        f,
        Frozen::Delete {
            id: 1,
            request: "r".into()
        }
    );
    s.uncertain(&t, "lost".into());
    assert_eq!(s.retry(), Some(f));
    assert!(!s.set_candidate(&t, draft()));
    assert!(s.delete_cancelled(&t));
    assert_eq!(s.phase, Phase::Editing);
    assert_eq!(s.owner, Some(o));
    assert_eq!(s.candidate, Some(draft()));
    assert!(s.intent.is_none());
    assert!(s.frozen.is_none());
    assert!(!s.delete_cancelled(&t));
}
#[test]
fn only_authoritative_current_finish_returns_first_intent_never_clears_successor() {
    let (mut s, t, _) = editing(target());
    s.queue(Some("first".into()));
    s.queue(Some("second".into()));
    assert_eq!(s.finish(&t, 8, true), None);
    assert_eq!(s.finish(&t, 7, false), None);
    assert!(s.active());
    assert_eq!(s.finish(&t, 7, true), Some("first".into()));
    assert!(!s.active());
    let next = s.begin(7, 0, "r".into(), target()).unwrap();
    assert!(next.generation > t.generation);
    assert_eq!(s.finish(&t, 7, true), None);
    s.reject(&t, "stale".into());
    s.uncertain(&t, "stale".into());
    assert!(s.owns(&next));
    assert_eq!(s.phase, Phase::BeginPending);
    assert!(s.error.is_none());
}
#[test]
fn locked_existing_freeze_and_wrong_page_refuse_without_losing_candidate() {
    let mut locked = existing();
    if let Target::Existing { original } = &mut locked {
        original.flags = 64;
    }
    let (mut s, _, _) = editing(locked);
    assert!(s.freeze(page()).is_err());
    assert!(s.freeze_delete().is_err());
    let (mut s, _, _) = editing(target());
    assert!(s.freeze(Page { index: 1, ..page() }).is_err());
    assert_eq!(s.candidate, Some(draft()));
}
#[test]
fn queued_finish_without_navigation_preserves_first_actual_intent() {
    let (mut s, t, _) = editing(target());
    s.queue(None);
    assert!(s.finish_requested);
    s.queue(Some("first".into()));
    s.queue(Some("second".into()));
    s.queue(None);
    assert_eq!(s.intent.as_deref(), Some("first"));
    assert_eq!(s.finish(&t, 7, true), Some("first".into()));
    assert_eq!(s.phase, Phase::Idle);
}

// Debug includes the private immutable target and generation as well as every
// public field, so a no-op assertion cannot overlook lost acknowledgement data.
fn snapshot(s: &Session<String>) -> String {
    format!("{s:?}")
}

fn frozen_wire(action: &Frozen) -> serde_json::Value {
    match action {
        Frozen::Commit(payload) => serde_json::to_value(payload).unwrap(),
        Frozen::Delete { id, request } => serde_json::json!({"id": id, "request": request}),
    }
}

fn assert_local_preserves_frozen(delete: bool, uncertain: bool) {
    let (mut s, t, _) = editing(existing());
    let mut candidate = draft();
    candidate.opacity = 0.5;
    assert!(s.set_candidate(&t, candidate));
    s.queue(Some("first".into()));
    let action = if delete {
        s.freeze_delete().unwrap().unwrap()
    } else {
        Frozen::Commit(s.freeze(page()).unwrap().unwrap())
    };
    if uncertain {
        s.uncertain(&t, "unknown native outcome".into());
    }
    let before = snapshot(&s);
    let wire = frozen_wire(&action);
    s.reject_local("unrelated local callback".into());
    assert_eq!(snapshot(&s), before);
    assert_eq!(s.frozen.as_ref().map(frozen_wire), Some(wire.clone()));
    assert!(!s.set_candidate(&t, draft()));
    if uncertain {
        let retry = s.retry().unwrap();
        assert_eq!(retry, action);
        assert_eq!(frozen_wire(&retry), wire);
    }
}

#[test]
fn local_refusal_preserves_committing_commit() {
    assert_local_preserves_frozen(false, false);
}
#[test]
fn local_refusal_preserves_uncertain_commit_and_exact_retry() {
    assert_local_preserves_frozen(false, true);
}
#[test]
fn local_refusal_preserves_committing_delete() {
    assert_local_preserves_frozen(true, false);
}
#[test]
fn local_refusal_preserves_uncertain_delete_and_exact_retry() {
    assert_local_preserves_frozen(true, true);
}

#[test]
fn pending_local_refusal_preserves_full_owner_acknowledgement() {
    let mut s = Session::<String>::default();
    let target = target();
    let t = s.begin(7, 0, "r".into(), target.clone()).unwrap();
    let mut candidate = draft();
    candidate.opacity = 0.5;
    assert!(s.set_candidate(&t, candidate.clone()));
    s.queue(Some("first".into()));
    s.error = Some("earlier diagnostic".into());
    let before = snapshot(&s);
    s.reject_local("not native refusal".into());
    assert_eq!(snapshot(&s), before);
    let owner: Owner = serde_json::from_value(serde_json::json!({
        "id": 1, "request": "r", "page": 0, "target": target
    }))
    .unwrap();
    assert!(s.accept(&t, owner));
    assert!(s.ready(&t));
    assert_eq!(s.candidate, Some(candidate));
    assert_eq!(s.intent.as_deref(), Some("first"));
}

#[test]
fn local_refusal_is_noop_in_idle_preparing_cancelling_and_handoff() {
    let unchanged: Vec<_> = [
        Phase::Idle,
        Phase::Preparing,
        Phase::Cancelling,
        Phase::Handoff,
    ]
    .into_iter()
    .map(|phase| {
        let (mut s, _, _) = editing(existing());
        s.queue(Some("first".into()));
        s.error = Some("earlier diagnostic".into());
        // Cancelling/Handoff are controller-assigned public phases.
        s.phase = phase;
        let before = snapshot(&s);
        s.reject_local("local".into());
        snapshot(&s) == before
    })
    .collect();
    assert_eq!(unchanged, vec![true; 4]);
}

#[test]
fn editing_local_refusal_clears_only_rejected_attempt_and_retains_input() {
    let (mut s, t, owner) = editing(existing());
    let mut candidate = draft();
    candidate.opacity = 0.5;
    assert!(s.set_candidate(&t, candidate.clone()));
    s.queue(Some("first".into()));
    s.reject_local("invalid local geometry".into());
    assert_eq!(s.phase, Phase::Editing);
    assert_eq!(s.ticket, Some(t.clone()));
    assert_eq!(s.owner, Some(owner.clone()));
    assert_eq!(s.candidate, Some(candidate));
    assert!(s.frozen.is_none());
    assert!(s.intent.is_none());
    assert!(!s.finish_requested);
    assert_eq!(s.error.as_deref(), Some("invalid local geometry"));
    assert!(s.matches_owner(7, &owner));
    assert!(s.freeze(page()).unwrap().is_some());
}

#[test]
fn native_refusal_releases_exact_commit_or_delete_in_safe_phases_only() {
    for delete in [false, true] {
        for phase in [Phase::Editing, Phase::Committing, Phase::Uncertain] {
            let (mut s, t, owner) = editing(existing());
            s.queue(Some("first".into()));
            if phase != Phase::Editing {
                if delete {
                    s.freeze_delete().unwrap().unwrap();
                } else {
                    s.freeze(page()).unwrap().unwrap();
                }
                if phase == Phase::Uncertain {
                    s.uncertain(&t, "unknown".into());
                }
            }
            let candidate = s.candidate.clone();
            for field in 0..4 {
                let mut wrong = t.clone();
                match field {
                    0 => wrong.document_id += 1,
                    1 => wrong.page += 1,
                    2 => wrong.generation += 1,
                    _ => wrong.request.push('x'),
                }
                let before = snapshot(&s);
                s.reject(&wrong, "wrong ticket".into());
                assert_eq!(snapshot(&s), before);
            }
            s.reject(&t, "native refusal".into());
            assert_eq!(s.phase, Phase::Editing);
            assert_eq!(s.ticket, Some(t));
            assert_eq!(s.owner, Some(owner));
            assert_eq!(s.candidate, candidate);
            assert!(s.frozen.is_none());
            assert!(s.intent.is_none());
            assert!(!s.finish_requested);
            assert_eq!(s.error.as_deref(), Some("native refusal"));
        }
    }
}

#[test]
fn native_refusal_preserves_unsafe_phases_without_losing_data() {
    let unchanged: Vec<_> = [
        Phase::Idle,
        Phase::BeginPending,
        Phase::Preparing,
        Phase::Cancelling,
        Phase::Handoff,
        Phase::Committing,
        Phase::Uncertain,
    ]
    .into_iter()
    .map(|phase| {
        let (mut s, t, _) = editing(existing());
        s.queue(Some("first".into()));
        s.error = Some("earlier diagnostic".into());
        s.phase = phase; // Committing/Uncertain here deliberately lack a frozen action.
        let before = snapshot(&s);
        s.reject(&t, "unsafe native refusal".into());
        snapshot(&s) == before
    })
    .collect();
    assert_eq!(unchanged, vec![true; 7]);
}

#[test]
fn native_refusal_preserves_action_when_accepted_owner_is_invalid() {
    let mut unchanged = Vec::new();
    for field in 0..5 {
        let (mut s, t, _) = editing(existing());
        s.freeze(page()).unwrap().unwrap();
        match field {
            0 => s.owner = None,
            1 => s.owner.as_mut().unwrap().id = 0,
            2 => s.owner.as_mut().unwrap().page += 1,
            3 => s.owner.as_mut().unwrap().request.push('x'),
            _ => {
                let Target::Existing { original } = &mut s.owner.as_mut().unwrap().target else {
                    panic!()
                };
                original.revision[0] += 1;
            }
        }
        let before = snapshot(&s);
        s.reject(&t, "invalid owner".into());
        unchanged.push(snapshot(&s) == before);
    }
    assert_eq!(unchanged, vec![true; 5]);
}

#[test]
fn cancelled_delete_preserves_candidate_and_owner_for_both_active_outcomes() {
    for uncertain in [false, true] {
        let (mut s, t, owner) = editing(existing());
        let mut candidate = draft();
        candidate.opacity = 0.5;
        s.set_candidate(&t, candidate.clone());
        s.queue(Some("first".into()));
        s.freeze_delete().unwrap().unwrap();
        if uncertain {
            s.uncertain(&t, "unknown".into());
        }
        let mut wrong = t.clone();
        wrong.generation += 1;
        let before = snapshot(&s);
        assert!(!s.delete_cancelled(&wrong));
        assert_eq!(snapshot(&s), before);
        assert!(s.delete_cancelled(&t));
        assert_eq!(s.phase, Phase::Editing);
        assert_eq!(s.owner, Some(owner));
        assert_eq!(s.candidate, Some(candidate));
        assert!(s.intent.is_none());
        assert!(!s.finish_requested);
        assert!(s.frozen.is_none());
        assert_eq!(s.error.as_deref(), Some("shape deletion cancelled"));
        assert!(s.set_candidate(&t, draft()));
    }
}
