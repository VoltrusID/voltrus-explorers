//! OPC UA views: the address-space browser beside a client's points, the inspector for a node
//! picked in it (attributes, adding a variable, calling a method), and the typed write of an
//! OPC UA point.

use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, ElementId, FontWeight, MouseButton, SharedString, UniformListScrollHandle,
    div, px, uniform_list,
};
use vt_app_core::{ConnectionId, PaneId};
use vt_protocol::{OpcuaNodeClass, PointInfo, Role, SessionSpec, SessionState};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, divider, field, kbd, pill, section_label};
use vt_ui::theme::*;

use super::points;
use crate::model::AppModel;
use crate::model::live::Live;
use crate::model::opcua::{
    BROWSER, BrowseActivate, BrowseCollapse, BrowseDown, BrowseExpand, BrowseUp, Load,
    MAX_ARGUMENTS, Row, RowKind, TreeNode,
};

/// Height of a tree row.
const TREE_ROW: f32 = 24.0;
/// Indent per tree level.
const INDENT: f32 = 14.0;
/// The browser's width, and in narrow panes.
const BROWSER_WIDE: f32 = 290.0;
const BROWSER_NARROW: f32 = 230.0;
/// Below this pane width the browser takes its narrow width.
const NARROW_PANE: f32 = 900.0;
/// The points table needs this much beside an open browser; narrower panes fold it to a rail.
const TABLE_MIN: f32 = 380.0;
/// Width of the rail a hidden browser leaves.
const RAIL: f32 = 32.0;

fn class_icon(node: &TreeNode) -> Icon {
    match node.class {
        OpcuaNodeClass::Object if node.has_children => Icon::Folder,
        OpcuaNodeClass::Object => Icon::Boxes,
        OpcuaNodeClass::Variable => Icon::Gauge,
        OpcuaNodeClass::Method => Icon::Terminal,
        OpcuaNodeClass::View => Icon::Eye,
        OpcuaNodeClass::ObjectType
        | OpcuaNodeClass::VariableType
        | OpcuaNodeClass::ReferenceType
        | OpcuaNodeClass::DataType => Icon::Layers,
        OpcuaNodeClass::Unspecified => Icon::Circle,
    }
}

/// An OPC UA client's points with its address-space browser on the left.
pub fn points(
    model: &AppModel,
    pane: PaneId,
    connection: ConnectionId,
    live: &Live,
    pane_width: f32,
    scroll: UniformListScrollHandle,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let open = model
        .opcua
        .browsers
        .get(&connection)
        .is_none_or(|browser| browser.open)
        && pane_width - BROWSER_NARROW >= TABLE_MIN;
    let width = if !open {
        RAIL
    } else if pane_width < NARROW_PANE {
        BROWSER_NARROW
    } else {
        BROWSER_WIDE
    };
    div()
        .flex()
        .size_full()
        .child(if open {
            browser(model, pane, connection, width, cx)
        } else {
            rail(connection, cx)
        })
        .child(div().flex_1().min_w_0().h_full().child(points::render(
            pane,
            connection,
            live,
            false,
            pane_width - width,
            scroll,
            cx,
        )))
        .into_any_element()
}

/// What a hidden browser leaves: a button to show it again.
fn rail(connection: ConnectionId, cx: &mut Context<AppModel>) -> AnyElement {
    let theme = theme(cx).clone();
    div()
        .flex()
        .flex_col()
        .flex_none()
        .items_center()
        .w(px(RAIL))
        .h_full()
        .pt(SPACE_3)
        .border_r_1()
        .border_color(theme.border)
        .bg(theme.chrome)
        .child(
            IconButton::new(
                ElementId::NamedInteger("opcua-show-browser".into(), connection.0),
                Icon::PanelLeft,
            )
            .tooltip("Show the address space  ⌘⇧B")
            .on_click(cx.listener(move |model, _, _, cx| model.toggle_browser(connection, cx))),
        )
        .into_any_element()
}

