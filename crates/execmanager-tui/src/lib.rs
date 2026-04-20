pub mod app;
pub mod runtime;
pub mod terminal;

use app::{DashboardApp, ViewMode};
use execmanager_contracts::ExecutionId;
use execmanager_daemon::{ReplayError, RuntimeProjection};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Widget},
};
use runtime::{build_dashboard_model, detail_for_exec, DashboardView, PressureLevel};

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
    let width = 140;
    let height = 46;
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    render_dashboard_buffer(app, selected_detail, area, &mut buffer);
    buffer_to_string(&buffer, area, ansi_color)
}

pub fn render_dashboard_buffer(
    app: &DashboardApp,
    selected_detail: Option<&[String]>,
    area: Rect,
    buf: &mut Buffer,
) {
    let outer = Block::default().style(Style::default().bg(Color::Rgb(18, 20, 24)));
    outer.render(area, buf);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(18),
            Constraint::Length(9),
        ])
        .split(area);

    render_header(app, layout[0], buf);
    render_main(app, selected_detail, layout[1], buf);
    render_logs(app, selected_detail, layout[2], buf);
}

fn render_header(app: &DashboardApp, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .title(" ExecManager Dashboard ")
        .borders(Borders::ALL)
        .border_style(border_style());
    let inner = block.inner(area);
    block.render(area, buf);

    let header_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(44),
            Constraint::Length(22),
            Constraint::Length(18),
            Constraint::Min(24),
        ])
        .split(inner);

    let tab_spans = ViewMode::ALL
        .iter()
        .flat_map(|view| {
            let label = if *view == app.state.view {
                format!("[{}]", view.title())
            } else {
                view.title().to_string()
            };
            [
                Span::styled(
                    label,
                    Style::default()
                        .fg(if *view == app.state.view {
                            Color::Cyan
                        } else {
                            Color::Gray
                        })
                        .add_modifier(if *view == app.state.view {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::raw("  "),
            ]
        })
        .collect::<Vec<_>>();
    Paragraph::new(Line::from(tab_spans)).render(header_chunks[0], buf);

    stat_paragraph(
        format!("Running instances: {}", app.model.instances.len()),
        header_chunks[1],
        buf,
    );
    stat_paragraph(
        format!(
            "High pressure: {}",
            count_high_pressure(&app.model.instances)
        ),
        header_chunks[2],
        buf,
    );
    stat_paragraph(
        global_load_label(&app.model.instances),
        header_chunks[3],
        buf,
    );
}

fn render_main(
    app: &DashboardApp,
    selected_detail: Option<&[String]>,
    area: Rect,
    buf: &mut Buffer,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(area);

    render_instances_list(app, chunks[0], buf);
    render_detail_panels(app, selected_detail, chunks[1], buf);
}

fn render_instances_list(app: &DashboardApp, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .title(format!(" {} ", app.state.view.title()))
        .borders(Borders::ALL)
        .border_style(border_style());
    let inner = block.inner(area);
    block.render(area, buf);

    if app.active_view().is_empty() {
        let empty = Paragraph::new(Text::from(vec![
            Line::from(Span::styled(
                "No running instances",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "AI-triggered commands will appear here while they are still running.",
                Style::default().fg(Color::Gray),
            )),
        ]))
        .wrap(ratatui::widgets::Wrap { trim: true })
        .block(Block::default());
        empty.render(inner, buf);
        return;
    }

    let items = app
        .active_view()
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let style = row_style(row.pressure, index == app.state.selected_index);
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(row.title.clone(), style.add_modifier(Modifier::BOLD)),
                    Span::raw("  "),
                    Span::styled(runtime_fragment(&row.subtitle), style),
                ]),
                Line::from(Span::styled(
                    metadata_fragment(&row.subtitle),
                    style.fg(Color::Gray),
                )),
            ])
        })
        .collect::<Vec<_>>();

    List::new(items).render(inner, buf);
}

