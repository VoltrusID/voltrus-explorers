//! A single-line text field, after gpui's `examples/input.rs` (Apache-2.0):
//! IME-aware through `EntityInputHandler`, with themed styling, change and
//! submit events, and horizontal scrolling that keeps the caret in view.

use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, KeyBinding,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    ShapedLine, SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div,
    fill, point, px, relative, size,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::theme::*;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        SelectHome,
        SelectEnd,
        Paste,
        Cut,
        Copy,
        Submit,
        Cancel,
    ]
);

const CONTEXT: &str = "TextInput";
/// What a secure field draws for each character.
const BULLET: char = '•';

pub fn key_bindings() -> Vec<KeyBinding> {
    let primary = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let mut bindings = vec![
        KeyBinding::new("backspace", Backspace, Some(CONTEXT)),
        KeyBinding::new("delete", Delete, Some(CONTEXT)),
        KeyBinding::new("left", Left, Some(CONTEXT)),
        KeyBinding::new("right", Right, Some(CONTEXT)),
        KeyBinding::new("shift-left", SelectLeft, Some(CONTEXT)),
        KeyBinding::new("shift-right", SelectRight, Some(CONTEXT)),
        KeyBinding::new("home", Home, Some(CONTEXT)),
        KeyBinding::new("end", End, Some(CONTEXT)),
        KeyBinding::new("shift-home", SelectHome, Some(CONTEXT)),
        KeyBinding::new("shift-end", SelectEnd, Some(CONTEXT)),
        KeyBinding::new("enter", Submit, Some(CONTEXT)),
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new(&format!("{primary}-a"), SelectAll, Some(CONTEXT)),
        KeyBinding::new(&format!("{primary}-v"), Paste, Some(CONTEXT)),
        KeyBinding::new(&format!("{primary}-c"), Copy, Some(CONTEXT)),
        KeyBinding::new(&format!("{primary}-x"), Cut, Some(CONTEXT)),
    ];
    if cfg!(target_os = "macos") {
        bindings.extend([
            KeyBinding::new("cmd-left", Home, Some(CONTEXT)),
            KeyBinding::new("cmd-right", End, Some(CONTEXT)),
            KeyBinding::new("cmd-shift-left", SelectHome, Some(CONTEXT)),
            KeyBinding::new("cmd-shift-right", SelectEnd, Some(CONTEXT)),
        ]);
    }
    bindings
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    Changed,
    Submitted,
    Cancelled,
    Blurred,
}

/// Which characters the field accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputKind {
    #[default]
    Text,
    /// Digits only.
    Unsigned,
    /// A number: digits, sign, decimal point, exponent.
    Number,
}

impl InputKind {
    fn accepts(self, character: char) -> bool {
        match self {
            Self::Text => !character.is_control(),
            Self::Unsigned => character.is_ascii_digit(),
            Self::Number => character.is_ascii_digit() || "+-.eE".contains(character),
        }
    }
}

pub struct TextInput {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    kind: InputKind,
    mono: bool,
    /// Draw bullets instead of the text, and refuse copy and cut.
    secure: bool,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    scroll_x: Pixels,
    is_selecting: bool,
    was_focused: bool,
}

impl EventEmitter<InputEvent> for TextInput {}