fn browser(
    model: &AppModel,
    pane: PaneId,
    connection: ConnectionId,
    width: f32,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let state = model.opcua.browsers.get(&connection);
    let rows: Rc<Vec<Row>> = Rc::new(state.map(|browser| browser.rows()).unwrap_or_default());
    let connected = model
        .live
        .get(&connection)
        .is_some_and(|live| live.state == SessionState::Connected);
    let focused = model.focused_pane().map(|focused| focused.id) == Some(pane);
    let notice = state.and_then(|browser| browser.notice.clone());
    let scroll = state.map(|browser| browser.scroll.clone());

    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_2)
        .h(px(30.0))
        .pl(SPACE_6)
        .pr(SPACE_3)
        .border_b_1()
        .border_color(theme.border)
        .child(section_label("Address space", cx))
        .child(div().flex_1())
        .child(
            IconButton::new(
                ElementId::NamedInteger("opcua-refresh".into(), pane.0),
                Icon::RefreshCw,
            )
            .disabled(!connected)
            .tooltip("Read the address space again")
            .on_click(cx.listener(move |model, _, _, cx| model.refresh_browser(connection, cx))),
        )
        .child(
            IconButton::new(
                ElementId::NamedInteger("opcua-hide-browser".into(), pane.0),
                Icon::PanelLeft,
            )
            .tooltip("Hide the address space")
            .on_click(cx.listener(move |model, _, _, cx| model.toggle_browser(connection, cx))),
        );

    let body: AnyElement = if rows.is_empty() {
        div()
            .flex_1()
            .p(SPACE_6)
            .text_size(FONT_SMALL)
            .text_color(theme.fg_dim)
            .child(if connected {
                "Reading the address space…"
            } else {
                "Connect to browse the server's address space."
            })
            .into_any_element()
    } else {
        let count = rows.len();
        let list_rows = rows.clone();
        let list = uniform_list(
            ElementId::NamedInteger("opcua-tree".into(), pane.0),
            count,
            cx.processor(move |model: &mut AppModel, range: Range<usize>, _, cx| {
                let theme = vt_ui::theme::theme(cx).clone();
                let Some(browser) = model.opcua.browsers.get(&connection) else {
                    return Vec::new();
                };
                let selected = browser.selected.clone();
                range
                    .filter_map(|index| {
                        let row = list_rows.get(index)?;
                        Some(tree_row(
                            model,
                            connection,
                            index,
                            row,
                            selected.as_ref(),
                            &theme,
                            cx,
                        ))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .flex_1();
        match scroll {
            Some(scroll) => list.track_scroll(scroll).into_any_element(),
            None => list.into_any_element(),
        }
    };

    let footer = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_3)
        .min_h(px(30.0))
        .px(SPACE_6)
        .py(SPACE_2)
        .border_t_1()
        .border_color(theme.border)
        .text_size(FONT_CAPTION)
        .text_color(theme.fg_dim)
        .map(|this| match notice {
            Some((error, text)) => this
                .child(vt_ui::icon(
                    if error {
                        Icon::CircleAlert
                    } else {
                        Icon::CircleCheck
                    },
                    ICON_SM,
                    if error { theme.danger } else { theme.success },
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(if error { theme.danger } else { theme.fg_muted })
                        .child(text),
                ),
            None => this
                .child(kbd("↑↓", cx))
                .child("move")
                .child(kbd("→", cx))
                .child("open")
                .child(kbd("⏎", cx))
                .child("add"),
        });

    let accent = theme.accent;
    div()
        .id(ElementId::NamedInteger("opcua-browser".into(), pane.0))
        .flex()
        .flex_col()
        .flex_none()
        .w(px(width))
        .h_full()
        .bg(theme.chrome)
        .border_r_1()
        .border_color(theme.border)
        .key_context(BROWSER)
        .when(focused, |this| {
            this.track_focus(&model.opcua.focus)
                .focus(move |style| style.border_color(accent))
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|model, _, window, _| window.focus(&model.opcua.focus)),
        )
        .on_action(cx.listener(|model, _: &BrowseUp, _, cx| model.browse_step(-1, cx)))
        .on_action(cx.listener(|model, _: &BrowseDown, _, cx| model.browse_step(1, cx)))
        .on_action(cx.listener(|model, _: &BrowseExpand, _, cx| model.browse_expand(cx)))
        .on_action(cx.listener(|model, _: &BrowseCollapse, _, cx| model.browse_collapse(cx)))
        .on_action(cx.listener(|model, _: &BrowseActivate, window, cx| {
            model.browse_activate(window, cx);
        }))
        .child(header)
        .child(div().flex().flex_col().flex_1().min_h_0().child(body))
        .child(footer)
        .into_any_element()
}

fn tree_row(
    model: &AppModel,
    connection: ConnectionId,
    index: usize,
    row: &Row,
    selected: Option<&(String, String)>,
    theme: &Theme,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let indent = px(f32::from(SPACE_4) + INDENT * row.depth as f32);
    let id = ElementId::NamedInteger("opcua-row".into(), index as u64);
    let placeholder = |icon: Icon, color: gpui::Hsla, text: SharedString| {
        div()
            .id(id.clone())
            .flex()
            .items_center()
            .gap(SPACE_3)
            .h(px(TREE_ROW))
            .pl(indent + px(INDENT))
            .pr(SPACE_4)
            .text_size(FONT_SMALL)
            .text_color(color)
            .child(vt_ui::icon(icon, ICON_SM, color))
            .child(div().flex_1().min_w_0().truncate().child(text))
            .into_any_element()
    };
    match &row.kind {
        RowKind::Loading => return placeholder(Icon::RefreshCw, theme.fg_dim, "Reading…".into()),
        RowKind::Empty => return placeholder(Icon::Circle, theme.fg_dim, "No children".into()),
        RowKind::Failed(error) => {
            return placeholder(Icon::CircleAlert, theme.danger, error.clone().into());
        }
        RowKind::Node => {}
    }
    let Some(browser) = model.opcua.browsers.get(&connection) else {
        return div().into_any_element();
    };
    let Some(node) = browser.nodes.get(&row.node) else {
        return div().into_any_element();
    };
    let is_selected = selected.is_some_and(|(parent, id)| *parent == row.parent && *id == row.node);
    let expanded = browser.expanded.contains(&row.node);
    let variable = node.class == OpcuaNodeClass::Variable;
    let monitored = variable && model.is_monitored(connection, &row.node);
    let (parent, node_id) = (row.parent.clone(), row.node.clone());
    let toggle_id = row.node.clone();
    let (add_parent, add_node) = (row.parent.clone(), row.node.clone());
    let hover = theme.hover;
    let class_color = match node.class {
        OpcuaNodeClass::Variable => theme.info,
        OpcuaNodeClass::Method => theme.warning,
        _ => theme.fg_muted,
    };

    div()
        .id(id)
        .flex()
        .items_center()
        .gap(SPACE_3)
        .h(px(TREE_ROW))
        .pl(indent)
        .pr(SPACE_4)
        .cursor_pointer()
        .when(is_selected, |this| this.bg(theme.selected))
        .when(!is_selected, |this| {
            this.hover(move |style| style.bg(hover))
        })
        .on_click(
            cx.listener(move |model, event: &gpui::ClickEvent, window, cx| {
                window.focus(&model.opcua.focus);
                model.select_node(connection, parent.clone(), node_id.clone(), cx);
                // A double click adds a variable, or opens a node.
                if event.click_count() == 2 {
                    model.browse_activate(window, cx);
                }
            }),
        )
        .child(
            div()
                .id(ElementId::NamedInteger(
                    "opcua-chevron".into(),
                    index as u64,
                ))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(INDENT))
                .when(node.has_children, |this| {
                    this.child(vt_ui::icon(
                        if expanded {
                            Icon::ChevronDown
                        } else {
                            Icon::ChevronRight
                        },
                        ICON_SM,
                        theme.fg_dim,
                    ))
                    .on_click(cx.listener(move |model, _, _, cx| {
                        cx.stop_propagation();
                        model.toggle_node(connection, &toggle_id, cx);
                    }))
                }),
        )
        .child(vt_ui::icon(class_icon(node), ICON_SM, class_color))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(FONT_SMALL)
                .when(is_selected, |this| this.font_weight(FontWeight::MEDIUM))
                .child(node.display_name.clone()),
        )
        .when(variable && !node.data_type.is_empty(), |this| {
            this.child(
                div()
                    .flex_none()
                    .max_w(px(72.0))
                    .truncate()
                    .font_family(MONO)
                    .text_size(FONT_CAPTION)
                    .text_color(theme.fg_dim)
                    .child(node.data_type.clone()),
            )
        })
        .when(monitored, |this| {
            this.child(vt_ui::icon(Icon::Check, ICON_SM, theme.success))
        })
        .when(variable && !monitored && is_selected, |this| {
            this.child(
                IconButton::new(
                    ElementId::NamedInteger("opcua-add".into(), index as u64),
                    Icon::Plus,
                )
                .size(px(18.0))
                .tooltip("Add to points  ⏎")
                .on_click(cx.listener(move |model, _, _, cx| {
                    model.add_to_points(connection, &add_parent, &add_node, cx);
                })),
            )
        })
        .into_any_element()
}

/// Whether the inspector shows a node picked in the browser: an OPC UA client with a tree
/// selection and no point selected.
pub fn inspects_node(model: &AppModel, connection: ConnectionId) -> bool {
    let client = model
        .workspace
        .connection(connection)
        .is_some_and(|saved| matches!(saved.spec, SessionSpec::OpcuaClient(_)));
    client
        && model
            .live
            .get(&connection)
            .is_some_and(|live| live.selected.is_none())
        && model
            .opcua
            .browsers
            .get(&connection)
            .is_some_and(|browser| browser.selected.is_some())
}

fn result_line(result: Result<String, String>, theme: &Theme) -> impl IntoElement {
    let (glyph, color, text) = match result {
        Ok(text) => (Icon::CircleCheck, theme.success, text),
        Err(error) => (Icon::CircleAlert, theme.danger, error),
    };
    div()
        .flex()
        .items_start()
        .gap(SPACE_3)
        .text_size(FONT_SMALL)
        .text_color(color)
        .child(vt_ui::icon(glyph, ICON_MD, color))
        .child(div().flex_1().child(text))
}

/// The inspector for the node selected in the browser.
pub fn node_detail(
    model: &AppModel,
    connection: ConnectionId,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(browser) = model.opcua.browsers.get(&connection) else {
        return div().into_any_element();
    };
    let Some((parent, node_id)) = browser.selected.clone() else {
        return div().into_any_element();
    };
    let Some(node) = browser.nodes.get(&node_id) else {
        return div().into_any_element();
    };
    let connected = model
        .live
        .get(&connection)
        .is_some_and(|live| live.state == SessionState::Connected);
    let monitored = model.is_monitored(connection, &node_id);

    let mut content = div().flex().flex_col().gap(SPACE_7).p(SPACE_7).child(
        div()
            .flex()
            .flex_col()
            .gap(SPACE_2)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_4)
                    .child(vt_ui::icon(class_icon(node), ICON_LG, theme.fg_muted))
                    .child(
                        div()
                            .text_size(FONT_TITLE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(node.display_name.clone()),
                    ),
            )
            .child(
                div()
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_dim)
                    .child(node_id.clone()),
            )
            .child(
                div()
                    .flex()
                    .gap(SPACE_3)
                    .child(pill(node.class.name().to_uppercase(), theme.info))
                    .when(!node.data_type.is_empty(), |this| {
                        this.child(pill(node.data_type.to_uppercase(), theme.fg_muted))
                    }),
            ),
    );

    match node.class {
        OpcuaNodeClass::Variable => {
            let (add_parent, add_node) = (parent.clone(), node_id.clone());
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_4)
                    .child(
                        div().flex().child(
                            Button::new(
                                "opcua-add-point",
                                if monitored { "On the points" } else { "Add to points" },
                            )
                            .icon(if monitored { Icon::Check } else { Icon::Plus })
                            .primary()
                            .disabled(monitored || !connected)
                            .on_click(cx.listener(move |model, _, _, cx| {
                                model.add_to_points(connection, &add_parent, &add_node, cx);
                            })),
                        ),
                    )
                    .child(
                        div()
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_dim)
                            .child(
                                "The server samples it and sends each change; it is saved with the connection.",
                            ),
                    ),
            );
        }
        OpcuaNodeClass::Method => {
            content = content.child(method_panel(model, connection, &theme, cx));
        }
        _ => {}
    }

    // Attributes and properties, as the server reports them.
    let attributes: AnyElement = match &browser.attributes {
        Some((id, Load::Loaded(attributes))) if *id == node_id => div()
            .flex()
            .flex_col()
            .rounded(RADIUS_LG)
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface)
            // Attributes a node class does not have (a method's Value) are left out.
            .children(attributes.iter().filter(|attribute| !attribute.status.contains("BadAttributeIdInvalid")).enumerate().map(|(index, attribute)| {
                let good = attribute.status == "Good";
                div()
                    .flex()
                    .items_start()
                    .gap(SPACE_5)
                    .px(SPACE_6)
                    .py(SPACE_3)
                    .when(index > 0, |this| {
                        this.border_t_1().border_color(theme.border)
                    })
                    .text_size(FONT_SMALL)
                    .child(
                        div()
                            .w(px(116.0))
                            .flex_none()
                            .truncate()
                            .text_color(theme.fg_muted)
                            .child(attribute.name.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(MONO)
                            .text_color(if good { theme.fg } else { theme.fg_dim })
                            .child(if good {
                                attribute.value.clone()
                            } else {
                                attribute.status.clone()
                            }),
                    )
            }))
            .into_any_element(),
        Some((id, Load::Failed(error))) if *id == node_id => {
            result_line(Err(error.clone()), &theme).into_any_element()
        }
        _ => div()
            .text_size(FONT_SMALL)
            .text_color(theme.fg_dim)
            .child(if connected {
                "Reading attributes…"
            } else {
                "Connect to read its attributes."
            })
            .into_any_element(),
    };
    content = content
        .child(divider(cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .child(section_label("Attributes", cx))
                .child(attributes),
        )
        .child(
            div()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child("↑ ↓ → ← walk the address space; Enter adds a variable."),
        );
    content.into_any_element()
}

