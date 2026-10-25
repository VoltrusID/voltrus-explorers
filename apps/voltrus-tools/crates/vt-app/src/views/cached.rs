//! A part of the window drawn by its own view, as in muxy, so gpui can reuse
//! its last frame while something else animates.
//!
//! gpui redraws a notified view and its ancestors. Without this, a live trend
//! asking for a frame would rebuild the whole window: the sidebar, every
//! pane, the inspector. Wrapped in [`CachedView`]s and drawn with
//! `AnyView::cached`, only the trend's pane and the thin window shell rebuild;
//! everything else replays its previous frame until the model changes.

use gpui::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, Render, Subscription, WeakEntity,
    Window, div,
};

use crate::model::AppModel;

type RenderFn = Box<dyn Fn(&mut AppModel, &mut Window, &mut Context<AppModel>) -> AnyElement>;

pub struct CachedView {
    model: WeakEntity<AppModel>,
    render: RenderFn,
    _model_changes: Subscription,
}

impl CachedView {
    /// A view drawing `render` over the model, redrawn whenever the model
    /// notifies.
    pub fn new(
        render: impl Fn(&mut AppModel, &mut Window, &mut Context<AppModel>) -> AnyElement + 'static,
        cx: &mut Context<AppModel>,
    ) -> Entity<Self> {
        let model = cx.entity();
        cx.new(|cx| Self {
            model: model.downgrade(),
            render: Box::new(render),
            _model_changes: cx.observe(&model, |_, _, cx| cx.notify()),
        })
    }
}

impl Render for CachedView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let render = &self.render;
        self.model
            .update(cx, |model, cx| render(model, window, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}
