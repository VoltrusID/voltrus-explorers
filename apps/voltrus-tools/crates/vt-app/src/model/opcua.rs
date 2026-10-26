//! The OPC UA parts of the model: the editor's client fields with the endpoint picker and the
//! server simulator fields; each client's address-space browser (a lazily read tree the keyboard
//! walks), adding its variables to the points, typed writes and method calls; and the OPC UA demo.

use std::collections::{HashMap, HashSet};
use std::net::TcpListener;
use std::path::Path;

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, KeyBinding, PathPromptOptions,
    ScrollStrategy, Subscription, UniformListScrollHandle, Window, actions,
};
use vt_app_core::forms::{OpcuaAuthKind, OpcuaClientForm, OpcuaServerForm};
use vt_app_core::{Axis, ConnectionId, PaneView};
use vt_protocol::{
    Command as EngineCommand, OPCUA_PORT, OpcuaAttribute, OpcuaBrowseNode, OpcuaClientSpec,
    OpcuaEndpoint, OpcuaEvent, OpcuaMethodInfo, OpcuaNode, OpcuaNodeClass, OpcuaRequest,
    OpcuaSecurityMode, OpcuaSecurityPolicy, OpcuaServerSpec, SessionId, SessionSpec,
};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};

use super::AppModel;

actions!(
    opcua_browser,
    [
        BrowseUp,
        BrowseDown,
        BrowseExpand,
        BrowseCollapse,
        BrowseActivate,
        FocusBrowser,
    ]
);

/// Key context of a focused address-space browser.
pub const BROWSER: &str = "OpcuaBrowser";

/// The browser's keys: arrows walk the tree, Enter adds a variable (or opens a node), and
/// ⌘⇧B jumps into the focused pane's browser.
pub fn key_bindings() -> Vec<KeyBinding> {
    let primary = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    vec![
        KeyBinding::new("up", BrowseUp, Some(BROWSER)),
        KeyBinding::new("down", BrowseDown, Some(BROWSER)),
        KeyBinding::new("right", BrowseExpand, Some(BROWSER)),
        KeyBinding::new("left", BrowseCollapse, Some(BROWSER)),
        KeyBinding::new("enter", BrowseActivate, Some(BROWSER)),
        KeyBinding::new(
            &format!("{primary}-shift-b"),
            FocusBrowser,
            Some("Workspace && !Editor && !Palette"),
        ),
    ]
}

/// The browser's root: the Objects folder.
pub const ROOT: &str = "Objects";
/// Deepest level the tree draws, against reference cycles.
const MAX_DEPTH: usize = 24;
/// Method argument fields kept ready (a method with more is called from another client).
pub const MAX_ARGUMENTS: usize = 8;
/// Name of a new client until the user names it.
pub const NEW_CLIENT_NAME: &str = "OPC UA server";
pub const NEW_SERVER_NAME: &str = "OPC UA simulator";
/// Variables the demo client monitors from the start.
pub const DEMO_NODES: [(&str, &str); 6] = [
    ("Plant.Reactor.Temperature", "Reactor › Temperature"),
    ("Plant.Reactor.Setpoint", "Reactor › Setpoint"),
    ("Plant.Reactor.Pressure", "Reactor › Pressure"),
    ("Plant.Reactor.Level", "Reactor › Level"),
    ("Plant.Boiler.SteamPressure", "Boiler › Steam pressure"),
    ("Plant.Utilities.PowerDemand", "Utilities › Power demand"),
];
/// The plant's namespace on the simulator: the first one a server's node manager gets.
pub const PLANT_NAMESPACE_INDEX: u16 = 2;

/// Something the engine is fetching.
#[derive(Debug, Clone, PartialEq)]
pub enum Load<T> {
    Loading,
    Loaded(T),
    Failed(String),
}

/// The endpoint picker's last GetEndpoints.
pub enum Discovery {
    Idle,
    Searching(SessionId),
    Found(Vec<OpcuaEndpoint>),
    Failed(String),
}

/// Which certificate file a file prompt chooses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertificateFile {
    Certificate,
    PrivateKey,
}

pub struct OpcuaFields {
    // Client
    pub endpoint: Entity<TextInput>,
    pub policy: OpcuaSecurityPolicy,
    pub mode: OpcuaSecurityMode,
    pub auth: OpcuaAuthKind,
    pub username: Entity<TextInput>,
    pub password: Entity<TextInput>,
    pub certificate: Entity<TextInput>,
    pub private_key: Entity<TextInput>,
    pub publishing: Entity<TextInput>,
    pub sampling: Entity<TextInput>,
    pub nodes: Vec<OpcuaNode>,
    pub discovery: Discovery,
    // Server simulator
    pub server_port: Entity<TextInput>,
    pub anonymous: bool,
    pub server_user: Entity<TextInput>,
    pub server_password: Entity<TextInput>,
}

fn input(
    text: String,
    placeholder: &'static str,
    kind: InputKind,
    mono: bool,
    cx: &mut Context<AppModel>,
) -> Entity<TextInput> {
    cx.new(|cx| {
        let input = TextInput::new(cx)
            .text(text)
            .placeholder(placeholder)
            .kind(kind);
        if mono { input.mono() } else { input }
    })
}

fn secret(text: String, cx: &mut Context<AppModel>) -> Entity<TextInput> {
    cx.new(|cx| TextInput::new(cx).text(text).placeholder("none").secure())
}