/// A method's arguments, the Call button and its outcome.
fn method_panel(
    model: &AppModel,
    connection: ConnectionId,
    theme: &Theme,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let Some(method) = model
        .opcua
        .browsers
        .get(&connection)
        .and_then(|browser| browser.method.as_ref())
    else {
        return div().into_any_element();
    };
    let connected = model
        .live
        .get(&connection)
        .is_some_and(|live| live.state == SessionState::Connected);
    let info = match &method.info {
        Load::Loading => {
            return div()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child("Reading its arguments…")
                .into_any_element();
        }
        Load::Failed(error) => return result_line(Err(error.clone()), theme).into_any_element(),
        Load::Loaded(info) => info,
    };
    let too_many = info.inputs.len() > MAX_ARGUMENTS;
    let fields = info
        .inputs
        .iter()
        .zip(&model.opcua.arguments)
        .map(|(argument, input)| {
            div()
                .flex()
                .flex_col()
                .gap(SPACE_2)
                .child(field(
                    format!("{} · {}", argument.name, argument.data_type),
                    input.clone(),
                    cx,
                ))
                .when(!argument.description.is_empty(), |this| {
                    this.child(
                        div()
                            .text_size(FONT_CAPTION)
                            .text_color(theme.fg_dim)
                            .child(argument.description.clone()),
                    )
                })
        })
        .collect::<Vec<_>>();
    let outputs = info.outputs.clone();
    let result = method.result.clone().map(|result| {
        result.map(|values| {
            if values.is_empty() {
                "Called".to_owned()
            } else {
                values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let name = outputs.get(index).map_or_else(
                            || format!("#{}", index + 1),
                            |output| output.name.clone(),
                        );
                        format!("{name} = {value}")
                    })
                    .collect::<Vec<_>>()
                    .join(" · ")
            }
        })
    });
    div()
        .flex()
        .flex_col()
        .gap(SPACE_5)
        .child(section_label(
            if info.inputs.is_empty() {
                "Call (no arguments)".to_owned()
            } else {
                format!(
                    "Call with {} argument{}",
                    info.inputs.len(),
                    if info.inputs.len() == 1 { "" } else { "s" }
                )
            },
            cx,
        ))
        .children(fields)
        .child(
            div().flex().child(
                Button::new(
                    "opcua-call",
                    if method.calling { "Calling…" } else { "Call" },
                )
                .icon(Icon::Play)
                .primary()
                .disabled(method.calling || !connected || too_many)
                .on_click(cx.listener(|model, _, _, cx| model.call_selected_method(cx))),
            ),
        )
        .when(too_many, |this| {
            this.child(result_line(
                Err(format!(
                    "More than {MAX_ARGUMENTS} arguments: call it from another client"
                )),
                theme,
            ))
        })
        .when_some(result, |this, result| {
            this.child(result_line(result, theme))
        })
        .into_any_element()
}

