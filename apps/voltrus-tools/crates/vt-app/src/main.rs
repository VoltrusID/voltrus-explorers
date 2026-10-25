//! Voltrus Tools: industrial protocol tools in one native window.

mod actions;
mod demo;
mod model;
mod views;

use std::process::ExitCode;
use std::rc::Rc;

use gpui::{
    App, AppContext as _, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point,
    px, size,
};
use vt_app_core::{Workspace, store};
use vt_ui::theme::ThemeCatalog;

use model::{AppModel, Boot};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let demo = match args.iter().position(|arg| arg == "--demo") {
        Some(index) => match args
            .get(index + 1)
            .and_then(|name| demo::Scenario::parse(name))
        {
            Some(scenario) => Some(scenario),
            None => {
                eprintln!(
                    "voltrus-tools: --demo takes one of {}",
                    demo::Scenario::NAMES.join(", ")
                );
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };

    let boot = match demo {
        // Demo runs never touch the user's saved workspace.
        Some(_) => Boot {
            workspace: Workspace::default(),
            store: None,
            notice: None,
        },
        None => load_workspace(),
    };

    Application::new()
        .with_assets(vt_ui::Assets)
        .run(move |cx: &mut App| {
            let catalog = Rc::new(ThemeCatalog::bundled());
            let initial = catalog.resolve(&boot.workspace.ui.dark_theme, true);
            vt_ui::init(initial, cx);
            cx.bind_keys(actions::key_bindings());
            cx.set_menus(actions::menus());
            // The model saves on the way out (`AppModel::new` hooks app quit).
            cx.on_action(|_: &actions::Quit, cx| cx.quit());

            // Startup failures exit straight away with a failing status: on
            // macOS `cx.quit()` ends the process with status 0.
            let (engine, events) = match vt_engine::start() {
                Ok(started) => started,
                Err(error) => {
                    eprintln!("voltrus-tools: could not start the session engine: {error}");
                    std::process::exit(1);
                }
            };

            let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Voltrus Tools".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(14.0), px(13.0))),
                }),
                window_min_size: Some(size(px(960.0), px(600.0))),
                app_id: Some("id.voltrus.tools".into()),
                ..WindowOptions::default()
            };
            let opened = cx.open_window(options, move |window, cx| {
                let model = cx.new(|cx| {
                    let mut model = AppModel::new(boot, engine, events, catalog, window, cx);
                    model.apply_theme(window, cx);
                    if let Some(scenario) = demo {
                        demo::apply(&mut model, scenario, window, cx);
                    }
                    model
                });
                window.on_window_should_close(cx, |_, cx| {
                    cx.quit();
                    true
                });
                model
            });
            if let Err(error) = opened {
                eprintln!("voltrus-tools: could not open the window: {error}");
                std::process::exit(1);
            }
            cx.activate(true);
        });
    ExitCode::SUCCESS
}

/// The saved workspace, or a fresh one with a note about why.
fn load_workspace() -> Boot {
    let path = match store::default_path() {
        Ok(path) => path,
        Err(error) => {
            return Boot {
                workspace: Workspace::default(),
                store: None,
                notice: Some(format!("Changes won't be saved: {error}")),
            };
        }
    };
    match store::load(&path) {
        Ok(workspace) => Boot {
            workspace,
            store: Some(path),
            notice: None,
        },
        Err(error @ store::StoreError::Corrupt { .. }) => Boot {
            workspace: Workspace::default(),
            store: Some(path),
            notice: Some(error.to_string()),
        },
        // Unreadable but present: work in memory rather than overwrite it.
        Err(error) => Boot {
            workspace: Workspace::default(),
            store: None,
            notice: Some(format!("{error}. Changes won't be saved this session.")),
        },
    }
}
