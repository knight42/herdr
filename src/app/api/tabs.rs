use std::path::PathBuf;

use crate::api::schema::{
    EventData, EventEnvelope, EventKind, ResponseResult, TabCreateParams, TabListParams,
    TabMoveParams, TabRenameParams, TabTarget, TabTransferDestination, TabTransferParams,
};
use crate::app::{App, Mode};

use super::responses::{encode_error, encode_success};

impl App {
    pub(super) fn handle_tab_list(&mut self, id: String, params: TabListParams) -> String {
        let tabs = if let Some(workspace_id) = params.workspace_id {
            let Some(ws_idx) = self.parse_workspace_id(&workspace_id) else {
                return workspace_not_found(id, &workspace_id);
            };
            let Some(_) = self.state.workspaces.get(ws_idx) else {
                return workspace_not_found(id, &workspace_id);
            };
            self.tab_list_info(ws_idx)
        } else {
            let mut tabs = Vec::new();
            for (ws_idx, ws) in self.state.workspaces.iter().enumerate() {
                for tab_idx in 0..ws.tabs.len() {
                    if let Some(tab) = self.tab_info(ws_idx, tab_idx) {
                        tabs.push(tab);
                    }
                }
            }
            tabs
        };

        encode_success(id, ResponseResult::TabList { tabs })
    }