impl OpcuaFields {
    /// Fields for `editing` when it is an OPC UA connection, defaults otherwise.
    pub fn new(editing: Option<&SessionSpec>, cx: &mut Context<AppModel>) -> Self {
        let client = match editing {
            Some(SessionSpec::OpcuaClient(spec)) => spec.clone(),
            _ => OpcuaClientSpec::default(),
        };
        let server = match editing {
            Some(SessionSpec::OpcuaServer(spec)) => spec.clone(),
            _ => OpcuaServerSpec::default(),
        };
        let client = OpcuaClientForm::from_spec("", &client);
        let server = OpcuaServerForm::from_spec("", &server);
        let text = InputKind::Text;
        let unsigned = InputKind::Unsigned;
        Self {
            endpoint: input(
                client.endpoint,
                "opc.tcp://192.168.1.20:4840",
                text,
                true,
                cx,
            ),
            policy: client.policy,
            mode: client.mode,
            auth: client.auth,
            username: input(client.username, "operator", text, false, cx),
            password: secret(client.password, cx),
            certificate: input(client.certificate, "user.der or user.pem", text, true, cx),
            private_key: input(client.private_key, "user.pem", text, true, cx),
            publishing: input(client.publishing_ms, "500", unsigned, true, cx),
            sampling: input(client.sampling_ms, "250", unsigned, true, cx),
            nodes: client.nodes,
            discovery: Discovery::Idle,
            server_port: input(server.port, "4840", unsigned, true, cx),
            anonymous: server.anonymous,
            server_user: input(server.username, "none", text, false, cx),
            server_password: secret(server.password, cx),
        }
    }

    /// Every text field, so the editor can watch them.
    pub fn inputs(&self) -> [Entity<TextInput>; 10] {
        [
            self.endpoint.clone(),
            self.username.clone(),
            self.password.clone(),
            self.certificate.clone(),
            self.private_key.clone(),
            self.publishing.clone(),
            self.sampling.clone(),
            self.server_port.clone(),
            self.server_user.clone(),
            self.server_password.clone(),
        ]
    }

    fn text(field: &Entity<TextInput>, cx: &App) -> String {
        field.read(cx).value().to_owned()
    }

    pub fn client_form(&self, name: String, cx: &App) -> OpcuaClientForm {
        OpcuaClientForm {
            name,
            endpoint: Self::text(&self.endpoint, cx),
            policy: self.policy,
            mode: self.mode,
            auth: self.auth,
            username: Self::text(&self.username, cx),
            password: Self::text(&self.password, cx),
            certificate: Self::text(&self.certificate, cx),
            private_key: Self::text(&self.private_key, cx),
            publishing_ms: Self::text(&self.publishing, cx),
            sampling_ms: Self::text(&self.sampling, cx),
            nodes: self.nodes.clone(),
        }
    }

    pub fn server_form(&self, name: String, cx: &App) -> OpcuaServerForm {
        OpcuaServerForm {
            name,
            port: Self::text(&self.server_port, cx),
            anonymous: self.anonymous,
            username: Self::text(&self.server_user, cx),
            password: Self::text(&self.server_password, cx),
        }
    }

    /// Whether the security fields match `endpoint`.
    pub fn picked(&self, endpoint: &OpcuaEndpoint) -> bool {
        endpoint.policy == Some(self.policy) && endpoint.mode == self.mode
    }
}

/// A node the browser has seen.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeNode {
    pub display_name: String,
    pub class: OpcuaNodeClass,
    pub data_type: String,
    pub has_children: bool,
}

/// What a row of the drawn tree shows.
#[derive(Debug, Clone, PartialEq)]
pub enum RowKind {
    Node,
    /// The children of `parent` are on their way.
    Loading,
    Failed(String),
    /// `parent` turned out to have no children.
    Empty,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub depth: usize,
    pub parent: String,
    /// Empty unless the row is a node.
    pub node: String,
    pub kind: RowKind,
}

/// A method picked in the browser, ready to call.
pub struct MethodPanel {
    pub object_id: String,
    pub method_id: String,
    pub info: Load<OpcuaMethodInfo>,
    pub calling: bool,
    pub result: Option<Result<Vec<String>, String>>,
}

/// One client's view of its server's address space.
pub struct Browser {
    /// The session the tree was read through; a new session reads it again.
    pub session: Option<SessionId>,
    pub nodes: HashMap<String, TreeNode>,
    pub children: HashMap<String, Load<Vec<String>>>,
    pub expanded: HashSet<String>,
    /// Parent and node of the selected row.
    pub selected: Option<(String, String)>,
    pub attributes: Option<(String, Load<Vec<OpcuaAttribute>>)>,
    pub method: Option<MethodPanel>,
    /// Variables asked to be monitored, until the session answers.
    pub pending: Vec<OpcuaNode>,
    /// What the last add did, for the browser's footer.
    pub notice: Option<(bool, String)>,
    pub open: bool,
    pub scroll: UniformListScrollHandle,
}

impl Default for Browser {
    fn default() -> Self {
        Self {
            session: None,
            nodes: HashMap::new(),
            children: HashMap::new(),
            expanded: HashSet::new(),
            selected: None,
            attributes: None,
            method: None,
            pending: Vec::new(),
            notice: None,
            open: true,
            scroll: UniformListScrollHandle::new(),
        }
    }
}

