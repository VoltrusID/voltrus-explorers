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
    // Source builds only: a fixed licence to see the store's sheets. A store
    // build's licence comes from the App Store alone.
    let preview = match args.iter().position(|arg| arg == "--licence-preview") {
        Some(index) if !vt_store::store_build() => match args
            .get(index + 1)
            .and_then(|name| model::licence::Preview::parse(name))
        {
            Some(preview) => Some(preview),
            None => {
                eprintln!(
                    "voltrus-tools: --licence-preview takes one of {}",
                    model::licence::Preview::NAMES.join(", ")
                );
                return ExitCode::FAILURE;
            }
        },
        _ => None,
    };

    // Demo and licence-preview runs never touch the user's saved workspace.
    let boot = if demo.is_some() || preview.is_some() {
        Boot {
            workspace: Workspace::default(),
            store: None,
            notice: None,
        }
    } else {
        load_workspace()
    };

    Application::new()
        .with_assets(vt_ui::Assets)
        .run(move |cx: &mut App| {
            let catalog = Rc::new(ThemeCatalog::bundled());
            let initial = catalog.resolve(&boot.workspace.ui.dark_theme, true);
            vt_ui::init(initial, cx);
            cx.bind_keys(actions::key_bindings());
            cx.bind_keys(model::licence::key_bindings());
            // The app menu starts with About (and, in store builds, Unlock).
            let mut menus = actions::menus();
            if let Some(app_menu) = menus.first_mut() {
                let mut items = vec![gpui::MenuItem::action(
                    "About Voltrus Tools",
                    model::licence::ShowAbout,
                )];
                if vt_store::store_build() {
                    items.push(gpui::MenuItem::action(
                        "Unlock Voltrus Tools…",
                        model::licence::ShowUnlock,
                    ));
                }
                items.push(gpui::MenuItem::separator());
                items.append(&mut app_menu.items);
                app_menu.items = items;
            }
            cx.set_menus(menus);
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
                    if let Some(preview) = preview {
                        model.preview_licence(preview, window, cx);
                    }
                    model
                });
                // Closing the window ends the app: save and stop the sessions
                // here as well, since quit hooks don't run on every path. A
                // second save from the quit hook is harmless.
                let weak = model.downgrade();
                window.on_window_should_close(cx, move |_, cx| {
                    let _ = weak.update(cx, |model, _| model.shutdown());
                    cx.quit();
                    true
                });
                let about = model.downgrade();
                cx.on_action(move |_: &model::licence::ShowAbout, cx| {
                    let _ = about.update(cx, |model, cx| {
                        model.open_licence_sheet(model::licence::Sheet::About, cx);
                    });
                });
                let unlock = model.downgrade();
                cx.on_action(move |_: &model::licence::ShowUnlock, cx| {
                    let _ = unlock.update(cx, |model, cx| {
                        model.open_licence_sheet(model::licence::Sheet::Unlock, cx);
                    });
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
