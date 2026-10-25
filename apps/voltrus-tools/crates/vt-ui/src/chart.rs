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
    App, Bounds, ContentMask, Hsla, PathBuilder, Pixels, Point, ShapedLine, SharedString, TextRun,
    Window, canvas, fill, font, point, px, size,
};

use crate::theme::*;

/// A bounded run of `(time ms, value)` samples, oldest first.
#[derive(Debug)]
pub struct History {
    samples: RefCell<VecDeque<(f64, f64)>>,
    capacity: usize,
}

impl History {
    /// An empty history keeping at most `capacity` samples. Storage grows as
    /// samples arrive, so many idle histories cost next to nothing.
    pub fn new(capacity: usize) -> Rc<Self> {
        Rc::new(Self {
            samples: RefCell::new(VecDeque::new()),
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

/// A full trend, time running right to left from `now_ms` back `window_ms`.
///
/// Series of comparable size share one value axis. When their sizes differ by
/// more than [`LANE_RATIO`] (say watts beside hertz), one axis would flatten
/// all but the largest, so each series gets its own stacked lane with its own
/// scale, name and latest value: the usual industrial trend layout. When the
/// chart is too short for every lane to get [`MIN_LANE_HEIGHT`], the series
/// share one axis after all. Nothing paints outside the chart's bounds.
pub fn trend_chart(series: Vec<Series>, now_ms: f64, window_ms: f64, cx: &App) -> impl IntoElement {
    let theme = theme(cx).clone();
    canvas(
        |_, _, _| (),
        move |bounds, (), window, cx| {
            let since = now_ms - window_ms;
            let ranges: Vec<Option<(f64, f64)>> = series
                .iter()
                .map(|series| series.history.range(since))
                .collect();
            let known: Vec<(f64, f64)> = ranges.iter().flatten().copied().collect();
            let lanes =
                needs_lanes(&known) && lanes_fit(series.len(), f32::from(bounds.size.height));
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                if lanes {
                    paint_lanes(
                        &series, &ranges, now_ms, window_ms, &theme, bounds, window, cx,
                    );
                } else {
                    paint_shared(
                        &series, &known, now_ms, window_ms, &theme, bounds, window, cx,
                    );
                }
            });
        },
    )
    .size_full()
}

/// How many times larger one series may be than another before they get
/// separate lanes.
pub const LANE_RATIO: f64 = 10.0;

/// The shortest lane worth drawing, header included, in pixels.
pub const MIN_LANE_HEIGHT: f32 = 36.0;

/// Whether series with these value ranges need one lane each instead of a
/// shared axis: true when the largest magnitude exceeds the smallest by more
/// than [`LANE_RATIO`].
pub fn needs_lanes(ranges: &[(f64, f64)]) -> bool {
    if ranges.len() < 2 {
        return false;
    }
    let magnitudes = ranges
        .iter()
        .map(|(low, high)| low.abs().max(high.abs()).max(high - low).max(1e-9));
    let (smallest, largest) = magnitudes.fold((f64::INFINITY, 0.0_f64), |(small, large), m| {
        (small.min(m), large.max(m))
    });
    largest > smallest * LANE_RATIO
}

const LABEL_GUTTER: f32 = 56.0;
/// Height of the time labels under the plot.
const TIME_STRIP: f32 = 18.0;
/// Space between stacked lanes.
const LANE_GAP: f32 = 12.0;

/// Whether `count` lanes, with their gaps and the time strip, fit in a chart
/// `height` pixels tall at [`MIN_LANE_HEIGHT`] each.
fn lanes_fit(count: usize, height: f32) -> bool {
    if count == 0 {
        return false;
    }
    #[allow(clippy::cast_precision_loss)]
    let lanes = count as f32;
    let usable = height - TIME_STRIP - LANE_GAP * (lanes - 1.0);
    usable >= MIN_LANE_HEIGHT * lanes
}

#[allow(clippy::too_many_arguments)]
fn paint_shared(
    series: &[Series],
    ranges: &[(f64, f64)],
    now_ms: f64,
    window_ms: f64,
    theme: &Theme,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let since = now_ms - window_ms;
    let range = ranges
        .iter()
        .copied()
        .reduce(|(low, high), (l, h)| (low.min(l), high.max(h)));
    let (low, high) = match range {
        Some((low, high)) => value_scale(low, high),
        None => (0.0, 1.0),
    };
    let plot = Bounds::new(
        point(bounds.left() + px(LABEL_GUTTER), bounds.top() + px(8.0)),
        size(
            (bounds.size.width - px(LABEL_GUTTER + 8.0)).max(px(10.0)),
            (bounds.size.height - px(8.0 + TIME_STRIP)).max(px(10.0)),
        ),
    );
    paint_grid(plot, low, high, 4, theme, bounds.left(), window, cx);
    paint_time_labels(plot, window_ms, theme, window, cx);
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

#[allow(clippy::too_many_arguments)]
fn paint_lanes(
    series: &[Series],
    ranges: &[Option<(f64, f64)>],
    now_ms: f64,
    window_ms: f64,
    theme: &Theme,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    const HEADER: f32 = 18.0;
    const GAP: f32 = LANE_GAP;
    let since = now_ms - window_ms;
    let count = series.len().max(1);
    #[allow(clippy::cast_precision_loss)]
    let lanes = count as f32;
    let usable = bounds.size.height - px(TIME_STRIP + GAP * (lanes - 1.0));
    let lane_height = (usable * (1.0 / lanes)).max(px(HEADER + 16.0));
    let plot_left = bounds.left() + px(LABEL_GUTTER);
    let plot_width = (bounds.size.width - px(LABEL_GUTTER + 8.0)).max(px(10.0));
    let mut last_plot = None;
    for (index, (line, range)) in series.iter().zip(ranges).enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let top = bounds.top() + (lane_height + px(GAP)) * index as f32;

        // Header: colour key and name on the left, latest value on the right.
        window.paint_quad(
            fill(
                Bounds::new(point(plot_left, top + px(5.0)), size(px(8.0), px(8.0))),
                line.color,
            )
            .corner_radii(px(4.0)),
        );
        paint_text(
            &line.label,
            point(plot_left + px(14.0), top + px(2.0)),
            theme.fg_muted,
            SANS,
            window,
            cx,
        );
        let latest = line
            .history
            .latest()
            .map_or_else(|| "—".to_owned(), |(_, value)| format_value(value));
        let shaped = shape(&latest, theme.fg, MONO, window);
        let _ = shaped.paint(
            point(plot_left + plot_width - shaped.width, top + px(2.0)),
            px(14.0),
            window,
            cx,
        );

        let plot = Bounds::new(
            point(plot_left, top + px(HEADER + 4.0)),
            size(plot_width, (lane_height - px(HEADER + 8.0)).max(px(8.0))),
        );
        let (low, high) = range.map_or((0.0, 1.0), |(low, high)| value_scale(low, high));
        paint_grid(plot, low, high, 1, theme, bounds.left(), window, cx);
        let to_point = |at: f64, value: f64| -> Point<Pixels> {
            let x = ((at - since) / window_ms).clamp(0.0, 1.0);
            let y = ((value - low) / (high - low)).clamp(0.0, 1.0);
            point(
                plot.left() + plot.size.width * x as f32,
                plot.bottom() - plot.size.height * y as f32,
            )
        };
        stroke(line, since, &to_point, px(1.75), window);
        last_plot = Some(plot);
    }
    if let Some(plot) = last_plot {
        paint_time_labels(plot, window_ms, theme, window, cx);
    }
}

/// Horizontal grid lines with their values in the gutter left of the plot.
#[allow(clippy::too_many_arguments)]
fn paint_grid(
    plot: Bounds<Pixels>,
    low: f64,
    high: f64,
    rows: u32,
    theme: &Theme,
    gutter_left: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
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
            point(gutter_left + px(4.0), y - px(7.0)),
            theme.fg_dim,
            window,
            cx,
        );
    }
}

