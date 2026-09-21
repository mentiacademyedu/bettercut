//! Undo/redo (§11).
//!
//! §11 is explicit about what *not* to do: "Do not snapshot the entire project
//! after every edit." Each command stores only its own inverse, so the memory
//! cost of history is proportional to what changed, not to project size.

use std::collections::VecDeque;

use bettercut_project_format::Project;

use crate::command::EditorCommand;
use crate::error::EditorError;

/// §11 says 100–500. 300 is comfortably inside that and bounded enough that a
/// long session cannot grow history without limit.
pub const DEFAULT_HISTORY_LIMIT: usize = 300;

#[derive(Default)]
pub struct History {
    undo: VecDeque<Box<dyn EditorCommand>>,
    redo: Vec<Box<dyn EditorCommand>>,
    limit: usize,
}

impl std::fmt::Debug for History {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("History")
            .field("undo", &self.undo.len())
            .field("redo", &self.redo.len())
            .field("limit", &self.limit)
            .finish()
    }
}

impl History {
    pub fn new() -> Self {
        Self::with_limit(DEFAULT_HISTORY_LIMIT)
    }

    pub fn with_limit(limit: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            limit: limit.max(1),
        }
    }

    /// Execute a command and record it.
    ///
    /// A command that fails is **not** recorded: a failed edit must never become
    /// an undoable step, or undo would "restore" a state that never existed.
    pub fn execute(
        &mut self,
        mut command: Box<dyn EditorCommand>,
        project: &mut Project,
    ) -> Result<(), EditorError> {
        command.execute(project)?;

        // Any new edit invalidates the redo branch.
        self.redo.clear();

        self.undo.push_back(command);
        while self.undo.len() > self.limit {
            self.undo.pop_front();
        }
        Ok(())
    }

    /// Execute `command` as part of the step on top of the stack, so one undo
    /// takes both back — for a consequence of an edit, like a magnetic
    /// timeline closing the gap a delete left. With nothing on the stack it is
    /// a step of its own.
    pub fn amend_top(
        &mut self,
        mut command: Box<dyn EditorCommand>,
        project: &mut Project,
    ) -> Result<(), EditorError> {
        command.execute(project)?;
        self.redo.clear();
        let merged: Box<dyn EditorCommand> = match self.undo.pop_back() {
            Some(top) => {
                let label = top.label();
                Box::new(crate::command::CommandGroup::already_run(
                    label,
                    vec![top, command],
                ))
            }
            None => command,
        };
        self.undo.push_back(merged);
        Ok(())
    }

    /// Execute a command that continues the gesture already on top of the
    /// stack, replacing that entry instead of adding one (§11).
    ///
    /// Dragging a slider produces a value per frame. Recorded plainly that is
    /// sixty undo steps for one adjustment, and §11 is explicit that history
    /// holds user intentions rather than mouse samples.
    ///
    /// The previous entry is **undone** before the new one runs, so the single
    /// surviving entry restores the value from before the drag began — not
    /// from one frame earlier, which is what makes undo useless here.
    ///
    /// `same_gesture` decides. It is given the entry currently on top; the
    /// caller answers whether the incoming command continues it.
    pub fn execute_coalescing(
        &mut self,
        mut command: Box<dyn EditorCommand>,
        project: &mut Project,
        same_gesture: impl FnOnce(&dyn EditorCommand) -> bool,
    ) -> Result<(), EditorError> {
        let continues = self
            .undo
            .back()
            .is_some_and(|top| same_gesture(top.as_ref()));

        if continues && let Some(mut previous) = self.undo.pop_back() {
            // Roll the gesture back so the new command captures the value from
            // before it started. A failure here means the project moved under
            // us, so the entry is dropped rather than left half-applied.
            if let Err(err) = previous.undo(project) {
                tracing::warn!(?err, "could not coalesce; keeping the edit separate");
            }
        }

        command.execute(project)?;
        self.redo.clear();
        self.undo.push_back(command);
        while self.undo.len() > self.limit {
            self.undo.pop_front();
        }
        Ok(())
    }

    pub fn undo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let mut command = self.undo.pop_back().ok_or(EditorError::NothingToUndo)?;
        match command.undo(project) {
            Ok(()) => {
                self.redo.push(command);
                Ok(())
            }
            Err(err) => {
                // Undo failed: put it back so the stack still describes reality.
                self.undo.push_back(command);
                Err(err)
            }
        }
    }

    pub fn redo(&mut self, project: &mut Project) -> Result<(), EditorError> {
        let mut command = self.redo.pop().ok_or(EditorError::NothingToRedo)?;
        match command.execute(project) {
            Ok(()) => {
                self.undo.push_back(command);
                Ok(())
            }
            Err(err) => {
                self.redo.push(command);
                Err(err)
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Label for the undo menu item, e.g. `"Undo Add Clip"`.
    pub fn undo_label(&self) -> Option<String> {
        self.undo.back().map(|c| c.label())
    }

    pub fn redo_label(&self) -> Option<String> {
        self.redo.last().map(|c| c.label())
    }

    /// Every step Undo can reach, oldest first — the history panel's list.
    pub fn undo_labels(&self) -> Vec<String> {
        self.undo.iter().map(|c| c.label()).collect()
    }

    /// Every step Redo can reach, the next one first.
    pub fn redo_labels(&self) -> Vec<String> {
        self.redo.iter().rev().map(|c| c.label()).collect()
    }

    /// The most steps Undo keeps. Past it, the oldest are forgotten.
    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// Called after open/new: the previous document's history is meaningless.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::RenameProject;

    fn rename(name: &str) -> Box<dyn EditorCommand> {
        Box::new(RenameProject::new(name))
    }

    #[test]
    fn undo_and_redo_walk_the_stack() {
        let mut project = Project::new("original");
        let mut history = History::new();

        history.execute(rename("first"), &mut project).expect("ok");
        history.execute(rename("second"), &mut project).expect("ok");
        assert_eq!(project.name, "second");

        history.undo(&mut project).expect("ok");
        assert_eq!(project.name, "first");
        history.undo(&mut project).expect("ok");
        assert_eq!(project.name, "original");

        assert!(!history.can_undo());
        assert!(matches!(
            history.undo(&mut project),
            Err(EditorError::NothingToUndo)
        ));

        history.redo(&mut project).expect("ok");
        assert_eq!(project.name, "first");
        history.redo(&mut project).expect("ok");
        assert_eq!(project.name, "second");
        assert!(!history.can_redo());
    }

    #[test]
    fn a_new_edit_discards_the_redo_branch() {
        let mut project = Project::new("original");
        let mut history = History::new();

        history.execute(rename("a"), &mut project).expect("ok");
        history.undo(&mut project).expect("ok");
        assert!(history.can_redo());

        history.execute(rename("b"), &mut project).expect("ok");
        assert!(!history.can_redo(), "redo survived a divergent edit");
        assert_eq!(project.name, "b");
    }

    #[test]
    fn history_is_bounded_and_drops_the_oldest() {
        let mut project = Project::new("original");
        let mut history = History::with_limit(3);

        for i in 0..10 {
            history
                .execute(rename(&format!("n{i}")), &mut project)
                .expect("ok");
        }
        assert_eq!(history.undo_depth(), 3);

        // Only the last three are reversible; the rest have aged out.
        for _ in 0..3 {
            history.undo(&mut project).expect("ok");
        }
        assert_eq!(project.name, "n6");
        assert!(!history.can_undo());
    }

    #[test]
    fn labels_describe_the_next_action() {
        let mut project = Project::new("original");
        let mut history = History::new();
        assert_eq!(history.undo_label(), None);

        history.execute(rename("x"), &mut project).expect("ok");
        assert_eq!(history.undo_label().as_deref(), Some("Rename Project"));

        history.undo(&mut project).expect("ok");
        assert_eq!(history.redo_label().as_deref(), Some("Rename Project"));
    }

    #[test]
    fn clearing_drops_both_stacks() {
        let mut project = Project::new("original");
        let mut history = History::new();
        history.execute(rename("x"), &mut project).expect("ok");
        history.clear();
        assert!(!history.can_undo());
        assert!(!history.can_redo());
    }
}