    pub(super) fn handle_tab_get(&mut self, id: String, target: TabTarget) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&target.tab_id) else {
            return tab_not_found(id, &target.tab_id);
        };
        let Some(tab) = self.tab_info(ws_idx, tab_idx) else {
            return tab_not_found(id, &target.tab_id);
        };

        encode_success(id, ResponseResult::TabInfo { tab })
    }

    pub(super) fn handle_tab_create(&mut self, id: String, params: TabCreateParams) -> String {
        let TabCreateParams {
            workspace_id,
            cwd,
            focus,
            label,
            env,
        } = params;
        let ws_idx = if let Some(workspace_id) = workspace_id {
            let Some(ws_idx) = self.parse_workspace_id(&workspace_id) else {
                return workspace_not_found(id, &workspace_id);
            };
            ws_idx
        } else if let Some(active) = self.state.active {
            active
        } else {
            return encode_error(id, "workspace_not_found", "no active workspace");
        };
        let cwd = cwd.map(PathBuf::from).unwrap_or_else(|| {
            self.resolve_new_terminal_cwd(self.focused_pane_cwd_in_workspace(ws_idx))
        });
        let (rows, cols) = self.state.new_pane_size(crate::ui::NewPanePlacement::Alone);
        let default_shell = self.state.default_shell.clone();
        let scrollback_limit_bytes = self.state.pane_scrollback_limit_bytes;
        let host_terminal_theme = self.state.host_terminal_theme;
        let host_terminal_appearance = self.state.host_terminal_appearance;
        let extra_env = match super::env::normalize_launch_env(env) {
            Ok(env) => env,
            Err((code, message)) => return encode_error(id, &code, message),
        };
        let result = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .ok_or_else(|| std::io::Error::other("workspace disappeared"))
            .and_then(|ws| {
                ws.create_tab(
                    rows,
                    cols,
                    cwd,
                    scrollback_limit_bytes,
                    host_terminal_theme,
                    host_terminal_appearance,
                    crate::pane::PaneShellConfig::new(&default_shell, self.state.shell_mode),
                    extra_env,
                )
            });
        match result {
            Ok((tab_idx, terminal, runtime)) => {
                self.terminal_runtimes.insert(terminal.id.clone(), runtime);
                self.state.terminals.insert(terminal.id.clone(), terminal);
                self.state.remove_alias_shadowed_by_new_pane(
                    self.state.workspaces[ws_idx].tabs[tab_idx].root_pane,
                );
                if let Some(label) = label {
                    let workspace_id = self.state.workspaces[ws_idx].id.clone();
                    let tab_id = self.public_tab_id(ws_idx, tab_idx).unwrap_or_else(|| {
                        crate::workspace::public_tab_id_for_number(&workspace_id, tab_idx + 1)
                    });
                    if let Some(tab) = self
                        .state
                        .workspaces
                        .get_mut(ws_idx)
                        .and_then(|ws| ws.tabs.get_mut(tab_idx))
                    {
                        tab.set_custom_name(label);
                        crate::logging::tab_renamed(&workspace_id, &tab_id);
                    }
                }
                if focus {
                    self.state.switch_workspace_tab(ws_idx, tab_idx);
                    self.state.mode = Mode::Terminal;
                }
                self.schedule_session_save();
                self.emit_tab_created_events(ws_idx, tab_idx);
                encode_success(
                    id,
                    self.tab_created_result(ws_idx, tab_idx)
                        .expect("new tab should produce a complete create response"),
                )
            }
            Err(err) => encode_error(id, "tab_create_failed", err.to_string()),
        }
    }

    pub(super) fn handle_tab_focus(&mut self, id: String, target: TabTarget) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&target.tab_id) else {
            return tab_not_found(id, &target.tab_id);
        };
        self.state.switch_workspace_tab(ws_idx, tab_idx);
        let tab = self.tab_info(ws_idx, tab_idx).unwrap();

        encode_success(id, ResponseResult::TabInfo { tab })
    }

    pub(super) fn handle_tab_rename(&mut self, id: String, params: TabRenameParams) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&params.tab_id) else {
            return tab_not_found(id, &params.tab_id);
        };
        let workspace_id = self.state.workspaces[ws_idx].id.clone();
        let tab_id = self.public_tab_id(ws_idx, tab_idx).unwrap_or_else(|| {
            crate::workspace::public_tab_id_for_number(&workspace_id, tab_idx + 1)
        });
        let Some(tab) = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .and_then(|ws| ws.tabs.get_mut(tab_idx))
        else {
            return tab_not_found(id, &params.tab_id);
        };
        tab.set_custom_name(params.label.clone());
        crate::logging::tab_renamed(&workspace_id, &tab_id);
        self.schedule_session_save();
        self.emit_event(EventEnvelope {
            event: EventKind::TabRenamed,
            data: EventData::TabRenamed {
                tab_id: self.public_tab_id(ws_idx, tab_idx).unwrap(),
                workspace_id: self.public_workspace_id(ws_idx),
                label: params.label,
            },
        });
        let tab = self.tab_info(ws_idx, tab_idx).unwrap();

        encode_success(id, ResponseResult::TabInfo { tab })
    }

    pub(super) fn handle_tab_move(&mut self, id: String, params: TabMoveParams) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&params.tab_id) else {
            return tab_not_found(id, &params.tab_id);
        };
        let Some(ws) = self.state.workspaces.get(ws_idx) else {
            return tab_not_found(id, &params.tab_id);
        };
        if params.insert_index > ws.tabs.len() {
            return encode_error(
                id,
                "tab_move_failed",
                format!("insert_index {} is out of bounds", params.insert_index),
            );
        }

        let tab_id = self
            .public_tab_id(ws_idx, tab_idx)
            .unwrap_or_else(|| crate::workspace::public_tab_id_for_number(&ws.id, tab_idx + 1));
        let workspace_id = self.public_workspace_id(ws_idx);
        let insert_index = params.insert_index;
        let moved = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .is_some_and(|ws| ws.move_tab(tab_idx, insert_index));
        let tabs = self.tab_list_info(ws_idx);
        if moved {
            self.schedule_session_save();
            self.emit_event(EventEnvelope {
                event: EventKind::TabMoved,
                data: EventData::TabMoved {
                    tab_id,
                    workspace_id,
                    insert_index,
                    tabs: tabs.clone(),
                },
            });
        }

        encode_success(id, ResponseResult::TabList { tabs })
    }

    pub(super) fn handle_tab_transfer(&mut self, id: String, params: TabTransferParams) -> String {
        let TabTransferParams {
            tab_id,
            destination,
            focus,
        } = params;
        let Some((source_ws_idx, tab_idx)) = self.parse_tab_id(&tab_id) else {
            return tab_not_found(id, &tab_id);
        };
        let Some(previous_tab_id) = self.public_tab_id(source_ws_idx, tab_idx) else {
            return tab_not_found(id, &tab_id);
        };
        let target_ws_idx = match &destination {
            TabTransferDestination::Workspace { workspace_id } => {
                let Some(target_ws_idx) = self.parse_workspace_id(workspace_id) else {
                    return workspace_not_found(id, workspace_id);
                };
                if target_ws_idx == source_ws_idx {
                    return encode_error(
                        id,
                        "tab_transfer_same_workspace",
                        "tab is already in that workspace",
                    );
                }
                Some(target_ws_idx)
            }
            TabTransferDestination::NewWorkspace { .. } => None,
        };
        let source_workspace_id = self.public_workspace_id(source_ws_idx);
        let source = &self.state.workspaces[source_ws_idx];
        let previous_pane_ids: Vec<_> = source.tabs[tab_idx]
            .layout
            .pane_ids()
            .into_iter()
            .filter_map(|pane_id| Some((self.public_pane_id(source_ws_idx, pane_id)?, pane_id)))
            .collect();
        let identity_cwd = source.tabs[tab_idx]
            .terminal_id(source.tabs[tab_idx].layout.focused())
            .and_then(|terminal_id| self.state.terminals.get(terminal_id))
            .map(|terminal| terminal.cwd.clone())
            .unwrap_or_else(|| source.identity_cwd.clone());
        let previous_focus = self.state.current_pane_focus_target();

        let Some((tab, source_empty)) =
            self.state.workspaces[source_ws_idx].take_tab_for_transfer(tab_idx)
        else {
            return tab_not_found(id, &tab_id);
        };
        let focused_pane = tab.layout.focused();
        let pane_ids = tab.layout.pane_ids();
        // Shells in the moved panes keep the HERDR_PANE_ID they were spawned with.
        self.state.public_pane_id_aliases.extend(previous_pane_ids);
        if source_empty {
            self.state.remove_workspace_emptied_by_move(source_ws_idx);
        }

        let (target_ws_idx, target_tab_idx, created_workspace) = match target_ws_idx {
            Some(target_ws_idx) => {
                let target_ws_idx = if source_empty && target_ws_idx > source_ws_idx {
                    target_ws_idx - 1
                } else {
                    target_ws_idx
                };
                let target_tab_idx =
                    self.state.workspaces[target_ws_idx].insert_transferred_tab(tab);
                (target_ws_idx, target_tab_idx, false)
            }
            None => {
                let label = match destination {
                    TabTransferDestination::NewWorkspace { label } => label,
                    TabTransferDestination::Workspace { .. } => None,
                };
                self.state
                    .workspaces
                    .push(crate::workspace::Workspace::from_existing_tab(
                        label,
                        identity_cwd,
                        tab,
                    ));
                (self.state.workspaces.len() - 1, 0, true)
            }
        };
        let target_workspace_id = self.public_workspace_id(target_ws_idx);
        let retarget = |target: Option<crate::app::state::PaneFocusTarget>| {
            target.map(|mut target| {
                if pane_ids.contains(&target.pane_id) {
                    target.workspace_id = target_workspace_id.clone();
                }
                target
            })
        };
        let previous_focus = retarget(previous_focus);
        self.state.previous_pane_focus = retarget(self.state.previous_pane_focus.take());
        for pane_id in &pane_ids {
            self.state.remove_alias_shadowed_by_new_pane(*pane_id);
        }
        if focus || self.state.active.is_none() {
            self.state
                .switch_workspace_tab(target_ws_idx, target_tab_idx);
            self.state
                .record_pane_focus_change(previous_focus, target_ws_idx, focused_pane);
            self.state.mode = Mode::Terminal;
        }
        self.state.mark_session_dirty();
        self.schedule_session_save();

        let Some(tab) = self.tab_info(target_ws_idx, target_tab_idx) else {
            return tab_not_found(id, &tab_id);
        };
        self.emit_event(EventEnvelope {
            event: EventKind::TabClosed,
            data: EventData::TabClosed {
                tab_id: previous_tab_id,
                workspace_id: source_workspace_id.clone(),
            },
        });
        if source_empty {
            self.emit_event(EventEnvelope {
                event: EventKind::WorkspaceClosed,
                data: EventData::WorkspaceClosed {
                    workspace_id: source_workspace_id,
                    workspace: None,
                },
            });
        }
        if created_workspace {
            self.emit_event(EventEnvelope {
                event: EventKind::WorkspaceCreated,
                data: EventData::WorkspaceCreated {
                    workspace: self.workspace_info(target_ws_idx),
                },
            });
        }
        self.emit_event(EventEnvelope {
            event: EventKind::TabCreated,
            data: EventData::TabCreated { tab: tab.clone() },
        });
        if let Some(layout) = self.pane_layout_snapshot(target_ws_idx, target_tab_idx) {
            self.emit_layout_updated_snapshot(layout);
        }

        encode_success(id, ResponseResult::TabInfo { tab })
    }

    pub(super) fn handle_tab_close(&mut self, id: String, target: TabTarget) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&target.tab_id) else {
            return tab_not_found(id, &target.tab_id);
        };
        let Some(tab_id) = self.public_tab_id(ws_idx, tab_idx) else {
            return tab_not_found(id, &target.tab_id);
        };
        let workspace_id = self.public_workspace_id(ws_idx);
        let Some(ws) = self.state.workspaces.get(ws_idx) else {
            return tab_not_found(id, &target.tab_id);
        };
        let closes_workspace = ws.tabs.len() <= 1;
        let terminal_ids = self.state.terminal_ids_for_tab(ws_idx, tab_idx);
        let pane_ids = ws
            .tabs
            .get(tab_idx)
            .map(|tab| tab.layout.pane_ids())
            .unwrap_or_default();

        if closes_workspace {
            if let Err(response) = self.require_restored_group_close_ready(
                &id,
                &self.state.workspace_close_indices(ws_idx),
            ) {
                return response;
            }
            if self.state.confirm_implicit_worktree_group_close(ws_idx) {
                return encode_error(
                    id,
                    "confirmation_required",
                    "closing this tab would close a worktree group",
                );
            }
            let workspace = self.workspace_info(ws_idx);
            self.state.selected = ws_idx;
            self.state.close_selected_workspace();
            self.state.remove_plugin_pane_records(pane_ids);
            self.shutdown_detached_terminal_runtimes();
            self.emit_event(EventEnvelope {
                event: EventKind::TabClosed,
                data: EventData::TabClosed {
                    tab_id,
                    workspace_id: workspace_id.clone(),
                },
            });
            self.emit_event(EventEnvelope {
                event: EventKind::WorkspaceClosed,
                data: EventData::WorkspaceClosed {
                    workspace_id,
                    workspace: Some(workspace),
                },
            });
            return encode_success(id, ResponseResult::Ok {});
        }

        let Some(ws) = self.state.workspaces.get_mut(ws_idx) else {
            return tab_not_found(id, &target.tab_id);
        };
        if !ws.close_tab(tab_idx) {
            return encode_error(
                id,
                "tab_close_failed",
                format!("tab {} could not be closed", target.tab_id),
            );
        }
        self.state.remove_plugin_pane_records(pane_ids);
        self.state.remove_unattached_terminal_ids(terminal_ids);
        self.shutdown_detached_terminal_runtimes();
        self.schedule_session_save();
        self.emit_event(EventEnvelope {
            event: EventKind::TabClosed,
            data: EventData::TabClosed {
                tab_id,
                workspace_id,
            },
        });

        encode_success(id, ResponseResult::Ok {})
    }

    fn tab_list_info(&self, ws_idx: usize) -> Vec<crate::api::schema::TabInfo> {
        self.state
            .workspaces
            .get(ws_idx)
            .map(|ws| {
                (0..ws.tabs.len())
                    .filter_map(|idx| self.tab_info(ws_idx, idx))
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn workspace_not_found(id: String, workspace_id: &str) -> String {
    encode_error(
        id,
        "workspace_not_found",
        format!("workspace {workspace_id} not found"),
    )
}

fn tab_not_found(id: String, tab_id: &str) -> String {
    encode_error(id, "tab_not_found", format!("tab {tab_id} not found"))
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{exiting_test_command, shutdown_test_runtimes};
    use super::*;
    use crate::{
        api::schema::SuccessResponse,
        config::{Config, ShellModeConfig},
        workspace::Workspace,
    };

    #[test]
    fn api_tab_close_last_tab_closes_workspace_and_emits_both_events() {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            event_hub.clone(),
        );
        app.state.workspaces = vec![Workspace::test_new("tabs")];
        app.state.active = Some(0);
        app.state.selected = 0;
        let tab_id = app.public_tab_id(0, 0).unwrap();
        let workspace_id = app.public_workspace_id(0);

        let response = app.handle_tab_close(
            "req".into(),
            TabTarget {
                tab_id: tab_id.clone(),
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(success.result, ResponseResult::Ok {});
        assert!(app.state.workspaces.is_empty());
        assert!(app.state.active.is_none());
        let events = event_hub.events_after(0);
        assert_eq!(
            events
                .iter()
                .map(|(_, event)| event.event)
                .collect::<Vec<_>>(),
            [EventKind::TabClosed, EventKind::WorkspaceClosed]
        );
        assert!(matches!(
            &events[0].1.data,
            EventData::TabClosed {
                tab_id: closed_tab_id,
                workspace_id: closed_workspace_id,
            } if closed_tab_id == &tab_id && closed_workspace_id == &workspace_id
        ));
        assert!(matches!(
            &events[1].1.data,
            EventData::WorkspaceClosed {
                workspace_id: closed_workspace_id,
                workspace: Some(workspace),
            } if closed_workspace_id == &workspace_id
                && workspace.workspace_id == workspace_id
        ));
    }

    fn app_with_workspaces(workspaces: Vec<Workspace>) -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = workspaces;
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.ensure_test_terminals();
        app
    }

    fn transfer(app: &mut App, params: TabTransferParams) -> serde_json::Value {
        serde_json::from_str(&app.handle_tab_transfer("req".into(), params)).unwrap()
    }

    #[test]
    fn api_tab_transfer_moves_live_panes_and_keeps_old_pane_ids_resolvable() {
        let mut source = Workspace::test_new("source");
        let moving_tab = source.test_add_tab(Some("moving"));
        source.active_tab = moving_tab;
        source.test_split(ratatui::layout::Direction::Horizontal);
        source.active_tab = 0;
        let mut app = app_with_workspaces(vec![source, Workspace::test_new("target")]);
        let moving_panes = app.state.workspaces[0].tabs[moving_tab].layout.pane_ids();
        let terminals: Vec<_> = moving_panes
            .iter()
            .map(|pane| {
                app.state.workspaces[0].tabs[moving_tab]
                    .terminal_id(*pane)
                    .unwrap()
                    .clone()
            })
            .collect();
        let old_pane_ids: Vec<_> = moving_panes
            .iter()
            .map(|pane| app.public_pane_id(0, *pane).unwrap())
            .collect();
        let tab_id = app.public_tab_id(0, moving_tab).unwrap();
        let target_workspace_id = app.public_workspace_id(1);

        let response = transfer(
            &mut app,
            TabTransferParams {
                tab_id: tab_id.clone(),
                destination: TabTransferDestination::Workspace {
                    workspace_id: target_workspace_id.clone(),
                },
                focus: true,
            },
        );

        let tab = &response["result"]["tab"];
        assert_eq!(tab["workspace_id"], target_workspace_id.as_str());
        assert_eq!(tab["label"], "moving");
        assert_eq!(tab["pane_count"], 2);
        assert_eq!(tab["focused"], true);
        assert_eq!(app.state.workspaces[0].tabs.len(), 1);
        assert_eq!(app.state.workspaces[1].tabs.len(), 2);
        let moved = &app.state.workspaces[1].tabs[1];
        assert_eq!(tab["tab_id"], app.public_tab_id(1, 1).unwrap().as_str());
        assert_eq!(moved.layout.pane_ids(), moving_panes);
        for (pane, terminal) in moving_panes.iter().zip(&terminals) {
            assert_eq!(moved.terminal_id(*pane), Some(terminal));
            assert!(app
                .public_pane_id(1, *pane)
                .unwrap()
                .starts_with(&format!("{target_workspace_id}:p")));
        }
        for (old, pane) in old_pane_ids.iter().zip(&moving_panes) {
            assert_eq!(app.parse_pane_id(old), Some((1, *pane)));
        }
        assert_eq!(app.state.active, Some(1));
        assert_eq!(app.state.workspaces[1].active_tab, 1);
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn api_tab_transfer_of_last_tab_closes_source_before_a_later_target() {
        let mut app = app_with_workspaces(vec![
            Workspace::test_new("source"),
            Workspace::test_new("middle"),
            Workspace::test_new("target"),
        ]);
        let moving_pane = app.state.workspaces[0].tabs[0].root_pane;
        let source_workspace_id = app.public_workspace_id(0);
        let target_workspace_id = app.public_workspace_id(2);

        let tab_id = app.public_tab_id(0, 0).unwrap();
        let response = transfer(
            &mut app,
            TabTransferParams {
                tab_id,
                destination: TabTransferDestination::Workspace {
                    workspace_id: target_workspace_id.clone(),
                },
                focus: false,
            },
        );

        assert_eq!(
            response["result"]["tab"]["workspace_id"],
            target_workspace_id.as_str()
        );
        assert_eq!(app.state.workspaces.len(), 2);
        assert!(app.parse_workspace_id(&source_workspace_id).is_none());
        let target_idx = app.parse_workspace_id(&target_workspace_id).unwrap();
        assert_eq!(app.state.workspaces[target_idx].tabs.len(), 2);
        assert_eq!(
            app.state.workspaces[target_idx].tabs[1].root_pane,
            moving_pane
        );
        let events = app.event_hub.events_after(0);
        assert!(events.iter().any(|(_, event)| matches!(
            &event.data,
            EventData::WorkspaceClosed { workspace_id, .. } if workspace_id == &source_workspace_id
        )));
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn api_tab_transfer_to_new_workspace_numbers_panes_from_one() {
        let mut source = Workspace::test_new("source");
        let moving_tab = source.test_add_tab(Some("moving"));
        let mut app = app_with_workspaces(vec![source]);
        let moving_pane = app.state.workspaces[0].tabs[moving_tab].root_pane;

        let tab_id = app.public_tab_id(0, moving_tab).unwrap();
        let response = transfer(
            &mut app,
            TabTransferParams {
                tab_id,
                destination: TabTransferDestination::NewWorkspace {
                    label: Some("promoted".into()),
                },
                focus: true,
            },
        );

        let workspace_id = response["result"]["tab"]["workspace_id"].as_str().unwrap();
        let ws_idx = app.parse_workspace_id(workspace_id).unwrap();
        assert_eq!(app.state.workspaces.len(), 2);
        assert_eq!(
            app.state.workspaces[ws_idx].custom_name.as_deref(),
            Some("promoted")
        );
        assert_eq!(
            app.state.workspaces[ws_idx].tabs[0].custom_name.as_deref(),
            Some("moving")
        );
        assert_eq!(
            app.public_pane_id(ws_idx, moving_pane),
            Some(format!("{workspace_id}:p1"))
        );
        assert_eq!(app.state.active, Some(ws_idx));
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn api_tab_transfer_rejects_unknown_tab_and_same_workspace() {
        let mut app = app_with_workspaces(vec![Workspace::test_new("only")]);
        let workspace_id = app.public_workspace_id(0);

        let unknown = transfer(
            &mut app,
            TabTransferParams {
                tab_id: format!("{workspace_id}:t99"),
                destination: TabTransferDestination::NewWorkspace { label: None },
                focus: false,
            },
        );
        let tab_id = app.public_tab_id(0, 0).unwrap();
        let same = transfer(
            &mut app,
            TabTransferParams {
                tab_id,
                destination: TabTransferDestination::Workspace { workspace_id },
                focus: false,
            },
        );

        assert_eq!(unknown["error"]["code"], "tab_not_found");
        assert_eq!(same["error"]["code"], "tab_transfer_same_workspace");
        assert_eq!(app.state.workspaces.len(), 1);
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn api_tab_move_reorders_tabs_in_target_workspace() {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            event_hub.clone(),
        );
        let mut workspace = Workspace::test_new("tabs");
        workspace.test_add_tab(Some("two"));
        workspace.test_add_tab(Some("three"));
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.selected = 0;
        let moved_root = app.state.workspaces[0].tabs[0].root_pane;
        let moved_id = app.public_tab_id(0, 0).unwrap();

        let response = app.handle_tab_move(
            "req".into(),
            TabMoveParams {
                tab_id: moved_id.clone(),
                insert_index: 3,
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::TabList { tabs } = success.result else {
            panic!("expected tab list");
        };
        assert_eq!(app.state.workspaces[0].tabs[2].root_pane, moved_root);
        assert_eq!(tabs[2].tab_id, app.public_tab_id(0, 2).unwrap());
        let events = event_hub.events_after(0);
        assert!(events.iter().any(|(_, event)| {
            matches!(
                &event.data,
                EventData::TabMoved {
                    tab_id,
                    workspace_id,
                    insert_index: 3,
                    tabs,
                } if tab_id == &moved_id
                    && workspace_id == &app.public_workspace_id(0)
                    && tabs[2].tab_id == moved_id
            )
        }));
    }

    #[tokio::test]
    async fn tab_create_follows_cached_focused_pane_cwd_without_runtime() {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            event_hub,
        );
        app.state.default_shell = exiting_test_command().into();
        app.state.shell_mode = ShellModeConfig::NonLogin;
        let workspace = Workspace::test_new("tabs");
        let focused_pane = workspace.tabs[0].root_pane;
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.ensure_test_terminals();
        let cached_cwd = std::env::temp_dir();
        let terminal_id = app.state.workspaces[0]
            .terminal_id(focused_pane)
            .cloned()
            .unwrap();
        app.state.terminals.get_mut(&terminal_id).unwrap().cwd = cached_cwd.clone();

        let response = app.handle_tab_create(
            "req".into(),
            TabCreateParams {
                workspace_id: None,
                cwd: None,
                focus: false,
                label: None,
                env: Default::default(),
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert!(matches!(success.result, ResponseResult::TabCreated { .. }));
        let created = &app.state.workspaces[0].tabs[1];
        let created_terminal_id = created.terminal_id(created.root_pane).unwrap();
        let created_cwd = &app.state.terminals.get(created_terminal_id).unwrap().cwd;
        assert_eq!(
            crate::worktree::canonical_or_original(created_cwd),
            crate::worktree::canonical_or_original(&cached_cwd)
        );
        shutdown_test_runtimes(&mut app);
    }
}