impl TextInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: SharedString::default(),
            placeholder: SharedString::default(),
            kind: InputKind::Text,
            mono: false,
            secure: false,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            scroll_x: px(0.0),
            is_selecting: false,
            was_focused: false,
        }
    }

    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    #[must_use]
    pub fn kind(mut self, kind: InputKind) -> Self {
        self.kind = kind;
        self
    }

    /// Draw in the monospace face (addresses, numbers, hex).
    #[must_use]
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// Mask the content, for passwords.
    #[must_use]
    pub fn secure(mut self) -> Self {
        self.secure = true;
        self
    }

    fn display_index(&self, offset: usize) -> usize {
        display_offset(&self.content, self.secure, offset)
    }

    fn content_index(&self, index: usize) -> usize {
        content_offset(&self.content, self.secure, index)
    }

    #[must_use]
    pub fn text(mut self, text: impl Into<SharedString>) -> Self {
        self.content = text.into();
        self.selected_range = self.content.len()..self.content.len();
        self
    }

    pub fn value(&self) -> &str {
        &self.content
    }

    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.content = text.into();
        self.selected_range = self.content.len()..self.content.len();
        self.marked_range = None;
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus_handle);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Submitted);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Cancelled);
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace('\n', " "), window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.secure && !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.secure && !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        self.is_selecting = true;
        let index = self.index_for_mouse_position(event.position);
        if event.click_count >= 2 {
            self.move_to(0, cx);
            self.select_to(self.content.len(), cx);
        } else if event.modifiers.shift {
            self.select_to(index, cx);
        } else {
            self.move_to(index, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = clamp_to_char_boundary(&self.content, offset);
        self.selected_range = offset..offset;
        cx.notify();
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        // The layout is last frame's; the text may have shrunk since.
        clamp_to_char_boundary(
            &self.content,
            self.content_index(
                line.closest_index_for_x(position.x - bounds.left() + self.scroll_x),
            ),
        )
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = clamp_to_char_boundary(&self.content, offset);
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        utf8_range_to_utf16(&self.content, range)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        utf16_range_to_utf8(&self.content, range_utf16)
    }

    /// The range an input-method edit replaces: the one it names, else the
    /// composition, else the selection.
    fn edit_range(&self, range_utf16: Option<&Range<usize>>) -> Range<usize> {
        range_utf16
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone())
    }

    fn apply(&mut self, edit: Edit, cx: &mut Context<Self>) {
        self.content = edit.content.into();
        self.selected_range = edit.selected;
        self.selection_reversed = false;
        self.marked_range = edit.marked;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(index, _)| (index < offset).then_some(index))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(index, _)| (index > offset).then_some(index))
            .unwrap_or(self.content.len())
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        // A password is never shown to the input method.
        if self.secure {
            return None;
        }
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.edit_range(range_utf16.as_ref());
        // Committing ends any composition, accepted or not.
        self.marked_range = None;
        match replace_edit(&self.content, range, new_text, self.kind) {
            Some(edit) => self.apply(edit, cx),
            None => cx.notify(),
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Secure fields skip composition: only the committed text, which
        // arrives through `replace_text_in_range`, goes in.
        if self.secure {
            return;
        }
        let range = self.edit_range(range_utf16.as_ref());
        if let Some(edit) = mark_edit(
            &self.content,
            range,
            new_text,
            new_selected_range_utf16,
            self.kind,
        ) {
            self.apply(edit, cx);
        }
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(
                bounds.left() + layout.x_for_index(self.display_index(range.start)) - self.scroll_x,
                bounds.top(),
            ),
            point(
                bounds.left() + layout.x_for_index(self.display_index(range.end)) - self.scroll_x,
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let local = self.last_bounds?.localize(&point)?;
        let layout = self.last_layout.as_ref()?;
        let index = self.content_index(layout.index_for_x(local.x + self.scroll_x)?);
        Some(utf8_to_utf16(&self.content, index))
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        if self.was_focused && !focused {
            cx.emit(InputEvent::Blurred);
        }
        self.was_focused = focused;
        let theme = theme(cx);
        div()
            .id(ElementId::from(("text-input", cx.entity_id())))
            .flex()
            .items_center()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .w_full()
            .h(CONTROL_MD)
            .px(SPACE_4)
            .rounded(RADIUS_MD)
            .bg(theme.bg)
            .border_1()
            .border_color(if focused {
                theme.accent
            } else {
                theme.border_strong
            })
            .text_size(FONT_BODY)
            .line_height(px(18.0))
            .text_color(theme.fg)
            .font_family(if self.mono { MONO } else { SANS })
            .child(TextElement { input: cx.entity() })
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
    origin: Point<Pixels>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let theme = theme(cx).clone();
        let input = self.input.read(cx);
        let content = input.content.clone();
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();
        // Offsets in the drawn text: a secure field draws one bullet per character.
        let display_cursor = input.display_index(cursor);
        let display_selection =
            input.display_index(selected_range.start)..input.display_index(selected_range.end);
        let style = window.text_style();

        let (display_text, text_color) = if content.is_empty() {
            (input.placeholder.clone(), theme.fg_dim)
        } else {
            (content, style.color)
        };
        let display_text: SharedString = if input.secure && !input.content.is_empty() {
            BULLET
                .to_string()
                .repeat(input.content.chars().count())
                .into()
        } else {
            display_text
        };
        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        // Composition offsets index the content, not the bullets of a masked field.
        let runs = match input
            .marked_range
            .as_ref()
            .filter(|marked| !input.secure && marked.end <= display_text.len())
        {
            Some(marked) => vec![
                TextRun {
                    len: marked.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked.end - marked.start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: display_text.len() - marked.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect(),
            None => vec![run],
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &runs, None);

        // Keep the caret inside the field.
        let mut scroll_x = input.scroll_x;
        let caret_x = if input.content.is_empty() {
            px(0.0)
        } else {
            line.x_for_index(display_cursor)
        };
        let width = bounds.size.width;
        if caret_x - scroll_x > width - px(2.0) {
            scroll_x = caret_x - width + px(2.0);
        } else if caret_x < scroll_x {
            scroll_x = caret_x;
        }
        if scroll_x < px(0.0) {
            scroll_x = px(0.0);
        }
        self.input.update(cx, |input, _| input.scroll_x = scroll_x);
        let origin = point(bounds.left() - scroll_x, bounds.top());

        let (selection, cursor) = if selected_range.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(origin.x + caret_x, bounds.top()),
                        size(px(1.5), bounds.bottom() - bounds.top()),
                    ),
                    theme.accent,
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            origin.x + line.x_for_index(display_selection.start),
                            bounds.top(),
                        ),
                        point(
                            origin.x + line.x_for_index(display_selection.end),
                            bounds.bottom(),
                        ),
                    ),
                    theme.accent.alpha(0.28),
                )),
                None,
            )
        };
        PrepaintState {
            line: Some(line),
            cursor,
            selection,
            origin,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let Some(line) = prepaint.line.take() else {
            return;
        };
        let origin = prepaint.origin;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(selection) = prepaint.selection.take() {
                window.paint_quad(selection);
            }
            let _ = line.paint(origin, window.line_height(), window, cx);
            if focus_handle.is_focused(window)
                && let Some(cursor) = prepaint.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });
        self.input.update(cx, |input, _| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
        });
    }
}