impl Browser {
    /// The tree as drawn: every loaded level under the root, expanded nodes opened.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut path = vec![ROOT.to_owned()];
        self.walk(ROOT, 0, &mut path, &mut rows);
        rows
    }

    fn walk(&self, parent: &str, depth: usize, path: &mut Vec<String>, rows: &mut Vec<Row>) {
        let row = |node: &str, kind: RowKind| Row {
            depth,
            parent: parent.to_owned(),
            node: node.to_owned(),
            kind,
        };
        match self.children.get(parent) {
            None => {}
            Some(Load::Loading) => rows.push(row("", RowKind::Loading)),
            Some(Load::Failed(error)) => rows.push(row("", RowKind::Failed(error.clone()))),
            Some(Load::Loaded(children)) if children.is_empty() => {
                rows.push(row("", RowKind::Empty));
            }
            Some(Load::Loaded(children)) => {
                for child in children {
                    rows.push(row(child, RowKind::Node));
                    if self.expanded.contains(child)
                        && depth < MAX_DEPTH
                        && !path.iter().any(|ancestor| ancestor == child)
                    {
                        path.push(child.clone());
                        self.walk(child, depth + 1, path, rows);
                        path.pop();
                    }
                }
            }
        }
    }

    /// The row index of the selection, if it is drawn.
    pub fn selected_row(&self, rows: &[Row]) -> Option<usize> {
        let (parent, node) = self.selected.as_ref()?;
        rows.iter()
            .position(|row| row.kind == RowKind::Node && &row.parent == parent && &row.node == node)
    }

    /// Take a browse answer: the children of `parent`.
    pub fn take_children(&mut self, parent: String, result: Result<Vec<OpcuaBrowseNode>, String>) {
        let loaded = match result {
            Ok(mut children) => {
                // Servers list references in any order: objects, then variables, then methods,
                // each by name, so the tree reads the same every time.
                let rank = |class: OpcuaNodeClass| match class {
                    OpcuaNodeClass::Object => 0,
                    OpcuaNodeClass::Variable => 1,
                    OpcuaNodeClass::Method => 2,
                    _ => 3,
                };
                children.sort_by_cached_key(|child| {
                    (rank(child.class), child.display_name.to_lowercase())
                });
                let mut ids = Vec::with_capacity(children.len());
                for child in children {
                    // A node reached twice in one level (two references) is listed once.
                    if ids.contains(&child.node_id) {
                        continue;
                    }
                    ids.push(child.node_id.clone());
                    self.nodes.insert(
                        child.node_id,
                        TreeNode {
                            display_name: if child.display_name.is_empty() {
                                child.browse_name
                            } else {
                                child.display_name
                            },
                            class: child.class,
                            data_type: child.data_type,
                            has_children: child.has_children,
                        },
                    );
                }
                Load::Loaded(ids)
            }
            Err(error) => Load::Failed(error),
        };
        self.children.insert(parent, loaded);
    }

    /// Forget what was read through an old session; keep what the user opened.
    pub fn reset(&mut self, session: SessionId) {
        self.session = Some(session);
        self.nodes.clear();
        self.children.clear();
        self.attributes = None;
        self.method = None;
        self.pending.clear();
    }
}

/// The OPC UA state of the window: browsers per client and the inspector's typed inputs.
pub struct OpcuaModel {
    pub browsers: HashMap<ConnectionId, Browser>,
    /// The focused pane's browser takes the keyboard through this handle.
    pub focus: FocusHandle,
    /// Writes to OPC UA points: any data type, so plain text.
    pub write_input: Entity<TextInput>,
    /// The picked method's arguments.
    pub arguments: Vec<Entity<TextInput>>,
    _subscriptions: Vec<Subscription>,
}

impl OpcuaModel {
    pub fn new(window: &mut Window, cx: &mut Context<AppModel>) -> Self {
        let write_input = cx.new(|cx| TextInput::new(cx).placeholder("New value").mono());
        let arguments: Vec<Entity<TextInput>> = (0..MAX_ARGUMENTS)
            .map(|_| cx.new(|cx| TextInput::new(cx).mono()))
            .collect();
        let mut subscriptions = vec![cx.subscribe_in(
            &write_input,
            window,
            |model, _, event: &InputEvent, window, cx| match event {
                InputEvent::Submitted => model.opcua_write_selected(window, cx),
                InputEvent::Cancelled => window.focus(&model.focus),
                InputEvent::Changed | InputEvent::Blurred => {}
            },
        )];
        for argument in &arguments {
            subscriptions.push(cx.subscribe_in(
                argument,
                window,
                |model, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submitted => model.call_selected_method(cx),
                    InputEvent::Cancelled => window.focus(&model.opcua.focus),
                    InputEvent::Changed | InputEvent::Blurred => {}
                },
            ));
        }
        Self {
            browsers: HashMap::new(),
            focus: cx.focus_handle(),
            write_input,
            arguments,
            _subscriptions: subscriptions,
        }
    }
}

/// Whether a saved connection is an OPC UA client.
fn is_client(model: &AppModel, connection: ConnectionId) -> bool {
    model
        .workspace
        .connection(connection)
        .is_some_and(|saved| matches!(saved.spec, SessionSpec::OpcuaClient(_)))
}

