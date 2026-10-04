//! No multiplexer: monitor-only. Owner: mux agent.
//!
//! Every capability is off and every operation says it needs a multiplexer
//! (`MuxError::Unsupported`); nothing is guessed or faked.
use std::path::Path;

use crate::{Caps, LayoutPlan, Mux, MuxError, Pane, PaneRef, Place};

pub struct NoMux;

impl Mux for NoMux {
    fn name(&self) -> &'static str {
        "none"
    }
    fn caps(&self) -> Caps {
        Caps::default()
    }
    fn panes(&self) -> Vec<Pane> {
        vec![]
    }
    fn open(&self, _cmd: &[String], _cwd: &Path, _place: Place) -> Result<PaneRef, MuxError> {
        Err(MuxError::Unsupported("opening a pane"))
    }
    fn focus(&self, _pane: &PaneRef) -> Result<(), MuxError> {
        Err(MuxError::Unsupported("focusing a pane"))
    }
    fn type_text(&self, _pane: &PaneRef, _text: &str) -> Result<(), MuxError> {
        Err(MuxError::Unsupported("typing into a pane"))
    }
    fn layout(&self, _plan: &LayoutPlan) -> Result<Vec<PaneRef>, MuxError> {
        Err(MuxError::Unsupported("making a layout"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_is_unsupported_and_says_so() {
        let m = NoMux;
        assert_eq!(m.name(), "none");
        assert_eq!(m.caps(), Caps::default());
        assert!(m.panes().is_empty());
        let pane = PaneRef { pane_id: "w1:p1".into(), tab_id: None, workspace_id: None };
        for err in [
            m.open(&["sh".into()], Path::new("/tmp"), Place::Workspace { label: "X".into() }).unwrap_err(),
            m.focus(&pane).unwrap_err(),
            m.type_text(&pane, "hi").unwrap_err(),
            m.layout(&LayoutPlan::default()).unwrap_err(),
        ] {
            assert!(matches!(err, MuxError::Unsupported(_)), "{err}");
            let msg = err.to_string();
            assert!(msg.contains("multiplexer") && msg.contains("needs one"), "{msg}");
        }
    }
}
