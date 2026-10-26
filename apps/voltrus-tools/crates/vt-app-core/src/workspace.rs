use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use vt_protocol::SessionSpec;

use crate::{Axis, Layout};

macro_rules! id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        pub struct $name(pub u64);
    };
}

id!(ConnectionId);
id!(TabId);
id!(PaneId);

/// A saved device, broker or simulator the user can open.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub id: ConnectionId,
    pub name: String,
    pub spec: SessionSpec,
}

/// What a pane shows of its connection's session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneView {
    #[default]
    Points,
    Trend,
    Traffic,
    Log,
    /// A protocol's own view of the device (an S7 CPU's identity, mode,
    /// diagnostics and blocks). Offered only for connections whose
    /// protocol has one, so it is not in [`PaneView::ALL`].
    Device,
}

impl PaneView {
    pub const ALL: [PaneView; 4] = [
        PaneView::Points,
        PaneView::Trend,
        PaneView::Traffic,
        PaneView::Log,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Points => "Points",
            Self::Trend => "Trend",
            Self::Traffic => "Traffic",
            Self::Log => "Log",
            Self::Device => "Device",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pane {
    pub id: PaneId,
    pub connection: ConnectionId,
    pub view: PaneView,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub id: TabId,
    pub layout: Layout,
    pub focused: PaneId,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

/// Window chrome the user arranged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    pub sidebar_open: bool,
    pub sidebar_width: f32,
    pub inspector_open: bool,
    pub inspector_width: f32,
    pub appearance: Appearance,
    pub dark_theme: String,
    pub light_theme: String,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            sidebar_open: true,
            sidebar_width: 248.0,
            inspector_open: true,
            inspector_width: 300.0,
            appearance: Appearance::System,
            dark_theme: "Voltrus".into(),
            light_theme: "Voltrus Light".into(),
        }
    }
}

/// Everything the app restores on launch.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub connections: Vec<Connection>,
    pub panes: Vec<Pane>,
    pub tabs: Vec<Tab>,
    pub active_tab: Option<TabId>,
    pub ui: UiState,
    next_id: u64,
}