impl AppModel {
    // ── Engine answers ────────────────────────────────────────────────────

    /// A client session is up: read its address space (again), reopening what was open.
    pub(super) fn opcua_connected(&mut self, connection: ConnectionId) {
        if !is_client(self, connection) {
            return;
        }
        let Some(session) = self.live.get(&connection).and_then(|live| live.session) else {
            return;
        };
        let browser = self.opcua.browsers.entry(connection).or_default();
        browser.reset(session);
        let mut parents: Vec<String> = vec![ROOT.to_owned()];
        parents.extend(browser.expanded.iter().cloned());
        for parent in parents {
            browser.children.insert(parent.clone(), Load::Loading);
            self.engine.send(EngineCommand::Opcua {
                session,
                request: OpcuaRequest::Browse { node_id: parent },
            });
        }
    }

    /// An answer of the OPC UA client (or of the endpoint picker's GetEndpoints).
    pub(super) fn opcua_answer(&mut self, session: SessionId, answer: OpcuaEvent) {
        if let OpcuaEvent::Endpoints(found) = answer {
            if let Some(editor) = self.editor_mut()
                && matches!(editor.opcua.discovery, Discovery::Searching(waiting) if waiting == session)
            {
                editor.opcua.discovery = match found {
                    Ok(endpoints) => Discovery::Found(endpoints),
                    Err(error) => Discovery::Failed(error),
                };
            }
            return;
        }
        let Some(connection) = self.sessions.get(&session).copied() else {
            return;
        };
        let Some(browser) = self.opcua.browsers.get_mut(&connection) else {
            return;
        };
        if browser.session != Some(session) {
            return;
        }
        match answer {
            OpcuaEvent::Children { parent, result } => browser.take_children(parent, result),
            OpcuaEvent::Attributes { node_id, result } => {
                if browser
                    .attributes
                    .as_ref()
                    .is_some_and(|(waiting, _)| *waiting == node_id)
                {
                    browser.attributes = Some((
                        node_id,
                        match result {
                            Ok(attributes) => Load::Loaded(attributes),
                            Err(error) => Load::Failed(error),
                        },
                    ));
                }
            }
            OpcuaEvent::Monitored(outcomes) => {
                let accepted: Vec<OpcuaNode> = outcomes
                    .iter()
                    .filter(|(_, result)| result.is_ok())
                    .filter_map(|(id, _)| {
                        browser
                            .pending
                            .iter()
                            .find(|node| node.node_id == *id)
                            .cloned()
                    })
                    .collect();
                let refused: Vec<String> = outcomes
                    .iter()
                    .filter_map(|(id, result)| {
                        result.as_ref().err().map(|error| {
                            let name = browser
                                .nodes
                                .get(id)
                                .map_or(id.as_str(), |node| node.display_name.as_str());
                            format!("{name}: {error}")
                        })
                    })
                    .collect();
                browser
                    .pending
                    .retain(|node| !outcomes.iter().any(|(id, _)| *id == node.node_id));
                browser.notice = match (accepted.len(), refused.as_slice()) {
                    (0, []) => None,
                    (added, []) => Some((
                        false,
                        format!("Added {added} point{}", if added == 1 { "" } else { "s" }),
                    )),
                    (_, [one]) => Some((true, one.clone())),
                    (_, many) => Some((
                        true,
                        format!("{} not added: {}", many.len(), many.join("; ")),
                    )),
                };
                if !accepted.is_empty() {
                    self.remember_nodes(connection, accepted);
                }
            }
            OpcuaEvent::Method { method_id, result } => {
                if let Some(method) = browser
                    .method
                    .as_mut()
                    .filter(|method| method.method_id == method_id)
                {
                    method.info = match result {
                        Ok(info) => Load::Loaded(info),
                        Err(error) => Load::Failed(error),
                    };
                }
            }
            OpcuaEvent::Called { method_id, result } => {
                if let Some(method) = browser
                    .method
                    .as_mut()
                    .filter(|method| method.method_id == method_id)
                {
                    method.calling = false;
                    method.result = Some(result);
                }
            }
            OpcuaEvent::Endpoints(_) => {}
        }
    }

    /// Save variables the session now monitors in the client's spec, so they come back.
    fn remember_nodes(&mut self, connection: ConnectionId, nodes: Vec<OpcuaNode>) {
        let Some(saved) = self.workspace.connection(connection) else {
            return;
        };
        let SessionSpec::OpcuaClient(mut spec) = saved.spec.clone() else {
            return;
        };
        let name = saved.name.clone();
        for node in nodes {
            if !spec.nodes.iter().any(|known| known.node_id == node.node_id) {
                spec.nodes.push(node);
            }
        }
        self.workspace
            .update_connection(connection, name, SessionSpec::OpcuaClient(spec));
        self.save_quietly();
    }

    // ── Browser ───────────────────────────────────────────────────────────

    fn opcua_session(&self, connection: ConnectionId) -> Option<SessionId> {
        self.live
            .get(&connection)
            .filter(|live| live.state == vt_protocol::SessionState::Connected)
            .and_then(|live| live.session)
    }

    /// Show or hide a client's browser column.
    pub fn toggle_browser(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        let browser = self.opcua.browsers.entry(connection).or_default();
        browser.open = !browser.open;
        cx.notify();
    }

