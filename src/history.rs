//! Undo and redo. One step per gesture: a click, a drag, a recorded take.

use crate::song::Song;

const MAX_UNDO: usize = 200;

#[derive(Default)]
pub struct History {
    undo: Vec<Song>,
    redo: Vec<Song>,
    /// The song as it was before the gesture that is still going on.
    pending: Option<Song>,
    /// Set when the song was swapped by undo/redo/open, so that swap isn't
    /// itself recorded as an edit.
    rebase: bool,
}

impl History {
    /// Call once per frame after all edits. `holding` is true while the mouse
    /// is still down, so a whole drag becomes one undo step.
    pub fn track(&mut self, before: &Song, now: &Song, holding: bool) {
        if std::mem::take(&mut self.rebase) {
            self.pending = None;
            return;
        }
        if self.pending.is_none() && now != before {
            self.pending = Some(before.clone());
        }
        if !holding {
            self.commit(now);
        }
    }

    fn commit(&mut self, now: &Song) {
        if let Some(prev) = self.pending.take()
            && prev != *now
        {
            self.undo.push(prev);
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.pending.is_some()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self, song: &mut Song) -> bool {
        self.commit(song);
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(song, prev));
        self.rebase = true;
        true
    }

    pub fn redo(&mut self, song: &mut Song) -> bool {
        self.commit(song);
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(song, next));
        self.rebase = true;
        true
    }

    /// Treat this frame's change as the new starting point, not as an edit.
    pub fn absorb(&mut self) {
        self.rebase = true;
    }

    /// Forget everything (a different file was opened).
    pub fn reset(&mut self) {
        *self = History {
            rebase: true,
            ..History::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(song: &mut Song, step: usize) {
        song.scenes[0].drums[0][step] = !song.scenes[0].drums[0][step];
    }

    #[test]
    fn each_click_is_one_step() {
        let mut h = History::default();
        let mut song = Song::default();
        for step in 0..3 {
            let before = song.clone();
            edit(&mut song, step);
            h.track(&before, &song, false);
        }
        assert!(h.undo(&mut song));
        assert!(song.scenes[0].drums[0][1] && !song.scenes[0].drums[0][2]);
        assert!(h.undo(&mut song));
        assert!(h.undo(&mut song));
        assert_eq!(song, Song::default());
        assert!(!h.undo(&mut song));
    }

    #[test]
    fn a_drag_is_one_step() {
        let mut h = History::default();
        let mut song = Song::default();
        for step in 0..5 {
            let before = song.clone();
            edit(&mut song, step);
            h.track(&before, &song, true);
        }
        let before = song.clone();
        h.track(&before, &song, false);
        assert!(h.undo(&mut song));
        assert_eq!(song, Song::default());
        assert!(!h.undo(&mut song));
    }

    #[test]
    fn redo_comes_back_until_you_edit() {
        let mut h = History::default();
        let mut song = Song::default();
        let before = song.clone();
        edit(&mut song, 0);
        h.track(&before, &song, false);
        let edited = song.clone();

        let before = song.clone();
        assert!(h.undo(&mut song));
        h.track(&before, &song, false); // the undo itself is not an edit
        assert!(h.can_redo());
        let before = song.clone();
        assert!(h.redo(&mut song));
        h.track(&before, &song, false);
        assert_eq!(song, edited);

        let before = song.clone();
        assert!(h.undo(&mut song));
        h.track(&before, &song, false);
        let before = song.clone();
        edit(&mut song, 7);
        h.track(&before, &song, false);
        assert!(!h.can_redo(), "a new edit throws away redo");
    }
}