fn render_detail_panels(
    app: &DashboardApp,
    selected_detail: Option<&[String]>,
    area: Rect,
    buf: &mut Buffer,
) {
    let detail = app
        .selected()
        .map(|selected| selected.detail_lines.as_slice())
        .or(selected_detail)
        .unwrap_or(&[]);

    let block = Block::default()
        .title(format!(
            " Selected: {} ",
            app.selected()
                .map(|selected| selected.title.as_str())
                .unwrap_or("none")
        ))
        .borders(Borders::ALL)
        .border_style(border_style());
    let inner = block.inner(area);
    block.render(area, buf);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(9),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Min(6),
        ])
        .split(inner);

    render_detail_panel(
        "Identity",
        extract_section(
            detail,
            "Identity",
            &[
                "Runtime",
                "Runtime / Resources",
                "Service / Ports",
                "Selection Summary / Recent State",
                "Recent stdout / stderr",
                "Logs",
                "Recent events",
            ],
        )
        .join("\n"),
        chunks[0],
        buf,
    );
    render_detail_panel(
        "Runtime / Resources",
        compact_runtime_section(first_non_empty_section(
            detail,
            &[
                (
                    "Runtime / Resources",
                    &[
                        "Service / Ports",
                        "Selection Summary / Recent State",
                        "Recent stdout / stderr",
                        "Logs",
                        "Recent events",
                    ] as &[&str],
                ),
                (
                    "Runtime",
                    &[
                        "Service / Ports",
                        "Selection Summary / Recent State",
                        "Recent stdout / stderr",
                        "Logs",
                        "Recent events",
                    ],
                ),
            ],
        ))
        .join("\n"),
        chunks[1],
        buf,
    );
    render_detail_panel(
        "Service / Ports",
        extract_section(
            detail,
            "Service / Ports",
            &[
                "Selection Summary / Recent State",
                "Recent stdout / stderr",
                "Logs",
                "Recent events",
            ],
        )
        .join("\n"),
        chunks[2],
        buf,
    );

    let summary_text = if detail.is_empty() {
        Text::from(vec![
            Line::from(Span::styled(
                "Select a running instance to inspect its runtime, resources, services, and recent output.",
                Style::default().fg(Color::Gray),
            )),
        ])
    } else {
        let mut lines = detail
            .iter()
            .filter(|line| line.starts_with("replay degraded:") || line.starts_with("ghost "))
            .cloned()
            .collect::<Vec<_>>();
        lines.extend(extract_section(
            detail,
            "Selection Summary / Recent State",
            &["Recent stdout / stderr", "Logs", "Recent events"],
        ));
        if lines.is_empty() {
            lines = extract_section(detail, "Recent events", &[]);
        }
        if lines.is_empty() {
            lines = vec!["No recent state changes captured yet.".to_string()];
        }
        Text::from(lines.into_iter().map(Line::from).collect::<Vec<_>>())
    };

    Paragraph::new(summary_text)
        .block(
            Block::default()
                .title(" Selection Summary / Recent State ")
                .borders(Borders::ALL)
                .border_style(border_style()),
        )
        .wrap(ratatui::widgets::Wrap { trim: true })
        .render(chunks[3], buf);
}

fn render_logs(
    app: &DashboardApp,
    selected_detail: Option<&[String]>,
    area: Rect,
    buf: &mut Buffer,
) {
    let detail = app
        .selected()
        .map(|selected| selected.detail_lines.as_slice())
        .or(selected_detail)
        .unwrap_or(&[]);
    let logs = extract_logs(detail);

    let lines = if logs.is_empty() {
        vec![Line::from(Span::styled(
            "No logs yet",
            Style::default().fg(Color::Gray),
        ))]
    } else {
        logs.into_iter().map(Line::from).collect()
    };

    Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .title(" Recent stdout / stderr ")
                .borders(Borders::ALL)
                .border_style(border_style()),
        )
        .wrap(ratatui::widgets::Wrap { trim: false })
        .render(area, buf);
}

fn render_detail_panel(title: &str, content: String, area: Rect, buf: &mut Buffer) {
    Paragraph::new(content)
        .block(
            Block::default()
                .title(format!(" {title} "))
                .borders(Borders::ALL)
                .border_style(border_style()),
        )
        .wrap(ratatui::widgets::Wrap { trim: true })
        .render(area, buf);
}

fn stat_paragraph(text: String, area: Rect, buf: &mut Buffer) {
    Paragraph::new(text).render(area, buf);
}

fn count_high_pressure(instances: &[DashboardView]) -> usize {
    instances
        .iter()
        .filter(|row| {
            matches!(
                row.pressure,
                PressureLevel::Elevated | PressureLevel::Constrained
            )
        })
        .count()
}

fn global_load_label(instances: &[DashboardView]) -> String {
    let constrained = instances
        .iter()
        .filter(|row| row.pressure == PressureLevel::Constrained)
        .count();
    let elevated = instances
        .iter()
        .filter(|row| row.pressure == PressureLevel::Elevated)
        .count();
    format!("Global load: {constrained} constrained / {elevated} elevated")
}