    /// Read the whole tree again.
    pub fn refresh_browser(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        if !self.licence_allows(cx) {
            return;
        }
        self.opcua_connected(connection);
        cx.notify();
    }

    /// Open or close a node, reading its children the first time.
    pub fn toggle_node(&mut self, connection: ConnectionId, node: &str, cx: &mut Context<Self>) {
        if !self.licence_allows(cx) {
            return;
        }
        let session = self.opcua_session(connection);
        let browser = self.opcua.browsers.entry(connection).or_default();
        if !browser.expanded.remove(node) {
            browser.expanded.insert(node.to_owned());
            if !browser.children.contains_key(node)
                && let Some(session) = session
            {
                browser.children.insert(node.to_owned(), Load::Loading);
                self.engine.send(EngineCommand::Opcua {
                    session,
                    request: OpcuaRequest::Browse {
                        node_id: node.to_owned(),
                    },
                });
            }
        }
        cx.notify();
    }

    /// Select a row: the inspector shows the node (its attributes; a method's arguments).
    pub fn select_node(
        &mut self,
        connection: ConnectionId,
        parent: String,
        node: String,
        cx: &mut Context<Self>,
    ) {
        if !self.licence_allows(cx) {
            return;
        }
        let session = self.opcua_session(connection);
        if let Some(live) = self.live.get_mut(&connection) {
            // The inspector follows the tree now, not a point.
            live.selected = None;
        }
        let browser = self.opcua.browsers.entry(connection).or_default();
        if browser.selected.as_ref() == Some(&(parent.clone(), node.clone())) {
            return;
        }
        let class = browser.nodes.get(&node).map(|found| found.class);
        browser.selected = Some((parent.clone(), node.clone()));
        browser.attributes = None;
        browser.method = None;
        if let Some(session) = session {
            browser.attributes = Some((node.clone(), Load::Loading));
            self.engine.send(EngineCommand::Opcua {
                session,
                request: OpcuaRequest::Attributes {
                    node_id: node.clone(),
                },
            });
            if class == Some(OpcuaNodeClass::Method) {
                browser.method = Some(MethodPanel {
                    object_id: parent,
                    method_id: node.clone(),
                    info: Load::Loading,
                    calling: false,
                    result: None,
                });
                self.engine.send(EngineCommand::Opcua {
                    session,
                    request: OpcuaRequest::Method { method_id: node },
                });
                for argument in &self.opcua.arguments {
                    argument.update(cx, |input, cx| input.set_text("", cx));
                }
            }
        }
        cx.notify();
    }

    /// The focused pane's client and its drawn tree, for the keyboard.
    fn focused_browser(&self) -> Option<(ConnectionId, Vec<Row>)> {
        let connection = self.focused_connection()?;
        if !is_client(self, connection) {
            return None;
        }
        let rows = self.opcua.browsers.get(&connection)?.rows();
        Some((connection, rows))
    }

    fn select_row(
        &mut self,
        connection: ConnectionId,
        rows: &[Row],
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = rows.get(index).filter(|row| row.kind == RowKind::Node) else {
            return;
        };
        self.select_node(connection, row.parent.clone(), row.node.clone(), cx);
        if let Some(browser) = self.opcua.browsers.get(&connection) {
            browser.scroll.scroll_to_item(index, ScrollStrategy::Center);
        }
    }

    /// ↑↓ through the tree's nodes.
    pub fn browse_step(&mut self, step: isize, cx: &mut Context<Self>) {
        let Some((connection, rows)) = self.focused_browser() else {
            return;
        };
        let nodes: Vec<usize> = (0..rows.len())
            .filter(|&index| rows[index].kind == RowKind::Node)
            .collect();
        if nodes.is_empty() {
            return;
        }
        let current = self
            .opcua
            .browsers
            .get(&connection)
            .and_then(|browser| browser.selected_row(&rows))
            .and_then(|row| nodes.iter().position(|&index| index == row));
        let next = match current {
            None => 0,
            Some(at) => {
                let at = isize::try_from(at).unwrap_or(0);
                let last = isize::try_from(nodes.len() - 1).unwrap_or(0);
                usize::try_from((at + step).clamp(0, last)).unwrap_or(0)
            }
        };
        self.select_row(connection, &rows, nodes[next], cx);
    }

    /// → opens the selected node, or moves into it when open.
    pub fn browse_expand(&mut self, cx: &mut Context<Self>) {
        let Some((connection, rows)) = self.focused_browser() else {
            return;
        };
        let Some(browser) = self.opcua.browsers.get(&connection) else {
            return;
        };
        let Some(index) = browser.selected_row(&rows) else {
            self.browse_step(1, cx);
            return;
        };
        let node = rows[index].node.clone();
        let has_children = browser
            .nodes
            .get(&node)
            .is_some_and(|found| found.has_children);
        if !has_children {
            return;
        }
        if browser.expanded.contains(&node) {
            if rows
                .get(index + 1)
                .is_some_and(|next| next.parent == node && next.kind == RowKind::Node)
            {
                self.select_row(connection, &rows, index + 1, cx);
            }
        } else {
            self.toggle_node(connection, &node, cx);
        }
    }