/// Byte offset in the drawn text of byte `offset` in `content`.
fn display_offset(content: &str, secure: bool, offset: usize) -> usize {
    let offset = clamp_to_char_boundary(content, offset);
    if secure {
        content[..offset].chars().count() * BULLET.len_utf8()
    } else {
        offset
    }
}

/// Byte offset in `content` of byte `index` in the drawn text.
fn content_offset(content: &str, secure: bool, index: usize) -> usize {
    if secure {
        content
            .char_indices()
            .nth(index / BULLET.len_utf8())
            .map_or(content.len(), |(offset, _)| offset)
    } else {
        clamp_to_char_boundary(content, index)
    }
}

/// `offset` pulled back onto the nearest character boundary at or before it,
/// and no further than the end of `text`.
fn clamp_to_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// `range` clamped onto character boundaries of `text`, start first.
fn clamp_range(text: &str, range: Range<usize>) -> Range<usize> {
    let start = clamp_to_char_boundary(text, range.start);
    let end = clamp_to_char_boundary(text, range.end);
    start.min(end)..start.max(end)
}

/// Byte offset in `text` of UTF-16 offset `offset`. An offset inside a
/// surrogate pair rounds up past the character; past the end gives the length.
fn utf16_to_utf8(text: &str, offset: usize) -> usize {
    let mut utf8_offset = 0;
    let mut utf16_count = 0;
    for character in text.chars() {
        if utf16_count >= offset {
            break;
        }
        utf16_count += character.len_utf16();
        utf8_offset += character.len_utf8();
    }
    utf8_offset
}

/// UTF-16 offset in `text` of byte offset `offset`, rounding a byte inside a
/// character up past it.
fn utf8_to_utf16(text: &str, offset: usize) -> usize {
    let mut utf16_offset = 0;
    let mut utf8_count = 0;
    for character in text.chars() {
        if utf8_count >= offset {
            break;
        }
        utf8_count += character.len_utf8();
        utf16_offset += character.len_utf16();
    }
    utf16_offset
}

