use execmanager_tui::{
    app::{AppAction, DashboardApp, ViewMode},
    runtime::{DashboardModel, DashboardView, PressureLevel},
};

fn model() -> DashboardModel {
    DashboardModel {
        instances: vec![
            DashboardView::new(
                "exec-2",
                "python server.py",
                "12s | shell | /repo | healthy",
                PressureLevel::Healthy,
                vec!["detail for exec-2".to_string()],
            ),
            DashboardView::new(
                "exec-1",
                "npm run dev",
                "2m 1s | shell | /repo | elevated",
                PressureLevel::Elevated,
                vec!["detail for exec-1".to_string()],
            ),
        ],
        services: vec![DashboardView::new(
            "svc-1",
            "web",
            "exec-1 | 3000",
            PressureLevel::Unknown,
            vec!["service detail".to_string()],
        )],
        history: vec![DashboardView::new(
            "hist-1",
            "launch_requested",
            "offset 10",
            PressureLevel::Unknown,
            vec!["history detail".to_string()],
        )],
        ghosts: vec![DashboardView::new(
            "ghost-1",
            "exec-ghost",
            "detached | stale runtime",
            PressureLevel::Unknown,
            vec!["ghost detail".to_string()],
        )],
    }
}

#[test]
fn arrow_keys_move_selection_and_switch_views() {
    let mut app = DashboardApp::new(model());

    assert_eq!(app.state.view, ViewMode::Instances);
    assert_eq!(app.state.selected_index, 0);

    app.apply(AppAction::Down);
    assert_eq!(app.state.selected_index, 1);

    app.apply(AppAction::Right);
    assert_eq!(app.state.view, ViewMode::Services);
    assert_eq!(app.state.selected_index, 0);

    app.apply(AppAction::Left);
    assert_eq!(app.state.view, ViewMode::Instances);
}

#[test]
fn instances_view_shows_running_items_newest_first() {
    let app = DashboardApp::new(model());

    assert_eq!(app.active_view()[0].key(), "exec-2");
    assert_eq!(app.active_view()[1].key(), "exec-1");
}

#[test]
fn detail_follows_selected_instance() {
    let mut app = DashboardApp::new(model());

    assert!(app.selected().unwrap().detail_lines[0].contains("exec-2"));

    app.apply(AppAction::Down);

    assert!(app.selected().unwrap().detail_lines[0].contains("exec-1"));
}

#[test]
fn replacing_model_keeps_selected_instance_when_it_still_exists() {
    let mut app = DashboardApp::new(model());
    app.apply(AppAction::Down);

    let mut next = model();
    next.instances = vec![
        DashboardView::new(
            "exec-3",
            "cargo test",
            "1s | shell | /repo | healthy",
            PressureLevel::Healthy,
            vec!["detail for exec-3".to_string()],
        ),
        DashboardView::new(
            "exec-1",
            "npm run dev",
            "2m 2s | shell | /repo | elevated",
            PressureLevel::Elevated,
            vec!["detail for exec-1 (updated)".to_string()],
        ),
    ];

    app.replace_model(next);

    assert_eq!(app.selected().unwrap().key(), "exec-1");
    assert!(app.selected().unwrap().detail_lines[0].contains("updated"));
}

#[test]
fn replacing_model_drops_selected_instance_when_it_exits() {
    let mut app = DashboardApp::new(model());
    app.apply(AppAction::Down);

    let mut next = model();
    next.instances = vec![DashboardView::new(
        "exec-3",
        "cargo test",
        "1s | shell | /repo | healthy",
        PressureLevel::Healthy,
        vec!["detail for exec-3".to_string()],
    )];

    app.replace_model(next);

    assert_eq!(app.selected().unwrap().key(), "exec-3");
    assert_eq!(app.state.selected_index, 0);
}