    /// ← closes the selected node, or moves to its parent.
    pub fn browse_collapse(&mut self, cx: &mut Context<Self>) {
        let Some((connection, rows)) = self.focused_browser() else {
            return;
        };
        let Some(browser) = self.opcua.browsers.get(&connection) else {
            return;
        };
        let Some(index) = browser.selected_row(&rows) else {
            return;
        };
        let row = &rows[index];
        if browser.expanded.contains(&row.node) {
            let node = row.node.clone();
            self.toggle_node(connection, &node, cx);
        } else if row.parent != ROOT
            && let Some(parent) = rows[..index]
                .iter()
                .rposition(|above| above.kind == RowKind::Node && above.node == row.parent)
        {
            self.select_row(connection, &rows, parent, cx);
        }
    }

    /// Enter adds the selected variable to the points; on other nodes it opens or closes them.
    pub fn browse_activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((connection, rows)) = self.focused_browser() else {
            return;
        };
        let Some(browser) = self.opcua.browsers.get(&connection) else {
            return;
        };
        let Some(index) = browser.selected_row(&rows) else {
            return;
        };
        let row = rows[index].clone();
        let Some(node) = browser.nodes.get(&row.node).cloned() else {
            return;
        };
        match node.class {
            OpcuaNodeClass::Variable => self.add_to_points(connection, &row.parent, &row.node, cx),
            OpcuaNodeClass::Method => {
                if let Some(first) = self.opcua.arguments.first() {
                    first.update(cx, |input, _| input.focus(window));
                }
            }
            _ if node.has_children => self.toggle_node(connection, &row.node, cx),
            _ => {}
        }
    }

    /// ⌘⇧B: put the keyboard in the focused pane's browser, showing it if hidden.
    pub fn focus_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection) = self.focused_connection() else {
            return;
        };
        if !is_client(self, connection) {
            return;
        }
        let browser = self.opcua.browsers.entry(connection).or_default();
        browser.open = true;
        window.focus(&self.opcua.focus);
        if browser.selected.is_none() {
            self.browse_step(1, cx);
        }
        cx.notify();
    }

    /// Whether a variable is one of the client's points already.
    pub fn is_monitored(&self, connection: ConnectionId, node: &str) -> bool {
        self.live
            .get(&connection)
            .is_some_and(|live| live.points.iter().any(|point| point.address == node))
    }

    /// Monitor a variable: it joins the points, and the connection keeps it.
    pub fn add_to_points(
        &mut self,
        connection: ConnectionId,
        parent: &str,
        node: &str,
        cx: &mut Context<Self>,
    ) {
        if !self.licence_allows(cx) {
            return;
        }
        if self.is_monitored(connection, node) {
            if let Some(browser) = self.opcua.browsers.get_mut(&connection) {
                browser.notice = Some((false, "Already a point".into()));
            }
            cx.notify();
            return;
        }
        let Some(session) = self.opcua_session(connection) else {
            return;
        };
        let browser = self.opcua.browsers.entry(connection).or_default();
        let display = |id: &str| {
            browser
                .nodes
                .get(id)
                .map(|found| found.display_name.clone())
                .unwrap_or_default()
        };
        // Named after the object it belongs to: `Reactor › Temperature`.
        let name = match (display(parent), display(node)) {
            (owner, own) if owner.is_empty() || parent == ROOT => own,
            (owner, own) => format!("{owner} › {own}"),
        };
        let wanted = OpcuaNode::new(node, name);
        if !browser.pending.iter().any(|known| known.node_id == node) {
            browser.pending.push(wanted.clone());
        }
        browser.notice = None;
        self.engine.send(EngineCommand::Opcua {
            session,
            request: OpcuaRequest::Monitor {
                nodes: vec![wanted],
            },
        });
        cx.notify();
    }

    /// Stop monitoring a point: the connection forgets it and the session restarts without it.
    pub fn remove_opcua_point(
        &mut self,
        connection: ConnectionId,
        point: u32,
        cx: &mut Context<Self>,
    ) {
        let Some(node_id) = self
            .live
            .get(&connection)
            .and_then(|live| live.point(point))
            .map(|(info, _)| info.address.clone())
        else {
            return;
        };
        let Some(saved) = self.workspace.connection(connection) else {
            return;
        };
        let SessionSpec::OpcuaClient(mut spec) = saved.spec.clone() else {
            return;
        };
        let name = saved.name.clone();
        spec.nodes.retain(|node| node.node_id != node_id);
        self.workspace
            .update_connection(connection, name, SessionSpec::OpcuaClient(spec));
        if let Some(live) = self.live.get_mut(&connection) {
            live.selected = None;
        }
        self.save(cx);
        if self.is_running(connection) {
            self.connect(connection, cx);
        }
        cx.notify();
    }

    /// Write the inspector's text to the selected OPC UA point.
    pub fn opcua_write_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.licence_allows(cx) {
            return;
        }
        let Some(connection) = self.focused_connection() else {
            return;
        };
        let text = self.opcua.write_input.read(cx).value().to_owned();
        let Some(live) = self.live.get_mut(&connection) else {
            return;
        };
        let (Some(session), Some(point)) = (live.session, live.selected) else {
            return;
        };
        if text.trim().is_empty() {
            live.write_result = Some((point, Err("Type a value to write".into())));
            cx.notify();
            return;
        }
        live.write_result = None;
        self.engine.send(EngineCommand::Opcua {
            session,
            request: OpcuaRequest::Write { point, text },
        });
        window.focus(&self.focus);
        cx.notify();
    }

    /// Call the method selected in the focused pane's browser with the typed arguments.
    pub fn call_selected_method(&mut self, cx: &mut Context<Self>) {
        if !self.licence_allows(cx) {
            return;
        }
        let Some(connection) = self.focused_connection() else {
            return;
        };
        let Some(session) = self.opcua_session(connection) else {
            return;
        };
        let texts: Vec<String> = self
            .opcua
            .arguments
            .iter()
            .map(|input| input.read(cx).value().to_owned())
            .collect();
        let Some(method) = self
            .opcua
            .browsers
            .get_mut(&connection)
            .and_then(|browser| browser.method.as_mut())
        else {
            return;
        };
        let Load::Loaded(info) = &method.info else {
            return;
        };
        if method.calling || info.inputs.len() > MAX_ARGUMENTS {
            return;
        }
        let arguments = texts.into_iter().take(info.inputs.len()).collect();
        method.calling = true;
        method.result = None;
        self.engine.send(EngineCommand::Opcua {
            session,
            request: OpcuaRequest::Call {
                object_id: method.object_id.clone(),
                method_id: method.method_id.clone(),
                arguments,
            },
        });
        cx.notify();
    }

    // ── Editor ────────────────────────────────────────────────────────────

    /// The endpoint picker: GetEndpoints from the typed endpoint.
    pub fn discover_opcua(&mut self, cx: &mut Context<Self>) {
        if !self.licence_allows(cx) {
            return;
        }
        self.next_session += 1;
        let request = SessionId(self.next_session);
        let Some(editor) = self.editor_mut() else {
            return;
        };
        let url = OpcuaFields::text(&editor.opcua.endpoint, cx)
            .trim()
            .to_owned();
        editor.opcua.discovery = Discovery::Searching(request);
        editor.error = None;
        self.engine.send(EngineCommand::Opcua {
            session: request,
            request: OpcuaRequest::Endpoints { url },
        });
        cx.notify();
    }

    /// Take an endpoint's security into the client fields.
    pub fn pick_opcua_endpoint(&mut self, endpoint: &OpcuaEndpoint, cx: &mut Context<Self>) {
        let Some(editor) = self.editor_mut() else {
            return;
        };
        let Some(policy) = endpoint.policy else {
            return;
        };
        editor.opcua.policy = policy;
        editor.opcua.mode = endpoint.mode;
        // Sign in the way the endpoint takes, if it does not take the chosen way.
        let offers = |token: &str| endpoint.user_tokens.iter().any(|offered| offered == token);
        let chosen = match editor.opcua.auth {
            OpcuaAuthKind::Anonymous => "Anonymous",
            OpcuaAuthKind::UserName => "UserName",
            OpcuaAuthKind::Certificate => "Certificate",
        };
        if !offers(chosen)
            && let Some(kind) = [
                ("Anonymous", OpcuaAuthKind::Anonymous),
                ("UserName", OpcuaAuthKind::UserName),
                ("Certificate", OpcuaAuthKind::Certificate),
            ]
            .into_iter()
            .find_map(|(token, kind)| offers(token).then_some(kind))
        {
            editor.opcua.auth = kind;
        }
        editor.error = None;
        cx.notify();
    }

    /// Choose a certificate or key file with the system's open panel (which grants the sandboxed
    /// app access to it).
    pub fn choose_opcua_file(&mut self, which: CertificateFile, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(
                match which {
                    CertificateFile::Certificate => "Use certificate",
                    CertificateFile::PrivateKey => "Use private key",
                }
                .into(),
            ),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |model, cx| model.set_opcua_file(which, &path, cx));
        })
        .detach();
    }

    fn set_opcua_file(&mut self, which: CertificateFile, path: &Path, cx: &mut Context<Self>) {
        let Some(editor) = self.editor_mut() else {
            return;
        };
        let field = match which {
            CertificateFile::Certificate => editor.opcua.certificate.clone(),
            CertificateFile::PrivateKey => editor.opcua.private_key.clone(),
        };
        editor.error = None;
        let text = path.display().to_string();
        field.update(cx, |input, cx| input.set_text(text, cx));
        cx.notify();
    }

    // ── Demo ──────────────────────────────────────────────────────────────

    /// A TCP port from the OPC UA port up that no saved simulator uses and nothing holds.
    fn free_opcua_port(&self) -> Option<u16> {
        let taken: Vec<u16> = self
            .workspace
            .connections
            .iter()
            .filter_map(|connection| match &connection.spec {
                SessionSpec::OpcuaServer(server) => Some(server.port),
                SessionSpec::ModbusSlave(slave) => Some(slave.port),
                SessionSpec::MqttBroker(broker) => Some(broker.port),
                SessionSpec::Dnp3Outstation(outstation) => Some(outstation.port),
                _ => None,
            })
            .collect();
        (OPCUA_PORT..OPCUA_PORT + 100)
            .find(|port| !taken.contains(port) && TcpListener::bind(("0.0.0.0", *port)).is_ok())
    }

    /// After a new OPC UA simulator is saved: a client of it, side by side.
    pub(super) fn add_opcua_client(
        &mut self,
        name: &str,
        server: &OpcuaServerSpec,
        cx: &mut Context<Self>,
    ) {
        let client = self.workspace.add_connection(
            format!("{name} client"),
            SessionSpec::OpcuaClient(server.client(demo_nodes())),
        );
        self.workspace
            .split(client, Axis::Horizontal, PaneView::Points);
        self.connect(client, cx);
    }

    /// A simulated process plant and a client browsing and monitoring it, side by side.
    pub fn start_opcua_demo(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(ConnectionId, ConnectionId)> {
        if !self.licence_allows(cx) {
            return None;
        }
        let Some(port) = self.free_opcua_port() else {
            self.show_toast(
                "No free TCP port between 4840 and 4939 for the server",
                true,
                cx,
            );
            return None;
        };
        let server = OpcuaServerSpec {
            port,
            ..OpcuaServerSpec::default()
        };
        let client = server.client(demo_nodes());
        let simulator = self
            .workspace
            .add_connection("Plant OPC UA server", SessionSpec::OpcuaServer(server));
        let client = self
            .workspace
            .add_connection("Process plant", SessionSpec::OpcuaClient(client));
        self.show_side_by_side(client, simulator, cx);
        // The plant's machines are open in the browser from the start.
        let browser = self.opcua.browsers.entry(client).or_default();
        let ns = PLANT_NAMESPACE_INDEX;
        browser.expanded.insert(format!("ns={ns};s=Plant"));
        browser.expanded.insert(format!("ns={ns};s=Plant.Reactor"));
        Some((client, simulator))
    }
}