/// The OPC UA part of a point's inspector: its node, the typed write and, for clients,
/// removing it.
pub fn point_section(
    model: &AppModel,
    connection: ConnectionId,
    live: &Live,
    point: u32,
    info: &PointInfo,
    cx: &mut Context<AppModel>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let spec = model
        .workspace
        .connection(connection)
        .map(|saved| &saved.spec);
    let simulator = spec.is_some_and(|spec| spec.role() == Role::Simulator);
    let client = matches!(spec, Some(SessionSpec::OpcuaClient(_)));
    let running = live.session.is_some();
    let result = live
        .write_result
        .as_ref()
        .filter(|(written, _)| *written == point)
        .map(|(_, result)| result.clone());
    let example = match info.data_type.as_str() {
        "Boolean" => "true or false",
        "String" | "LocalizedText" => "text",
        "DateTime" => "a time like 2026-10-26T08:30:00Z",
        "Float" | "Double" => "a number",
        _ => "a whole number",
    };
    div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_3)
                .child(section_label("Node", cx))
                .child(
                    div()
                        .px(SPACE_5)
                        .py(SPACE_3)
                        .rounded(RADIUS_MD)
                        .bg(theme.surface)
                        .border_1()
                        .border_color(theme.border)
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .child(info.address.clone()),
                ),
        )
        .when(info.writable, |this| {
            this.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_4)
                    .child(section_label(
                        if simulator {
                            "Set value"
                        } else {
                            "Write to server"
                        },
                        cx,
                    ))
                    .child(
                        div()
                            .flex()
                            .gap(SPACE_3)
                            .child(div().flex_1().child(model.opcua.write_input.clone()))
                            .child(
                                Button::new("opcua-write", if simulator { "Set" } else { "Write" })
                                    .icon(Icon::Send)
                                    .primary()
                                    .disabled(!running)
                                    .on_click(cx.listener(|model, _, window, cx| {
                                        model.opcua_write_selected(window, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_dim)
                            .child(if running {
                                format!("A {}: {example}.", info.data_type)
                            } else {
                                "Connect to write.".to_owned()
                            }),
                    )
                    .when_some(result, |this, result| {
                        this.child(result_line(
                            result.map(|()| {
                                if simulator {
                                    "Value set and held".to_owned()
                                } else {
                                    "The server took it".to_owned()
                                }
                            }),
                            &theme,
                        ))
                    }),
            )
        })
        .when(client, |this| {
            this.child(
                div().flex().child(
                    Button::new("opcua-remove-point", "Remove from points")
                        .icon(Icon::Trash)
                        .compact()
                        .tooltip("Stop monitoring it; the session restarts without it")
                        .on_click(cx.listener(move |model, _, _, cx| {
                            model.remove_opcua_point(connection, point, cx);
                        })),
                ),
            )
        })
        .into_any_element()
}