impl Workspace {
    fn allocate(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub fn connection(&self, id: ConnectionId) -> Option<&Connection> {
        self.connections
            .iter()
            .find(|connection| connection.id == id)
    }

    pub fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.panes.iter().find(|pane| pane.id == id)
    }

    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.active_tab.and_then(|id| self.tab(id))
    }

    /// The tab holding `pane`.
    pub fn tab_of(&self, pane: PaneId) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.layout.contains(pane))
    }

    pub fn add_connection(&mut self, name: impl Into<String>, spec: SessionSpec) -> ConnectionId {
        let id = ConnectionId(self.allocate());
        self.connections.push(Connection {
            id,
            name: name.into(),
            spec,
        });
        id
    }

    pub fn update_connection(&mut self, id: ConnectionId, name: String, spec: SessionSpec) -> bool {
        match self
            .connections
            .iter_mut()
            .find(|connection| connection.id == id)
        {
            Some(connection) => {
                connection.name = name;
                connection.spec = spec;
                true
            }
            None => false,
        }
    }

    /// Forget a connection and close every pane showing it.
    pub fn remove_connection(&mut self, id: ConnectionId) {
        self.connections.retain(|connection| connection.id != id);
        let doomed: Vec<PaneId> = self
            .panes
            .iter()
            .filter(|pane| pane.connection == id)
            .map(|pane| pane.id)
            .collect();
        for pane in doomed {
            self.close_pane(pane);
        }
    }

    /// Show `connection`: focus a pane already showing it, or open it in a
    /// new tab. Returns the pane.
    pub fn open(&mut self, connection: ConnectionId) -> Option<PaneId> {
        self.connection(connection)?;
        if let Some(pane) = self.panes.iter().find(|pane| pane.connection == connection) {
            let pane = pane.id;
            self.focus(pane);
            return Some(pane);
        }
        let pane = self.new_pane(connection, PaneView::Points);
        let tab = TabId(self.allocate());
        self.tabs.push(Tab {
            id: tab,
            layout: Layout::Leaf(pane),
            focused: pane,
        });
        self.active_tab = Some(tab);
        Some(pane)
    }

    /// Split the focused pane of the active tab and show `connection` in the
    /// new half. Opens a tab instead when none is open.
    pub fn split(
        &mut self,
        connection: ConnectionId,
        axis: Axis,
        view: PaneView,
    ) -> Option<PaneId> {
        self.connection(connection)?;
        let Some(tab_index) = self
            .active_tab
            .and_then(|active| self.tabs.iter().position(|tab| tab.id == active))
        else {
            return self.open(connection);
        };
        let pane = self.new_pane(connection, view);
        let tab = &mut self.tabs[tab_index];
        let target = tab.focused;
        if !tab.layout.split(target, pane, axis) {
            self.panes.retain(|existing| existing.id != pane);
            return None;
        }
        tab.focused = pane;
        Some(pane)
    }

    fn new_pane(&mut self, connection: ConnectionId, view: PaneView) -> PaneId {
        let pane = PaneId(self.allocate());
        self.panes.push(Pane {
            id: pane,
            connection,
            view,
        });
        pane
    }

    pub fn focus(&mut self, pane: PaneId) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.layout.contains(pane)) {
            tab.focused = pane;
            self.active_tab = Some(tab.id);
        }
    }

    pub fn set_view(&mut self, pane: PaneId, view: PaneView) {
        if let Some(pane) = self.panes.iter_mut().find(|existing| existing.id == pane) {
            pane.view = view;
        }
    }

    /// Close a pane; its tab closes with its last pane.
    pub fn close_pane(&mut self, pane: PaneId) {
        self.panes.retain(|existing| existing.id != pane);
        let Some(index) = self.tabs.iter().position(|tab| tab.layout.contains(pane)) else {
            return;
        };
        match self.tabs[index].layout.without(pane) {
            Some(layout) => {
                let tab = &mut self.tabs[index];
                if tab.focused == pane {
                    tab.focused = layout.panes()[0];
                }
                tab.layout = layout;
            }
            None => {
                let id = self.tabs[index].id;
                self.remove_tab(id);
            }
        }
    }

    /// Close a tab and every pane in it.
    pub fn close_tab(&mut self, id: TabId) {
        if let Some(tab) = self.tab(id) {
            let panes: HashSet<PaneId> = tab.layout.panes().into_iter().collect();
            self.panes.retain(|pane| !panes.contains(&pane.id));
            self.remove_tab(id);
        }
    }

    fn remove_tab(&mut self, id: TabId) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        self.tabs.remove(index);
        if self.active_tab == Some(id) {
            // The tab that slid into the closed one's place, else the one before.
            self.active_tab = self
                .tabs
                .get(index)
                .or_else(|| self.tabs.last())
                .map(|tab| tab.id);
        }
    }

    pub fn select_tab(&mut self, id: TabId) {
        if self.tab(id).is_some() {
            self.active_tab = Some(id);
        }
    }

    /// Select the tab `offset` places from the active one, wrapping around.
    pub fn cycle_tab(&mut self, offset: isize) {
        let count = self.tabs.len();
        if count == 0 {
            return;
        }
        let current = self
            .active_tab
            .and_then(|active| self.tabs.iter().position(|tab| tab.id == active))
            .unwrap_or(0);
        let count = isize::try_from(count).unwrap_or(isize::MAX);
        let current = isize::try_from(current).unwrap_or(0);
        let next = usize::try_from((current + offset).rem_euclid(count)).unwrap_or(0);
        self.active_tab = Some(self.tabs[next].id);
    }

    pub fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    /// Restore every invariant a stored or hand-edited file may break: panes
    /// point at existing connections and appear in exactly one tab, tabs are
    /// non-empty, focus and the active tab exist, ids never repeat.
    pub fn repair(&mut self) {
        let connections: HashSet<ConnectionId> = self
            .connections
            .iter()
            .map(|connection| connection.id)
            .collect();
        self.panes
            .retain(|pane| connections.contains(&pane.connection));
        let mut seen: HashSet<PaneId> = HashSet::new();
        let known: HashSet<PaneId> = self.panes.iter().map(|pane| pane.id).collect();
        self.tabs.retain_mut(|tab| {
            let Some(mut layout) = tab.layout.retain(&|pane| known.contains(&pane)) else {
                return false;
            };
            // A pane listed in two tabs stays in the first.
            let panes = layout.panes();
            if panes.iter().any(|pane| seen.contains(pane)) {
                match layout.retain(&|pane| !seen.contains(&pane)) {
                    Some(unique) => layout = unique,
                    None => return false,
                }
            }
            layout.sanitize();
            seen.extend(layout.panes());
            if !layout.contains(tab.focused) {
                tab.focused = layout.panes()[0];
            }
            tab.layout = layout;
            true
        });
        self.panes.retain(|pane| seen.contains(&pane.id));
        if self.active_tab().is_none() {
            self.active_tab = self.tabs.first().map(|tab| tab.id);
        }
        let highest = self
            .connections
            .iter()
            .map(|connection| connection.id.0)
            .chain(self.tabs.iter().map(|tab| tab.id.0))
            .chain(self.panes.iter().map(|pane| pane.id.0))
            .max()
            .unwrap_or(0);
        self.next_id = self.next_id.max(highest);
        let ui = &mut self.ui;
        if !ui.sidebar_width.is_finite() {
            ui.sidebar_width = UiState::default().sidebar_width;
        }
        if !ui.inspector_width.is_finite() {
            ui.inspector_width = UiState::default().inspector_width;
        }
        ui.sidebar_width = ui.sidebar_width.clamp(180.0, 480.0);
        ui.inspector_width = ui.inspector_width.clamp(240.0, 560.0);
    }
}