/// The variables a client of the plant simulator starts with.
pub fn demo_nodes() -> Vec<OpcuaNode> {
    DEMO_NODES
        .iter()
        .map(|(key, name)| OpcuaNode::new(format!("ns={PLANT_NAMESPACE_INDEX};s={key}"), *name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, class: OpcuaNodeClass, has_children: bool) -> OpcuaBrowseNode {
        OpcuaBrowseNode {
            node_id: id.into(),
            browse_name: id.into(),
            display_name: id.to_uppercase(),
            class,
            data_type: String::new(),
            has_children,
        }
    }

    #[test]
    fn the_tree_opens_level_by_level() {
        let mut browser = Browser::default();
        assert!(browser.rows().is_empty(), "nothing read yet");
        browser.children.insert(ROOT.into(), Load::Loading);
        assert_eq!(browser.rows()[0].kind, RowKind::Loading);
        browser.take_children(
            ROOT.into(),
            Ok(vec![
                node("plant", OpcuaNodeClass::Object, true),
                node("server", OpcuaNodeClass::Object, true),
                // Two references to one node: listed once.
                node("plant", OpcuaNodeClass::Object, true),
            ]),
        );
        let rows = browser.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(browser.nodes["plant"].display_name, "PLANT");
        browser.expanded.insert("plant".into());
        browser.children.insert("plant".into(), Load::Loading);
        let rows = browser.rows();
        assert_eq!(
            rows.iter()
                .map(|row| (row.depth, row.kind.clone()))
                .collect::<Vec<_>>(),
            [
                (0, RowKind::Node),
                (1, RowKind::Loading),
                (0, RowKind::Node)
            ]
        );
        browser.take_children(
            "plant".into(),
            Ok(vec![node("reactor", OpcuaNodeClass::Object, false)]),
        );
        browser.take_children("server".into(), Err("BadUserAccessDenied".into()));
        browser.expanded.insert("server".into());
        let rows = browser.rows();
        assert_eq!(rows[1].node, "reactor");
        assert_eq!(rows[1].parent, "plant");
        assert_eq!(rows[3].kind, RowKind::Failed("BadUserAccessDenied".into()));
        browser.selected = Some(("plant".into(), "reactor".into()));
        assert_eq!(browser.selected_row(&rows), Some(1));
    }

    #[test]
    fn reference_cycles_stop_the_walk() {
        let mut browser = Browser::default();
        browser.take_children(
            ROOT.into(),
            Ok(vec![node("a", OpcuaNodeClass::Object, true)]),
        );
        browser.take_children(
            "a".into(),
            Ok(vec![node("b", OpcuaNodeClass::Object, true)]),
        );
        // b organizes a again.
        browser.take_children(
            "b".into(),
            Ok(vec![node("a", OpcuaNodeClass::Object, true)]),
        );
        browser.expanded.extend(["a".to_owned(), "b".to_owned()]);
        let rows = browser.rows();
        assert_eq!(
            rows.iter().map(|row| row.node.as_str()).collect::<Vec<_>>(),
            ["a", "b", "a"],
            "a under b is listed but not opened again"
        );
    }

    #[test]
    fn a_new_session_reads_the_tree_again_but_keeps_what_was_open() {
        let mut browser = Browser::default();
        browser.take_children(
            ROOT.into(),
            Ok(vec![node("a", OpcuaNodeClass::Object, true)]),
        );
        browser.expanded.insert("a".into());
        browser.pending.push(OpcuaNode::new("x", ""));
        browser.reset(SessionId(9));
        assert!(browser.children.is_empty() && browser.pending.is_empty());
        assert!(browser.expanded.contains("a"));
        assert_eq!(browser.session, Some(SessionId(9)));
    }

    #[test]
    fn demo_nodes_live_in_the_plant_namespace() {
        let nodes = demo_nodes();
        assert_eq!(nodes[0].node_id, "ns=2;s=Plant.Reactor.Temperature");
        assert_eq!(nodes[0].name, "Reactor › Temperature");
    }
}
