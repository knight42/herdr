use super::*;
use crate::api::schema::{Method, PaneMoveDestination, SplitDirection, TabTransferDestination};

// ws_1 holds the focused tab_1 (pane_1) and tab_2 "logs"; ws_2 "api" holds tab_3 "server".
fn move_snapshot() -> ClientShellSnapshot {
    let mut snapshot = snapshot();
    let mut api = snapshot.workspaces[0].clone();
    api.workspace_id = "ws_2".into();
    api.active_tab_id = "tab_3".into();
    api.number = 2;
    api.label = "api".into();
    api.focused = false;
    snapshot.workspaces.push(api);
    let mut logs = snapshot.tabs[0].clone();
    logs.tab_id = "tab_2".into();
    logs.number = 2;
    logs.label = "logs".into();
    logs.focused = false;
    let mut server = logs.clone();
    server.tab_id = "tab_3".into();
    server.workspace_id = "ws_2".into();
    server.number = 1;
    server.label = "server".into();
    snapshot.tabs.extend([logs, server]);
    snapshot
}

fn open_picker(action: crate::input::KeybindAction) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(move_snapshot()));
    state.set_pane_surface(surface());
    let mut outcome = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Action(action), &mut outcome);
    assert!(
        outcome.actions.is_empty(),
        "the picker opens from the snapshot without a request"
    );
    state.compose(100, 30).expect("move picker frame");
    state
}

fn submitted(input: ClientShellInput) -> Method {
    let [ClientShellAction::Endpoint { request, .. }] = &input.actions[..] else {
        panic!("expected one endpoint request, got {:?}", input.actions);
    };
    request.method.clone()
}

#[test]
fn move_pane_lists_other_tabs_and_splits_right_or_down() {
    let mut state = open_picker(crate::input::KeybindAction::MovePaneToTab);
    // tab_2, tab_3, then new tab and new workspace; the source tab is not a destination.
    assert_eq!(state.hits.move_rows.len(), 4);

    state.handle_input_bytes(b"serv");
    let method = submitted(state.handle_input_bytes(b"\r"));
    assert_eq!(
        method,
        Method::PaneMove(crate::api::schema::PaneMoveParams {
            pane_id: "pane_1".into(),
            destination: PaneMoveDestination::Tab {
                tab_id: "tab_3".into(),
                target_pane_id: None,
                split: SplitDirection::Right,
                ratio: None,
            },
            focus: true,
        })
    );

    let mut state = open_picker(crate::input::KeybindAction::MovePaneToTab);
    state.handle_input_bytes(b"LOG");
    let method = submitted(state.handle_input_bytes(b"\x1bd"));
    assert!(matches!(
        method,
        Method::PaneMove(params) if params.destination
            == PaneMoveDestination::Tab {
                tab_id: "tab_2".into(),
                target_pane_id: None,
                split: SplitDirection::Down,
                ratio: None,
            }
    ));
}

#[test]
fn move_pane_names_new_tab_or_workspace_after_unmatched_query() {
    let mut state = open_picker(crate::input::KeybindAction::MovePaneToTab);
    state.handle_input_bytes(b"scratch");
    state.compose(100, 30).expect("filtered frame");
    assert_eq!(state.hits.move_rows.len(), 2, "only the create rows remain");
    let method = submitted(state.handle_input_bytes(b"\r"));
    assert!(matches!(
        method,
        Method::PaneMove(params) if params.destination
            == PaneMoveDestination::NewTab {
                workspace_id: Some("ws_1".into()),
                label: Some("scratch".into()),
            }
    ));

    let mut state = open_picker(crate::input::KeybindAction::MovePaneToTab);
    state.handle_input_bytes(b"scratch");
    state.handle_input_bytes(b"\x1b[B");
    let method = submitted(state.handle_input_bytes(b"\r"));
    assert!(matches!(
        method,
        Method::PaneMove(params) if params.destination
            == PaneMoveDestination::NewWorkspace {
                label: Some("scratch".into()),
                tab_label: None,
            }
    ));
}

#[test]
fn move_tab_transfers_the_whole_tab_to_a_picked_or_new_workspace() {
    let mut state = open_picker(crate::input::KeybindAction::MoveTabToWorkspace);
    // ws_2 and new workspace; the source workspace is not a destination.
    assert_eq!(state.hits.move_rows.len(), 2);
    let row = state.hits.move_rows[0].0;
    let clicked = state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: row.x + 2,
        row: row.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert_eq!(
        submitted(clicked),
        Method::TabTransfer(crate::api::schema::TabTransferParams {
            tab_id: "tab_1".into(),
            destination: TabTransferDestination::Workspace {
                workspace_id: "ws_2".into(),
            },
            focus: true,
        })
    );

    let mut state = open_picker(crate::input::KeybindAction::MoveTabToWorkspace);
    state.handle_input_bytes(b"fresh");
    let method = submitted(state.handle_input_bytes(b"\r"));
    assert!(matches!(
        method,
        Method::TabTransfer(params) if params.destination
            == TabTransferDestination::NewWorkspace { label: Some("fresh".into()) }
    ));
}

#[test]
fn move_picker_cancels_on_escape_and_shows_rejections_inline() {
    let mut state = open_picker(crate::input::KeybindAction::MovePaneToTab);
    state.handle_input_bytes(b"\x1b");
    assert!(state.overlay.is_none());

    let mut state = open_picker(crate::input::KeybindAction::MoveTabToWorkspace);
    let input = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &input.actions[..] else {
        panic!("enter should submit the transfer");
    };
    let request_id = request.id.clone();
    state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Err(ClientShellEndpointError {
            code: Some("tab_not_found".into()),
            message: "tab tab_1 not found".into(),
        }),
    );
    let Some(ClientShellOverlay::MovePicker(picker)) = &state.overlay else {
        panic!("a rejected move keeps the picker open");
    };
    assert_eq!(picker.error.as_deref(), Some("tab tab_1 not found"));
    assert!(!picker.moving);

    let input = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &input.actions[..] else {
        panic!("the picker can retry after a rejection");
    };
    let request_id = request.id.clone();
    state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Ok(crate::api::schema::ResponseResult::Ok {}),
    );
    assert!(state.overlay.is_none());
}
