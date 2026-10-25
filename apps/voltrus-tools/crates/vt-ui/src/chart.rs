//! Live charts painted straight to the GPU: a multi-series trend with grid and
//! labels, and an inline sparkline.
//!
//! Samples live in shared [`History`] buffers the app fills as values arrive,
//! so painting a frame never copies them.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    App, Bounds, Hsla, PathBuilder, Pixels, Point, SharedString, TextRun, Window, canvas, fill,
    font, point, px, size,
};

use crate::theme::*;

/// A bounded run of `(time ms, value)` samples, oldest first.
#[derive(Debug)]
pub struct History {
    samples: RefCell<VecDeque<(f64, f64)>>,
    capacity: usize,
}

impl History {
    pub fn new(capacity: usize) -> Rc<Self> {
        Rc::new(Self {
            samples: RefCell::new(VecDeque::with_capacity(capacity.min(4096))),
            capacity: capacity.max(2),
        })
    }

    /// Append a sample; non-finite values break the line instead of
    /// distorting the scale.
    pub fn push(&self, at_ms: f64, value: f64) {
        let mut samples = self.samples.borrow_mut();
        if samples.len() == self.capacity {
            samples.pop_front();
        }
        samples.push_back((at_ms, value));
    }

    pub fn clear(&self) {
        self.samples.borrow_mut().clear();
    }

    pub fn latest(&self) -> Option<(f64, f64)> {
        self.samples.borrow().back().copied()
    }

    pub fn len(&self) -> usize {
        self.samples.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.borrow().is_empty()
    }

    /// Smallest and largest finite value at or after `since_ms`.
    pub fn range(&self, since_ms: f64) -> Option<(f64, f64)> {
        self.samples
            .borrow()
            .iter()
            .filter(|(at, value)| *at >= since_ms && value.is_finite())
            .fold(None, |range, (_, value)| match range {
                None => Some((*value, *value)),
                Some((low, high)) => Some((low.min(*value), high.max(*value))),
            })
    }

    /// Minimum, maximum, mean and count of the finite values at or after
    /// `since_ms`.
    pub fn summary(&self, since_ms: f64) -> Option<Summary> {
        let samples = self.samples.borrow();
        let mut summary: Option<Summary> = None;
        for (_, value) in samples
            .iter()
            .filter(|(at, value)| *at >= since_ms && value.is_finite())
        {
            let entry = summary.get_or_insert(Summary {
                min: *value,
                max: *value,
                mean: 0.0,
                count: 0,
            });
            entry.min = entry.min.min(*value);
            entry.max = entry.max.max(*value);
            entry.count += 1;
            // Running mean: no overflow on long sums.
            #[allow(clippy::cast_precision_loss)]
            let count = entry.count as f64;
            entry.mean += (value - entry.mean) / count;
        }
        summary
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub count: usize,
}

/// One line of a trend.
#[derive(Debug, Clone)]
pub struct Series {
    pub label: SharedString,
    pub color: Hsla,
    pub history: Rc<History>,
    /// Booleans draw as steps.
    pub stepped: bool,
}

/// The value axis of a trend: padded around the data, never zero-height.
pub fn value_scale(low: f64, high: f64) -> (f64, f64) {
    if !low.is_finite() || !high.is_finite() {
        return (0.0, 1.0);
    }
    let span = high - low;
    if span.abs() < f64::EPSILON * high.abs().max(1.0) {
        let pad = (high.abs() * 0.05).max(1.0);
        return (low - pad, high + pad);
    }
    let pad = span * 0.08;
    (low - pad, high + pad)
}

/// Format an axis or value label with sensible precision.
pub fn format_value(value: f64) -> String {
    if !value.is_finite() {
        return "—".into();
    }
    let magnitude = value.abs();
    if magnitude >= 100_000.0 || (magnitude > 0.0 && magnitude < 0.001) {
        format!("{value:.3e}")
    } else if magnitude >= 1000.0 || value.fract() == 0.0 {
        format!("{value:.0}")
    } else if magnitude >= 10.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.3}")
    }
}

/// A full trend: horizontal grid with value labels, time running right to
/// left from `now_ms` back `window_ms`.
pub fn trend_chart(series: Vec<Series>, now_ms: f64, window_ms: f64, cx: &App) -> impl IntoElement {
    let theme = theme(cx).clone();
    canvas(
        |_, _, _| (),
        move |bounds, (), window, cx| {
            paint_trend(&series, now_ms, window_ms, &theme, bounds, window, cx);
        },
    )
    .size_full()
}

const LABEL_GUTTER: f32 = 56.0;

