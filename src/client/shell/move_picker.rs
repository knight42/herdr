use super::*;
use crate::api::schema::{
    Method, PaneMoveDestination, PaneMoveParams, SplitDirection, TabTransferDestination,
    TabTransferParams,
};

impl ClientShellState {
    pub(super) fn open_move_pane_picker(&mut self) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let Some(pane) = snapshot
            .focused_pane_id
            .as_deref()
            .and_then(|pane_id| snapshot.panes.iter().find(|pane| pane.pane_id == pane_id))
        else {
            return;
        };
        let workspace_label = |workspace_id: &str| {
            snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.workspace_id == workspace_id)
                .map_or("", |workspace| workspace.label.as_str())
        };
        let mut tabs = snapshot
            .tabs
            .iter()
            .filter(|tab| tab.tab_id != pane.tab_id)
            .collect::<Vec<_>>();
        // The source workspace's tabs first, then the others in sidebar order.
        tabs.sort_by_key(|tab| {
            (
                tab.workspace_id != pane.workspace_id,
                snapshot
                    .workspaces
                    .iter()
                    .position(|workspace| workspace.workspace_id == tab.workspace_id),
                tab.number,
            )
        });
        let mut entries = tabs
            .into_iter()
            .map(|tab| {
                (
                    ClientMoveTarget::PaneToTab {
                        pane_id: pane.pane_id.clone(),
                        tab_id: tab.tab_id.clone(),
                    },
                    format!("{} / {}", workspace_label(&tab.workspace_id), tab.label),
                )
            })
            .collect::<Vec<_>>();
        entries.push((
            ClientMoveTarget::PaneToNewTab {
                pane_id: pane.pane_id.clone(),
                workspace_id: pane.workspace_id.clone(),
            },
            "new tab".to_owned(),
        ));
        entries.push((
            ClientMoveTarget::PaneToNewWorkspace {
                pane_id: pane.pane_id.clone(),
            },
            "new workspace".to_owned(),
        ));
        self.overlay = Some(ClientShellOverlay::MovePicker(ClientMovePickerOverlay {
            title: "move pane to",
            moves_pane: true,
            entries,
            selected: 0,
            query: TextEditor::default(),
            error: None,
            moving: false,
        }));
    }

    pub(super) fn open_move_tab_picker(&mut self) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let Some(tab) = snapshot
            .focused_tab_id
            .as_deref()
            .and_then(|tab_id| snapshot.tabs.iter().find(|tab| tab.tab_id == tab_id))
        else {
            return;
        };
        let mut entries = snapshot
            .workspaces
            .iter()
            .filter(|workspace| workspace.workspace_id != tab.workspace_id)
            .map(|workspace| {
                (
                    ClientMoveTarget::TabToWorkspace {
                        tab_id: tab.tab_id.clone(),
                        workspace_id: workspace.workspace_id.clone(),
                    },
                    workspace.label.clone(),
                )
            })
            .collect::<Vec<_>>();
        entries.push((
            ClientMoveTarget::TabToNewWorkspace {
                tab_id: tab.tab_id.clone(),
            },
            "new workspace".to_owned(),
        ));
        self.overlay = Some(ClientShellOverlay::MovePicker(ClientMovePickerOverlay {
            title: "move tab to",
            moves_pane: false,
            entries,
            selected: 0,
            query: TextEditor::default(),
            error: None,
            moving: false,
        }));
    }

    pub(super) fn insert_move_picker_text(&mut self, text: &str) -> bool {
        let Some(ClientShellOverlay::MovePicker(picker)) = self.overlay.as_mut() else {
            return false;
        };
        if !picker.moving && picker.query.insert(text) {
            picker.selected = 0;
        }
        true
    }

    pub(super) fn route_move_picker_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        use crossterm::event::KeyModifiers;

        let Some(ClientShellOverlay::MovePicker(picker)) = self.overlay.as_mut() else {
            return false;
        };
        if picker.moving {
            return true;
        }
        let (code, modifiers) = crate::config::normalize_key_combo((key.code, key.modifiers));
        // Checked before the query editor, which binds alt+d to delete-word.
        if code == KeyCode::Char('d') && modifiers == KeyModifiers::ALT {
            self.submit_move_picker(SplitDirection::Down, outcome);
            return true;
        }
        if let Some(content_changed) = picker.query.handle_key(key) {
            if content_changed {
                picker.selected = 0;
            }
            outcome.repaint = true;
            return true;
        }
        match code {
            KeyCode::Esc => {
                self.overlay = None;
                outcome.repaint = true;
            }
            KeyCode::Enter => self.submit_move_picker(SplitDirection::Right, outcome),
            KeyCode::Up => self.move_move_picker_selection(-1, outcome),
            KeyCode::Down => self.move_move_picker_selection(1, outcome),
            KeyCode::Char('n') if modifiers == KeyModifiers::CONTROL => {
                self.move_move_picker_selection(1, outcome)
            }
            KeyCode::Char('p') if modifiers == KeyModifiers::CONTROL => {
                self.move_move_picker_selection(-1, outcome)
            }
            _ => {}
        }
        true
    }

    pub(super) fn move_move_picker_selection(
        &mut self,
        delta: isize,
        outcome: &mut ClientShellInput,
    ) {
        let Some(ClientShellOverlay::MovePicker(picker)) = self.overlay.as_mut() else {
            return;
        };
        let last = picker.visible_indices().len().saturating_sub(1);
        picker.selected = picker.selected.saturating_add_signed(delta).min(last);
        outcome.repaint = true;
    }

    pub(super) fn submit_move_picker(
        &mut self,
        split: SplitDirection,
        outcome: &mut ClientShellInput,
    ) {
        let Some(ClientShellOverlay::MovePicker(picker)) = self.overlay.as_mut() else {
            return;
        };
        if picker.moving {
            return;
        }
        let Some(index) = picker.visible_indices().get(picker.selected).copied() else {
            return;
        };
        let name = picker.query.trim();
        let label = (!name.is_empty()).then(|| name.to_owned());
        let pane_move = |pane_id: &str, destination| {
            Method::PaneMove(PaneMoveParams {
                pane_id: pane_id.to_owned(),
                destination,
                focus: true,
            })
        };
        let tab_transfer = |tab_id: &str, destination| {
            Method::TabTransfer(TabTransferParams {
                tab_id: tab_id.to_owned(),
                destination,
                focus: true,
            })
        };
        let method = match &picker.entries[index].0 {
            ClientMoveTarget::PaneToTab { pane_id, tab_id } => pane_move(
                pane_id,
                PaneMoveDestination::Tab {
                    tab_id: tab_id.clone(),
                    target_pane_id: None,
                    split,
                    ratio: None,
                },
            ),
            ClientMoveTarget::PaneToNewTab {
                pane_id,
                workspace_id,
            } => pane_move(
                pane_id,
                PaneMoveDestination::NewTab {
                    workspace_id: Some(workspace_id.clone()),
                    label,
                },
            ),
            ClientMoveTarget::PaneToNewWorkspace { pane_id } => pane_move(
                pane_id,
                PaneMoveDestination::NewWorkspace {
                    label,
                    tab_label: None,
                },
            ),
            ClientMoveTarget::TabToWorkspace {
                tab_id,
                workspace_id,
            } => tab_transfer(
                tab_id,
                TabTransferDestination::Workspace {
                    workspace_id: workspace_id.clone(),
                },
            ),
            ClientMoveTarget::TabToNewWorkspace { tab_id } => {
                tab_transfer(tab_id, TabTransferDestination::NewWorkspace { label })
            }
        };
        picker.moving = true;
        picker.error = None;
        if !self.push_endpoint_method_with_kind(method, PendingEndpointKind::MovePicker, outcome) {
            if let Some(ClientShellOverlay::MovePicker(picker)) = self.overlay.as_mut() {
                picker.moving = false;
            }
        }
        outcome.repaint = true;
    }

    pub(super) fn handle_move_picker_endpoint_result(
        &mut self,
        result: Result<crate::api::schema::ResponseResult, ClientShellEndpointError>,
    ) -> bool {
        let Some(ClientShellOverlay::MovePicker(picker)) = self.overlay.as_mut() else {
            return false;
        };
        match result {
            Ok(_) => self.overlay = None,
            Err(error) => {
                picker.moving = false;
                picker.error = Some(error.message);
            }
        }
        true
    }
}
