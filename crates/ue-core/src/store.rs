//! ProjectStore: the only gateway for mutating the project.
//! dispatch() applies a transaction of actions atomically (rollback if one
//! fails) and records it in the history.

use crate::action::{apply, Action};
use crate::error::{UeError, UeResult};
use crate::history::{History, HistoryEntry};
use crate::model::{Clip, Id, Project};
use crate::ops::{self, InsertMode};
use crate::time::TimeUs;
use crate::validate::validate;

pub struct ProjectStore {
    pub project: Project,
    history: History,
    /// Increments with each effective mutation (to keep mirrors in sync).
    pub version: u64,
    pub dirty: bool,
}

impl ProjectStore {
    pub fn new(project: Project) -> Self {
        ProjectStore { project, history: History::new(1000), version: 0, dirty: false }
    }

    /// Applies a transaction. Atomic: if an action fails, the previous ones are
    /// reverted and the project is left intact.
    pub fn dispatch(&mut self, label: &str, actions: Vec<Action>) -> UeResult<()> {
        if actions.is_empty() {
            return Ok(());
        }
        let mut inverses: Vec<Action> = Vec::with_capacity(actions.len());
        for action in &actions {
            match apply(&mut self.project, action.clone()) {
                Ok(inv) => inverses.push(inv),
                Err(e) => {
                    for inv in inverses.into_iter().rev() {
                        apply(&mut self.project, inv).expect("rollback must be infallible");
                    }
                    return Err(e);
                }
            }
        }
        debug_assert_eq!(
            validate(&self.project),
            Vec::<String>::new(),
            "invariants broken after '{label}'"
        );
        self.history.push(HistoryEntry { label: label.to_string(), actions, inverses });
        self.version += 1;
        self.dirty = true;
        Ok(())
    }

    pub fn undo(&mut self) -> UeResult<Option<String>> {
        let Some(entry) = self.history.pop_undo() else { return Ok(None) };
        for inv in entry.inverses.iter().rev() {
            apply(&mut self.project, inv.clone())
                .map_err(|e| UeError::Invalid(format!("inconsistent undo: {e}")))?;
        }
        let label = entry.label.clone();
        self.history.push_redo(entry);
        self.version += 1;
        self.dirty = true;
        Ok(Some(label))
    }

    pub fn redo(&mut self) -> UeResult<Option<String>> {
        let Some(mut entry) = self.history.pop_redo() else { return Ok(None) };
        let mut new_inverses = Vec::with_capacity(entry.actions.len());
        for action in &entry.actions {
            let inv = apply(&mut self.project, action.clone())
                .map_err(|e| UeError::Invalid(format!("inconsistent redo: {e}")))?;
            new_inverses.push(inv);
        }
        entry.inverses = new_inverses;
        let label = entry.label.clone();
        self.history.push_undo_from_redo(entry);
        self.version += 1;
        self.dirty = true;
        Ok(Some(label))
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }
    pub fn undo_labels(&self) -> Vec<&str> {
        self.history.undo_labels()
    }

    // ---- High-level operations (plan + dispatch) ----

    pub fn split_clip(&mut self, clip_id: Id, t: TimeUs) -> UeResult<(Id, Id)> {
        let (actions, l, r) = ops::split_clip(&self.project, clip_id, t)?;
        self.dispatch("Split clip", actions)?;
        Ok((l, r))
    }

    pub fn delete_clips(&mut self, ids: &[Id], ripple: bool) -> UeResult<()> {
        let actions = ops::delete_clips(&self.project, ids, ripple)?;
        let label = if ripple { "Delete (ripple)" } else { "Delete" };
        self.dispatch(label, actions)
    }

    pub fn insert_clip(&mut self, track_id: Id, clip: Clip, mode: InsertMode) -> UeResult<Id> {
        let clip_id = clip.id;
        let actions = ops::insert_clip(&self.project, track_id, clip, mode)?;
        self.dispatch("Add clip", actions)?;
        Ok(clip_id)
    }

    pub fn move_clip(
        &mut self,
        clip_id: Id,
        to_track: Id,
        to_start: TimeUs,
        mode: InsertMode,
    ) -> UeResult<()> {
        let actions = ops::move_clip(&self.project, clip_id, to_track, to_start, mode)?;
        self.dispatch("Move clip", actions)
    }

    pub fn trim_clip(&mut self, clip_id: Id, left: bool, new_edge: TimeUs) -> UeResult<()> {
        // also trim the linked clips at the same edge (one transaction)
        let mut actions = vec![];
        for gid in ops::linked_ids(&self.project, clip_id) {
            // planning each trim against the CURRENT project is correct:
            // linked clips live on different tracks and don't interfere
            match ops::trim_clip(&self.project, gid, left, new_edge) {
                Ok(a) => actions.extend(a),
                Err(_) if gid != clip_id => continue, // linked clip not trimmable: skipped
                Err(e) => return Err(e),
            }
        }
        self.dispatch("Trim clip", actions)
    }

    pub fn set_clip_speed(&mut self, clip_id: Id, speed: f64) -> UeResult<()> {
        let mut actions = vec![];
        for gid in ops::linked_ids(&self.project, clip_id) {
            match ops::set_clip_speed(&self.project, gid, speed) {
                Ok(a) => actions.extend(a),
                Err(_) if gid != clip_id => continue,
                Err(e) => return Err(e),
            }
        }
        self.dispatch("Change speed", actions)
    }

    pub fn speedup_ranges(
        &mut self,
        sequence_id: Id,
        ranges: &[(TimeUs, TimeUs)],
        factor: f64,
    ) -> UeResult<()> {
        let actions = ops::speedup_ranges(&self.project, sequence_id, ranges, factor)?;
        self.dispatch("Speed up ranges", actions)
    }

    pub fn move_range(
        &mut self,
        sequence_id: Id,
        from: TimeUs,
        to: TimeUs,
        dest: TimeUs,
    ) -> UeResult<()> {
        let actions = ops::move_range(&self.project, sequence_id, from, to, dest)?;
        self.dispatch("Move range", actions)
    }

    pub fn split_ranges(
        &mut self,
        sequence_id: Id,
        ranges: &[(TimeUs, TimeUs)],
    ) -> UeResult<()> {
        let actions = ops::split_ranges(&self.project, sequence_id, ranges)?;
        self.dispatch("Split at silences", actions)
    }

    pub fn cut_ranges(
        &mut self,
        sequence_id: Id,
        ranges: &[(TimeUs, TimeUs)],
        ripple: bool,
    ) -> UeResult<()> {
        let actions = ops::cut_ranges(&self.project, sequence_id, ranges, ripple)?;
        let label = format!("Cut {} range(s)", ranges.len());
        self.dispatch(&label, actions)
    }
}