fn paint_trend(
    series: &[Series],
    now_ms: f64,
    window_ms: f64,
    theme: &Theme,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let since = now_ms - window_ms;
    let range = series
        .iter()
        .filter_map(|series| series.history.range(since))
        .reduce(|(low, high), (l, h)| (low.min(l), high.max(h)));
    let (low, high) = match range {
        Some((low, high)) => value_scale(low, high),
        None => (0.0, 1.0),
    };
    let plot = Bounds::new(
        point(bounds.left() + px(LABEL_GUTTER), bounds.top() + px(8.0)),
        size(
            (bounds.size.width - px(LABEL_GUTTER + 8.0)).max(px(10.0)),
            (bounds.size.height - px(24.0)).max(px(10.0)),
        ),
    );

    // Grid and value labels.
    let rows = 4;
    for row in 0..=rows {
        let fraction = f64::from(row) / f64::from(rows);
        let y = plot.top() + plot.size.height * (1.0 - fraction as f32);
        window.paint_quad(fill(
            Bounds::new(point(plot.left(), y), size(plot.size.width, px(1.0))),
            if row == 0 {
                theme.border_strong
            } else {
                theme.border
            },
        ));
        let value = low + (high - low) * fraction;
        paint_label(
            &format_value(value),
            point(bounds.left() + px(4.0), y - px(7.0)),
            theme.fg_dim,
            window,
            cx,
        );
    }
    // Time labels.
    for (fraction, text) in [(0.0, format_span(window_ms)), (1.0, "now".to_string())] {
        let x = plot.left() + plot.size.width * fraction as f32;
        let x = if fraction > 0.5 { x - px(22.0) } else { x };
        paint_label(
            &text,
            point(x, plot.bottom() + px(4.0)),
            theme.fg_dim,
            window,
            cx,
        );
    }

    let to_point = |at: f64, value: f64| -> Point<Pixels> {
        let x = ((at - since) / window_ms).clamp(0.0, 1.0);
        let y = ((value - low) / (high - low)).clamp(0.0, 1.0);
        point(
            plot.left() + plot.size.width * x as f32,
            plot.bottom() - plot.size.height * y as f32,
        )
    };
    for line in series {
        stroke(line, since, &to_point, px(1.75), window);
    }
}

fn stroke(
    series: &Series,
    since: f64,
    to_point: &impl Fn(f64, f64) -> Point<Pixels>,
    width: Pixels,
    window: &mut Window,
) {
    let samples = series.history.samples.borrow();
    let mut builder = PathBuilder::stroke(width);
    let mut drawing = false;
    let mut segments = 0;
    let mut previous: Option<Point<Pixels>> = None;
    for (at, value) in samples.iter().skip_while(|(at, _)| *at < since) {
        if !value.is_finite() {
            drawing = false;
            continue;
        }
        let next = to_point(*at, *value);
        if drawing {
            if series.stepped
                && let Some(previous) = previous
            {
                builder.line_to(point(next.x, previous.y));
            }
            builder.line_to(next);
            segments += 1;
        } else {
            builder.move_to(next);
            drawing = true;
        }
        previous = Some(next);
    }
    if segments == 0 {
        // A lone sample still shows where the value is.
        if let Some(previous) = previous {
            window.paint_quad(
                fill(
                    Bounds::new(
                        point(previous.x - px(2.0), previous.y - px(2.0)),
                        size(px(4.0), px(4.0)),
                    ),
                    series.color,
                )
                .corner_radii(px(2.0)),
            );
        }
        return;
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, series.color);
    }
}

fn paint_label(text: &str, origin: Point<Pixels>, color: Hsla, window: &mut Window, cx: &mut App) {
    let text: SharedString = text.to_owned().into();
    let run = TextRun {
        len: text.len(),
        font: font(MONO),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(text, FONT_CAPTION, &[run], None);
    let _ = line.paint(origin, px(14.0), window, cx);
}

fn format_span(window_ms: f64) -> String {
    let seconds = window_ms / 1000.0;
    if seconds >= 120.0 {
        format!("-{:.0} min", seconds / 60.0)
    } else {
        format!("-{seconds:.0} s")
    }
}

/// A small line of recent values, scaled to its own range.
pub fn sparkline(
    history: Rc<History>,
    color: Hsla,
    now_ms: f64,
    window_ms: f64,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let since = now_ms - window_ms;
            let Some((low, high)) = history.range(since) else {
                return;
            };
            let (low, high) = value_scale(low, high);
            let to_point = |at: f64, value: f64| -> Point<Pixels> {
                let x = ((at - since) / window_ms).clamp(0.0, 1.0);
                let y = ((value - low) / (high - low)).clamp(0.0, 1.0);
                point(
                    bounds.left() + bounds.size.width * x as f32,
                    bounds.bottom() - px(1.0) - (bounds.size.height - px(2.0)) * y as f32,
                )
            };
            let series = Series {
                label: SharedString::default(),
                color,
                history: history.clone(),
                stepped: false,
            };
            stroke(&series, since, &to_point, px(1.25), window);
        },
    )
    .size_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_drops_oldest_beyond_capacity() {
        let history = History::new(3);
        for index in 0..5 {
            history.push(f64::from(index), f64::from(index) * 10.0);
        }
        assert_eq!(history.len(), 3);
        assert_eq!(history.range(0.0), Some((20.0, 40.0)));
        assert_eq!(history.range(4.0), Some((40.0, 40.0)));
    }

    #[test]
    fn flat_and_broken_data_still_get_a_usable_scale() {
        let (low, high) = value_scale(5.0, 5.0);
        assert!(low < 5.0 && high > 5.0);
        assert_eq!(value_scale(f64::NAN, 1.0), (0.0, 1.0));
        let history = History::new(4);
        history.push(0.0, f64::NAN);
        assert_eq!(history.range(0.0), None);
    }
}