/// A UTF-16 range of `text` as a byte range on character boundaries, start first.
fn utf16_range_to_utf8(text: &str, range: &Range<usize>) -> Range<usize> {
    let start = utf16_to_utf8(text, range.start.min(range.end));
    let end = utf16_to_utf8(text, range.start.max(range.end));
    start..end
}

fn utf8_range_to_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    utf8_to_utf16(text, range.start)..utf8_to_utf16(text, range.end)
}

/// The characters of `text` that a field of `kind` accepts.
fn filter_text(kind: InputKind, text: &str) -> String {
    text.chars()
        .filter(|character| kind.accepts(*character))
        .collect()
}

/// `content` with `range` replaced by `text`.
fn splice(content: &str, range: Range<usize>, text: &str) -> String {
    let mut spliced = String::with_capacity(content.len() - range.len() + text.len());
    spliced.push_str(&content[..range.start]);
    spliced.push_str(text);
    spliced.push_str(&content[range.end..]);
    spliced
}

/// An edit resolved to byte offsets in the new content.
#[derive(Debug, PartialEq, Eq)]
struct Edit {
    content: String,
    selected: Range<usize>,
    marked: Option<Range<usize>>,
}

/// Replace `range` of `content` with the part of `new_text` a field of `kind`
/// accepts, caret after it. `None` when nothing would change, including when
/// every typed character is rejected: a refused key leaves the selection be.
fn replace_edit(
    content: &str,
    range: Range<usize>,
    new_text: &str,
    kind: InputKind,
) -> Option<Edit> {
    let range = clamp_range(content, range);
    let accepted = filter_text(kind, new_text);
    if accepted.is_empty() && (!new_text.is_empty() || range.is_empty()) {
        return None;
    }
    let caret = range.start + accepted.len();
    Some(Edit {
        content: splice(content, range, &accepted),
        selected: caret..caret,
        marked: None,
    })
}