fn row_style(pressure: PressureLevel, selected: bool) -> Style {
    let base = Style::default().fg(match pressure {
        PressureLevel::Healthy => Color::Green,
        PressureLevel::Elevated => Color::Yellow,
        PressureLevel::Constrained => Color::Red,
        PressureLevel::Unknown => Color::DarkGray,
    });
    if selected {
        base.bg(Color::Rgb(34, 40, 49)).add_modifier(Modifier::BOLD)
    } else {
        base
    }
}

fn border_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn runtime_fragment(subtitle: &str) -> String {
    subtitle
        .split('|')
        .next()
        .unwrap_or(subtitle)
        .trim()
        .to_string()
}

fn metadata_fragment(subtitle: &str) -> String {
    subtitle
        .split('|')
        .skip(1)
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" • ")
}

fn extract_section(detail: &[String], header: &str, stop_headers: &[&str]) -> Vec<String> {
    let mut in_section = false;
    let mut lines = Vec::new();
    for line in detail {
        if line == header {
            in_section = true;
            continue;
        }
        if in_section && stop_headers.iter().any(|stop| line == stop) {
            break;
        }
        if in_section && !line.is_empty() {
            lines.push(line.clone());
        }
    }
    lines
}

fn extract_logs(detail: &[String]) -> Vec<String> {
    let mut lines = extract_section(detail, "Logs", &["Recent events"]);
    if lines.is_empty() {
        lines = extract_section(detail, "Recent stdout / stderr", &["Recent events"]);
    }
    lines
}

fn first_non_empty_section(detail: &[String], sections: &[(&str, &[&str])]) -> Vec<String> {
    for (header, stop_headers) in sections {
        let lines = extract_section(detail, header, stop_headers);
        if !lines.is_empty() {
            return lines;
        }
    }
    Vec::new()
}

fn compact_runtime_section(lines: Vec<String>) -> Vec<String> {
    let mut ordered = Vec::new();
    let priorities = [
        "effective state:",
        "runtime state:",
        "runtime:",
        "policy:",
        "resource governance:",
        "current memory:",
        "recent peak memory:",
        "pressure:",
        "cpu usage:",
    ];

    for prefix in priorities {
        if let Some(line) = lines.iter().find(|line| line.starts_with(prefix)) {
            ordered.push(line.clone());
        }
    }

    if ordered.is_empty() {
        lines
            .into_iter()
            .filter(|line| line != "Resources" && !line.starts_with("observed state:"))
            .collect()
    } else {
        ordered
    }
}

fn buffer_to_string(buffer: &Buffer, area: Rect, ansi_color: bool) -> String {
    let mut out = String::new();
    for y in 0..area.height {
        let mut line = String::new();
        let mut current_style = Style::default();
        for x in 0..area.width {
            let cell = &buffer[(x, y)];
            if ansi_color && cell.style() != current_style {
                line.push_str(&ansi_style(cell.style()));
                current_style = cell.style();
            }
            line.push_str(cell.symbol());
        }
        if ansi_color && current_style != Style::default() {
            line.push_str("\x1b[0m");
        }
        out.push_str(line.trim_end());
        if y + 1 != area.height {
            out.push('\n');
        }
    }
    out
}

fn ansi_style(style: Style) -> String {
    let mut parts = Vec::new();
    if let Some(fg) = style.fg {
        parts.push(match fg {
            Color::Black => "30".to_string(),
            Color::Red => "31".to_string(),
            Color::Green => "32".to_string(),
            Color::Yellow => "33".to_string(),
            Color::Blue => "34".to_string(),
            Color::Magenta => "35".to_string(),
            Color::Cyan => "36".to_string(),
            Color::Gray => "37".to_string(),
            Color::DarkGray => "90".to_string(),
            Color::White => "97".to_string(),
            Color::Rgb(r, g, b) => format!("38;2;{r};{g};{b}"),
            _ => "39".to_string(),
        });
    }
    if let Some(bg) = style.bg {
        parts.push(match bg {
            Color::Black => "40".to_string(),
            Color::Red => "41".to_string(),
            Color::Green => "42".to_string(),
            Color::Yellow => "43".to_string(),
            Color::Blue => "44".to_string(),
            Color::Magenta => "45".to_string(),
            Color::Cyan => "46".to_string(),
            Color::Gray => "47".to_string(),
            Color::DarkGray => "100".to_string(),
            Color::White => "107".to_string(),
            Color::Rgb(r, g, b) => format!("48;2;{r};{g};{b}"),
            _ => "49".to_string(),
        });
    }
    if style.add_modifier.contains(Modifier::BOLD) {
        parts.push("1".to_string());
    }
    if parts.is_empty() {
        "\x1b[0m".to_string()
    } else {
        format!("\x1b[{}m", parts.join(";"))
    }
}
