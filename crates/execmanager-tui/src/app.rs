use crate::runtime::{DashboardModel, DashboardView};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    Up,
    Down,
    Left,
    Right,
    Quit,
    Refresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Instances,
    Services,
    History,
    Ghosts,
}

impl ViewMode {
    pub const ALL: [Self; 4] = [Self::Instances, Self::Services, Self::History, Self::Ghosts];

    pub fn title(self) -> &'static str {
        match self {
            Self::Instances => "Instances",
            Self::Services => "Services",
            Self::History => "History",
            Self::Ghosts => "Ghosts/Reconcile",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardState {
    pub view: ViewMode,
    pub selected_index: usize,
    pub should_quit: bool,
}

impl Default for DashboardState {
    fn default() -> Self {
        Self {
            view: ViewMode::Instances,
            selected_index: 0,
            should_quit: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DashboardApp {
    pub state: DashboardState,
    pub model: DashboardModel,
}

impl DashboardApp {
    pub fn new(model: DashboardModel) -> Self {
        let mut app = Self {
            state: DashboardState::default(),
            model,
        };
        app.clamp_selection();
        app
    }

    pub fn apply(&mut self, action: AppAction) {
        match action {
            AppAction::Up => {
                self.state.selected_index = self.state.selected_index.saturating_sub(1);
            }
            AppAction::Down => {
                let len = self.active_len();
                if len > 0 {
                    self.state.selected_index = (self.state.selected_index + 1).min(len - 1);
                }
            }
            AppAction::Left => {
                self.state.view = prev_view(self.state.view);
                self.state.selected_index = 0;
            }
            AppAction::Right => {
                self.state.view = next_view(self.state.view);
                self.state.selected_index = 0;
            }
            AppAction::Quit => {
                self.state.should_quit = true;
            }
            AppAction::Refresh => {}
        }

        self.clamp_selection();
    }

    pub fn replace_model(&mut self, model: DashboardModel) {
        let selected_key = self.selected_key().map(ToOwned::to_owned);
        self.model = model;
        if let Some(selected_key) = selected_key {
            if let Some(index) = self
                .active_view()
                .iter()
                .position(|row| row.key() == selected_key)
            {
                self.state.selected_index = index;
                return;
            }
        }
        self.clamp_selection();
    }

    pub fn active_view(&self) -> &[DashboardView] {
        self.model.view(self.state.view)
    }

    pub fn active_len(&self) -> usize {
        self.active_view().len()
    }

    pub fn selected(&self) -> Option<&DashboardView> {
        self.active_view().get(self.state.selected_index)
    }

    fn selected_key(&self) -> Option<&str> {
        self.selected().map(DashboardView::key)
    }

    fn clamp_selection(&mut self) {
        let len = self.active_len();
        if len == 0 {
            self.state.selected_index = 0;
        } else if self.state.selected_index >= len {
            self.state.selected_index = len - 1;
        }
    }
}

fn next_view(current: ViewMode) -> ViewMode {
    let index = ViewMode::ALL
        .iter()
        .position(|candidate| *candidate == current)
        .unwrap_or(0);
    ViewMode::ALL[(index + 1) % ViewMode::ALL.len()]
}

fn prev_view(current: ViewMode) -> ViewMode {
    let index = ViewMode::ALL
        .iter()
        .position(|candidate| *candidate == current)
        .unwrap_or(0);
    ViewMode::ALL[(index + ViewMode::ALL.len() - 1) % ViewMode::ALL.len()]
}