/// The window's span at the left end of the time axis, "now" at the right.
fn paint_time_labels(
    plot: Bounds<Pixels>,
    window_ms: f64,
    theme: &Theme,
    window: &mut Window,
    cx: &mut App,
) {
    paint_label(
        &format_span(window_ms),
        point(plot.left(), plot.bottom() + px(4.0)),
        theme.fg_dim,
        window,
        cx,
    );
    let now = shape("now", theme.fg_dim, MONO, window);
    let _ = now.paint(
        point(plot.right() - now.width, plot.bottom() + px(4.0)),
        px(14.0),
        window,
        cx,
    );
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

fn shape(text: &str, color: Hsla, family: &'static str, window: &mut Window) -> ShapedLine {
    let text: SharedString = text.to_owned().into();
    let run = TextRun {
        len: text.len(),
        font: font(family),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text, FONT_CAPTION, &[run], None)
}

fn paint_text(
    text: &str,
    origin: Point<Pixels>,
    color: Hsla,
    family: &'static str,
    window: &mut Window,
    cx: &mut App,
) {
    let line = shape(text, color, family, window);
    let _ = line.paint(origin, px(14.0), window, cx);
}

fn paint_label(text: &str, origin: Point<Pixels>, color: Hsla, window: &mut Window, cx: &mut App) {
    paint_text(text, origin, color, MONO, window, cx);
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

    #[test]
    fn comparable_series_share_an_axis() {
        assert!(!needs_lanes(&[]));
        assert!(!needs_lanes(&[(56_000.0, 57_000.0)]));
        // Temperature beside frequency: same order of magnitude.
        assert!(!needs_lanes(&[(45.0, 46.5), (49.9, 50.1)]));
    }

    #[test]
    fn far_apart_series_get_lanes() {
        // Watts beside hertz.
        assert!(needs_lanes(&[(56_000.0, 57_000.0), (49.9, 50.1)]));
        // A 0/1 flag beside a pressure in the thousands.
        assert!(needs_lanes(&[(0.0, 1.0), (700.0, 760.0)]));
    }

    #[test]
    fn history_grows_only_as_samples_arrive() {
        let history = History::new(1800);
        assert_eq!(history.samples.borrow().capacity(), 0);
        history.push(0.0, 1.0);
        assert!(history.samples.borrow().capacity() < 1800);
    }

    #[test]
    fn short_charts_fall_back_to_a_shared_axis() {
        // Two lanes need the time strip, one gap and two minimum lanes.
        let two = TIME_STRIP + LANE_GAP + 2.0 * MIN_LANE_HEIGHT;
        assert!(lanes_fit(2, two));
        assert!(!lanes_fit(2, two - 1.0));
        // Twenty series in a 300 px pane would overflow it.
        assert!(!lanes_fit(20, 300.0));
        assert!(!lanes_fit(0, 1000.0));
    }
}
