//! Every app command, its shortcut and its menu entry.

use gpui::{KeyBinding, Menu, MenuItem, OsAction, actions};

actions!(
    voltrus_tools,
    [
        Quit,
        ToggleCommandPalette,
        NewConnection,
        NewSimulator,
        ToggleSidebar,
        ToggleInspector,
        SplitRight,
        SplitDown,
        ClosePane,
        NextTab,
        PreviousTab,
        ToggleSession,
        ShowPoints,
        ShowTrend,
        ShowTraffic,
        ShowLog,
        SelectNextPoint,
        SelectPreviousPoint,
        ChooseTheme,
        Dismiss,
        PaletteUp,
        PaletteDown,
        PaletteConfirm,
        FocusSend,
        SendHistoryPrevious,
        SendHistoryNext,
        SendMacro1,
        SendMacro2,
        SendMacro3,
        SendMacro4,
        SendMacro5,
        SendMacro6,
        SendMacro7,
        SendMacro8,
        SendMacro9,
    ]
);

pub const WORKSPACE: &str = "Workspace";
pub const PALETTE: &str = "Palette";
pub const EDITOR: &str = "Editor";
/// A serial pane's send bar: ↑↓ walk its history, ⌃1…⌃9 send macros.
pub const SERIAL_SEND: &str = "SerialSend";
/// Pane shortcuts: the workspace, but not behind the editor sheet or palette.
const PANE_KEYS: &str = "Workspace && !Editor && !Palette";
/// Where ↑↓ move through points: as above, and not while a text field has
/// focus, so they never retarget the inspector's write.
const POINT_KEYS: &str = "Workspace && !Editor && !Palette && !TextInput";

pub fn key_bindings() -> Vec<KeyBinding> {
    let primary = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let key = |keys: &str| format!("{primary}-{keys}");
    vec![
        KeyBinding::new(&key("q"), Quit, None),
        KeyBinding::new(&key("k"), ToggleCommandPalette, None),
        KeyBinding::new(&key("shift-p"), ToggleCommandPalette, None),
        KeyBinding::new(&key("n"), NewConnection, None),
        KeyBinding::new(&key("shift-n"), NewSimulator, None),
        KeyBinding::new(&key("b"), ToggleSidebar, None),
        KeyBinding::new(&key("i"), ToggleInspector, None),
        KeyBinding::new(&key("d"), SplitRight, Some(PANE_KEYS)),
        KeyBinding::new(&key("shift-d"), SplitDown, Some(PANE_KEYS)),
        KeyBinding::new(&key("w"), ClosePane, Some(PANE_KEYS)),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, None),
        KeyBinding::new(&key("shift-]"), NextTab, None),
        KeyBinding::new(&key("shift-["), PreviousTab, None),
        KeyBinding::new(&key("enter"), ToggleSession, Some(PANE_KEYS)),
        KeyBinding::new(&key("1"), ShowPoints, Some(PANE_KEYS)),
        KeyBinding::new(&key("2"), ShowTrend, Some(PANE_KEYS)),
        KeyBinding::new(&key("3"), ShowTraffic, Some(PANE_KEYS)),
        KeyBinding::new(&key("4"), ShowLog, Some(PANE_KEYS)),
        KeyBinding::new("down", SelectNextPoint, Some(POINT_KEYS)),
        KeyBinding::new("up", SelectPreviousPoint, Some(POINT_KEYS)),
        KeyBinding::new("escape", Dismiss, Some(WORKSPACE)),
        KeyBinding::new("up", PaletteUp, Some(PALETTE)),
        KeyBinding::new("down", PaletteDown, Some(PALETTE)),
        KeyBinding::new("enter", PaletteConfirm, Some(PALETTE)),
        KeyBinding::new("escape", Dismiss, Some(PALETTE)),
        KeyBinding::new(&key("l"), FocusSend, Some(PANE_KEYS)),
        KeyBinding::new("up", SendHistoryPrevious, Some(SERIAL_SEND)),
        KeyBinding::new("down", SendHistoryNext, Some(SERIAL_SEND)),
        KeyBinding::new("ctrl-1", SendMacro1, Some(SERIAL_SEND)),
        KeyBinding::new("ctrl-2", SendMacro2, Some(SERIAL_SEND)),
        KeyBinding::new("ctrl-3", SendMacro3, Some(SERIAL_SEND)),
        KeyBinding::new("ctrl-4", SendMacro4, Some(SERIAL_SEND)),
        KeyBinding::new("ctrl-5", SendMacro5, Some(SERIAL_SEND)),
        KeyBinding::new("ctrl-6", SendMacro6, Some(SERIAL_SEND)),
        KeyBinding::new("ctrl-7", SendMacro7, Some(SERIAL_SEND)),
        KeyBinding::new("ctrl-8", SendMacro8, Some(SERIAL_SEND)),
        KeyBinding::new("ctrl-9", SendMacro9, Some(SERIAL_SEND)),
    ]
}

pub fn menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "Voltrus Tools".into(),
            items: vec![
                MenuItem::action("Choose Theme…", ChooseTheme),
                MenuItem::separator(),
                MenuItem::action("Quit Voltrus Tools", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Connection…", NewConnection),
                MenuItem::action("New Simulator…", NewSimulator),
                MenuItem::separator(),
                MenuItem::action("Close Pane", ClosePane),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Cut", vt_ui::text_input::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", vt_ui::text_input::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", vt_ui::text_input::Paste, OsAction::Paste),
                MenuItem::os_action(
                    "Select All",
                    vt_ui::text_input::SelectAll,
                    OsAction::SelectAll,
                ),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Command Palette", ToggleCommandPalette),
                MenuItem::separator(),
                MenuItem::action("Points", ShowPoints),
                MenuItem::action("Trend", ShowTrend),
                MenuItem::action("Traffic", ShowTraffic),
                MenuItem::action("Log", ShowLog),
                MenuItem::separator(),
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
                MenuItem::action("Toggle Inspector", ToggleInspector),
            ],
        },
        Menu {
            name: "Session".into(),
            items: vec![
                MenuItem::action("Connect or Disconnect", ToggleSession),
                MenuItem::separator(),
                MenuItem::action("Split Right", SplitRight),
                MenuItem::action("Split Down", SplitDown),
                MenuItem::separator(),
                MenuItem::action("Next Tab", NextTab),
                MenuItem::action("Previous Tab", PreviousTab),
            ],
        },
    ]
}