#[cfg(test)]
mod tests {
    use vt_protocol::{ModbusMasterSpec, ModbusSlaveSpec};

    use super::*;

    fn master() -> SessionSpec {
        SessionSpec::ModbusMaster(ModbusMasterSpec::default())
    }

    #[test]
    fn opening_twice_focuses_the_existing_pane() {
        let mut workspace = Workspace::default();
        let plc = workspace.add_connection("PLC", master());
        let first = workspace.open(plc).unwrap();
        let again = workspace.open(plc).unwrap();
        assert_eq!(first, again);
        assert_eq!(workspace.tabs.len(), 1);
    }

    #[test]
    fn removing_a_connection_closes_its_panes_and_empty_tabs() {
        let mut workspace = Workspace::default();
        let plc = workspace.add_connection("PLC", master());
        let sim = workspace.add_connection(
            "Simulator",
            SessionSpec::ModbusSlave(ModbusSlaveSpec::default()),
        );
        workspace.open(plc);
        let split = workspace
            .split(sim, Axis::Horizontal, PaneView::Points)
            .unwrap();
        workspace.open(sim);
        let only_sim = workspace.open(sim).unwrap();
        assert_eq!(
            only_sim, split,
            "the split pane already shows the simulator"
        );

        workspace.remove_connection(plc);
        assert_eq!(workspace.panes.len(), 1);
        assert_eq!(workspace.tabs.len(), 1);
        let tab = workspace.active_tab().unwrap();
        assert_eq!(tab.layout, Layout::Leaf(split));
        assert_eq!(tab.focused, split);

        workspace.remove_connection(sim);
        assert!(workspace.tabs.is_empty());
        assert_eq!(workspace.active_tab, None);
    }

    #[test]
    fn closing_the_active_tab_selects_its_neighbour() {
        let mut workspace = Workspace::default();
        let ids: Vec<_> = (0..3)
            .map(|index| workspace.add_connection(format!("PLC {index}"), master()))
            .collect();
        for id in &ids {
            workspace.open(*id);
        }
        let tabs: Vec<_> = workspace.tabs.iter().map(|tab| tab.id).collect();
        workspace.select_tab(tabs[1]);
        workspace.close_tab(tabs[1]);
        assert_eq!(workspace.active_tab, Some(tabs[2]));
        workspace.close_tab(tabs[2]);
        assert_eq!(workspace.active_tab, Some(tabs[0]));
    }

    #[test]
    fn repair_drops_dangling_and_duplicate_panes() {
        let mut workspace = Workspace::default();
        let plc = workspace.add_connection("PLC", master());
        let pane = workspace.open(plc).unwrap();
        let tab = workspace.active_tab.unwrap();
        // A second tab claiming the same pane, and a pane of a deleted connection.
        workspace.tabs.push(Tab {
            id: TabId(900),
            layout: Layout::Leaf(pane),
            focused: pane,
        });
        workspace.panes.push(Pane {
            id: PaneId(901),
            connection: ConnectionId(999),
            view: PaneView::Log,
        });
        workspace.active_tab = Some(TabId(12_345));
        // A file whose counter fell behind the ids it holds.
        workspace.next_id = 0;
        workspace.repair();
        assert_eq!(workspace.tabs.len(), 1);
        assert_eq!(workspace.panes.len(), 1);
        assert_eq!(workspace.active_tab, Some(tab));
        let next = workspace.add_connection("Next", master());
        assert!(next.0 > tab.0 && next.0 > pane.0 && next.0 > plc.0);
    }
}