/// Replace `range` of `content` with the composition `new_text`, marked.
/// `new_selected_utf16` is the input method's caret in UTF-16 units of
/// `new_text` itself, so it is converted within `new_text` and then moved to
/// where the composition starts. Characters `kind` rejects are dropped; if any
/// were, the input method's caret no longer indexes the text and the caret goes
/// after it. `None` when nothing would change.
fn mark_edit(
    content: &str,
    range: Range<usize>,
    new_text: &str,
    new_selected_utf16: Option<Range<usize>>,
    kind: InputKind,
) -> Option<Edit> {
    let range = clamp_range(content, range);
    let accepted = filter_text(kind, new_text);
    if accepted.is_empty() && (!new_text.is_empty() || range.is_empty()) {
        return None;
    }
    let start = range.start;
    let end = start + accepted.len();
    let selected = match new_selected_utf16 {
        Some(selected) if accepted == new_text => {
            let selected = utf16_range_to_utf8(&accepted, &selected);
            start + selected.start..start + selected.end
        }
        _ => end..end,
    };
    Some(Edit {
        content: splice(content, range, &accepted),
        selected,
        marked: (!accepted.is_empty()).then_some(start..end),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masked_offsets_map_through_multibyte_characters() {
        // "pä€" is 1 + 2 + 3 bytes; drawn as three 3-byte bullets.
        let content = "pä€";
        let boundaries = [0, 1, 3, 6];
        for (index, offset) in boundaries.iter().enumerate() {
            let drawn = display_offset(content, true, *offset);
            assert_eq!(drawn, index * 3);
            assert_eq!(content_offset(content, true, drawn), *offset);
        }
        // Plain fields draw the content itself.
        assert_eq!(display_offset(content, false, 3), 3);
        assert_eq!(content_offset(content, false, 3), 3);
        // A stale offset inside "ä" or past the end never slices mid-character.
        assert_eq!(display_offset(content, true, 2), 3);
        assert_eq!(display_offset(content, true, 99), 9);
        assert_eq!(content_offset(content, false, 2), 1);
    }

    #[test]
    fn numeric_fields_reject_letters() {
        assert!(InputKind::Number.accepts('-') && InputKind::Number.accepts('7'));
        assert!(!InputKind::Number.accepts('x'));
        assert!(!InputKind::Unsigned.accepts('.'));
        assert!(!InputKind::Text.accepts('\n'));
    }

    #[test]
    fn offsets_clamp_onto_character_boundaries() {
        // "aé" is 1 + 2 bytes.
        assert_eq!(clamp_to_char_boundary("aé", 0), 0);
        assert_eq!(clamp_to_char_boundary("aé", 2), 1);
        assert_eq!(clamp_to_char_boundary("aé", 3), 3);
        assert_eq!(clamp_to_char_boundary("aé", 10), 3);
        assert_eq!(clamp_to_char_boundary("", 4), 0);
        // A backwards range (IME ranges can arrive reversed) is reordered.
        let (start, end) = (10, 2);
        assert_eq!(clamp_range("aé", start..end), 1..3);
    }

    #[test]
    fn surrogate_pairs_convert_whole() {
        // "😀" is 4 bytes and 2 UTF-16 units.
        let text = "😀x";
        assert_eq!(utf16_to_utf8(text, 2), 4);
        assert_eq!(utf16_to_utf8(text, 3), 5);
        // Inside the pair rounds past the emoji, never into it.
        assert_eq!(utf16_to_utf8(text, 1), 4);
        assert_eq!(utf16_to_utf8(text, 99), 5);
        assert_eq!(utf8_to_utf16(text, 4), 2);
        assert_eq!(utf8_to_utf16(text, 5), 3);
        let (start, end) = (3, 0);
        assert_eq!(utf16_range_to_utf8(text, &(start..end)), 0..5);
        assert_eq!(utf8_range_to_utf16(text, &(0..5)), 0..3);
    }

    #[test]
    fn dead_key_composition_after_text_keeps_the_caret_on_a_boundary() {
        // "ab", caret at the end; ⌥E marks "´" (2 bytes) with the caret after it.
        let marked =
            mark_edit("ab", 2..2, "´", Some(1..1), InputKind::Text).expect("composition starts");
        assert_eq!(marked.content, "ab´");
        assert_eq!(marked.marked, Some(2..4));
        assert_eq!(marked.selected, 4..4);
        assert!(marked.content.is_char_boundary(marked.selected.start));
        // Typing "e" commits "é" over the composition.
        let committed =
            replace_edit(&marked.content, 2..4, "é", InputKind::Text).expect("composition commits");
        assert_eq!(committed.content, "abé");
        assert_eq!(committed.selected, 4..4);
        assert_eq!(committed.marked, None);
    }

    #[test]
    fn emoji_composition_offsets_by_its_insertion_point() {
        let edit =
            mark_edit("😀", 4..4, "👍🏽", Some(2..2), InputKind::Text).expect("composition starts");
        assert_eq!(edit.content, "😀👍🏽");
        assert_eq!(edit.marked, Some(4..12));
        // Two UTF-16 units into the composition: after "👍", before the modifier.
        assert_eq!(edit.selected, 8..8);
    }

    #[test]
    fn rejected_keys_keep_the_selection() {
        assert_eq!(replace_edit("123", 0..3, "x", InputKind::Unsigned), None);
        assert_eq!(
            mark_edit("123", 0..3, "x", Some(0..1), InputKind::Unsigned),
            None
        );
        // Deleting the selection is still an edit; deleting nothing is not.
        let deleted = replace_edit("123", 1..3, "", InputKind::Unsigned).expect("deletes");
        assert_eq!(deleted.content, "1");
        assert_eq!(replace_edit("123", 0..0, "", InputKind::Unsigned), None);
    }

    #[test]
    fn numeric_fields_filter_compositions() {
        let edit =
            mark_edit("", 0..0, "1x2", Some(1..1), InputKind::Unsigned).expect("digits survive");
        assert_eq!(edit.content, "12");
        assert_eq!(edit.marked, Some(0..2));
        // The input method's caret indexed "1x2"; it goes after the kept text.
        assert_eq!(edit.selected, 2..2);
        let committed = replace_edit("12", 0..2, "1y2", InputKind::Unsigned).expect("commits");
        assert_eq!(committed.content, "12");
    }

    #[test]
    fn out_of_range_edits_are_clamped() {
        let edit = replace_edit("aé", 2..10, "z", InputKind::Text).expect("inserts");
        assert_eq!(edit.content, "az");
        assert_eq!(edit.selected, 2..2);
    }
}
