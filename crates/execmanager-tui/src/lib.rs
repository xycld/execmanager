pub mod app;
pub mod runtime;
pub mod terminal;

use app::{DashboardApp, ViewMode};
use execmanager_contracts::ExecutionId;
use execmanager_daemon::{ReplayError, RuntimeProjection};
use runtime::{build_dashboard_model, detail_for_exec, DashboardView};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrimaryView {
    Instances,
    Services,
    History,
    GhostsReconcile,
    InstanceDetail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneUiState {
    pub selected_exec_id: Option<ExecutionId>,
    pub focused_view: PrimaryView,
    pub instance_scroll: usize,
    pub service_scroll: usize,
    pub history_scroll: usize,
    pub ghost_scroll: usize,
}

#[derive(Debug)]
pub enum RenderError {
    History(ReplayError),
    Terminal(String),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::History(error) => write!(f, "unable to render history pane: {error}"),
            Self::Terminal(error) => write!(f, "unable to run dashboard terminal: {error}"),
        }
    }
}

impl std::error::Error for RenderError {}

impl Default for PaneUiState {
    fn default() -> Self {
        Self {
            selected_exec_id: None,
            focused_view: PrimaryView::Instances,
            instance_scroll: 0,
            service_scroll: 0,
            history_scroll: 0,
            ghost_scroll: 0,
        }
    }
}

pub fn render_screen(
    projection: &RuntimeProjection,
    ui: &PaneUiState,
) -> Result<String, RenderError> {
    let model = build_dashboard_model(projection)?;
    let state = app::DashboardState {
        view: match ui.focused_view {
            PrimaryView::Instances | PrimaryView::InstanceDetail => ViewMode::Instances,
            PrimaryView::Services => ViewMode::Services,
            PrimaryView::History => ViewMode::History,
            PrimaryView::GhostsReconcile => ViewMode::Ghosts,
        },
        selected_index: 0,
        should_quit: false,
    };
    let app = DashboardApp { state, model };
    let selected_detail = ui
        .selected_exec_id
        .as_ref()
        .and_then(|exec_id| detail_for_exec(projection, exec_id.as_str()).ok().flatten());
    Ok(render_dashboard_with_detail(
        &app,
        false,
        selected_detail.as_deref(),
    ))
}

pub fn render_dashboard(app: &DashboardApp, ansi_color: bool) -> String {
    render_dashboard_with_detail(app, ansi_color, None)
}

pub fn render_dashboard_with_detail(
    app: &DashboardApp,
    ansi_color: bool,
    selected_detail: Option<&[String]>,
) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "ExecManager Dashboard | view: {} | ↑/↓ select | ←/→ switch | q quit",
        app.state.view.title()
    ));
    lines.push(String::new());

    let nav = ViewMode::ALL
        .iter()
        .map(|view| {
            if *view == app.state.view {
                format!("[{}]", view.title())
            } else {
                view.title().to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    lines.push(nav);
    lines.push(String::new());

    lines.push(app.state.view.title().to_string());
    if app.active_view().is_empty() {
        lines.push("- no items".to_string());
    } else {
        for (index, row) in app.active_view().iter().enumerate() {
            lines.push(render_row(
                index == app.state.selected_index,
                row,
                ansi_color,
            ));
        }
    }

    lines.push(String::new());
    lines.push("Instance Detail".to_string());
    if let Some(selected) = app.selected() {
        lines.extend(selected.detail_lines.iter().cloned());
    } else if let Some(selected_detail) = selected_detail {
        lines.extend(selected_detail.iter().cloned());
    } else {
        lines.push("no selected item".to_string());
    }

    lines.join("\n")
}

fn render_row(selected: bool, row: &DashboardView, ansi_color: bool) -> String {
    let marker = if selected { '>' } else { ' ' };
    let body = format!("{marker} {} — {}", row.title, row.subtitle);
    if ansi_color {
        format!("{}{}\x1b[0m", row.pressure.ansi_color(), body)
    } else {
        body
    }
}
