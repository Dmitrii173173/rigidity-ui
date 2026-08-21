//! The visual language: one palette, one type scale, one spacing grid.
//!
//! Everything drawn takes its colour from here. A literal colour anywhere
//! else is a bug — that is how a two-theme application stays legible in
//! both themes instead of in whichever one it was written in.

use eframe::egui::{
    Color32, Context, CornerRadius, FontFamily, FontId, Margin, Stroke, Style, TextStyle, Theme,
    Vec2, Visuals,
};

/// Which palette is in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// The default.
    Dark,
    /// For bright rooms and for printing screenshots.
    Light,
}

impl Mode {
    /// What the system is set to, unless told otherwise.
    ///
    /// `RIGIDITY_UI_THEME` overrides it, which is how both themes get
    /// looked at without a person having to press anything.
    pub(crate) fn initial(ctx: &Context) -> Self {
        match std::env::var("RIGIDITY_UI_THEME").as_deref() {
            Ok("light") => return Self::Light,
            Ok("dark") => return Self::Dark,
            _ => {}
        }
        match ctx.system_theme() {
            Some(Theme::Light) => Self::Light,
            _ => Self::Dark,
        }
    }

    /// The other one.
    pub(crate) fn flipped(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }

    /// A one-word label for a toggle.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
}

/// Every colour the application is allowed to draw with.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Palette {
    /// Behind the viewport: the darkest surface, so the cloud carries the eye.
    pub(crate) background: Color32,
    /// Panels.
    pub(crate) surface: Color32,
    /// Anything that has to sit visibly on a panel.
    pub(crate) raised: Color32,
    /// A hairline. Used where spacing alone genuinely cannot separate two things.
    pub(crate) line: Color32,
    /// Body text.
    pub(crate) text: Color32,
    /// Labels, units, secondary numbers.
    pub(crate) muted: Color32,
    /// Text that is present but not meant to be read yet.
    pub(crate) faint: Color32,
    /// The one accent. If a second one is ever needed, the layout is wrong.
    pub(crate) accent: Color32,
    /// The cloud that moves during a registration.
    ///
    /// Distinct from [`point`](Self::point) because the whole question on
    /// screen is which of the two is which, and shading alone cannot say.
    pub(crate) point_moving: Color32,
    /// Colours for the third cloud onwards.
    pub(crate) series: [Color32; 4],
    /// An unclassified point in the viewport.
    ///
    /// Not `text`: the viewport is the darker surface in one theme and the
    /// lighter one in the other, and eye-dome lighting only darkens, so a
    /// cloud has to start bright enough to have somewhere to go.
    pub(crate) point: Color32,
    /// A degree of freedom the geometry determines well.
    pub(crate) high: Color32,
    /// One that is determined, but not to the required tolerance.
    pub(crate) medium: Color32,
    /// One the geometry does not determine at all.
    pub(crate) low: Color32,
}

impl Palette {
    /// The colour of the n-th cloud in the scene.
    ///
    /// The first two are the ones a registration has always used, so a
    /// two-cloud scene looks exactly as it did before there was a list.
    /// Beyond that the series has to stay apart from itself as well as
    /// from the background, which is why it is six deliberate colours
    /// rather than a hue rotation — a generated ramp puts two greens next
    /// to each other sooner or later and there is no way to notice.
    pub(crate) fn cloud(&self, index: usize) -> Color32 {
        [self.point, self.point_moving]
            .into_iter()
            .chain(self.series)
            .nth(index % 6)
            .unwrap_or(self.point)
    }

    /// The palette for a mode.
    ///
    /// The three observability colours run teal → amber → orange, along
    /// the blue-yellow axis rather than the red-green one, so the ramp
    /// survives the common colour blindness. Colour is never the only
    /// channel regardless: the state is always spelled out and the bar
    /// length says the same thing again.
    pub(crate) fn of(mode: Mode) -> Self {
        match mode {
            Mode::Dark => Self {
                background: Color32::from_rgb(0x0E, 0x0F, 0x11),
                surface: Color32::from_rgb(0x14, 0x16, 0x19),
                raised: Color32::from_rgb(0x1C, 0x1F, 0x23),
                line: Color32::from_rgb(0x24, 0x28, 0x2D),
                text: Color32::from_rgb(0xE6, 0xE8, 0xEB),
                muted: Color32::from_rgb(0x93, 0x9B, 0xA5),
                faint: Color32::from_rgb(0x79, 0x80, 0x87),
                accent: Color32::from_rgb(0x6A, 0xA9, 0xFF),
                point: Color32::from_rgb(0xC8, 0xCD, 0xD4),
                point_moving: Color32::from_rgb(0x7C, 0xB0, 0xF0),
                series: [
                    Color32::from_rgb(0x8F, 0xD1, 0xA8),
                    Color32::from_rgb(0xD9, 0xA0, 0xD6),
                    Color32::from_rgb(0xE0, 0xC0, 0x8A),
                    Color32::from_rgb(0xE8, 0x9A, 0x9A),
                ],
                high: Color32::from_rgb(0x4F, 0xB8, 0xA8),
                medium: Color32::from_rgb(0xE0, 0xB3, 0x41),
                low: Color32::from_rgb(0xE5, 0x73, 0x4A),
            },
            Mode::Light => Self {
                background: Color32::from_rgb(0xFB, 0xFB, 0xFC),
                surface: Color32::from_rgb(0xF2, 0xF3, 0xF5),
                raised: Color32::from_rgb(0xE8, 0xEA, 0xED),
                line: Color32::from_rgb(0xDD, 0xE0, 0xE4),
                text: Color32::from_rgb(0x16, 0x18, 0x1B),
                muted: Color32::from_rgb(0x5C, 0x63, 0x6C),
                faint: Color32::from_rgb(0x6A, 0x70, 0x77),
                accent: Color32::from_rgb(0x2A, 0x6B, 0xD7),
                point: Color32::from_rgb(0x3A, 0x41, 0x4A),
                point_moving: Color32::from_rgb(0x1E, 0x5F, 0xC8),
                series: [
                    Color32::from_rgb(0x1E, 0x7A, 0x46),
                    Color32::from_rgb(0x7A, 0x3A, 0x94),
                    Color32::from_rgb(0x7A, 0x5A, 0x18),
                    Color32::from_rgb(0x9A, 0x3A, 0x3A),
                ],
                high: Color32::from_rgb(0x1A, 0x7C, 0x6F),
                medium: Color32::from_rgb(0x93, 0x67, 0x17),
                low: Color32::from_rgb(0xBA, 0x4C, 0x28),
            },
        }
    }
}

/// The spacing grid, in points. Every gap in the application is one of these.
pub(crate) mod space {
    /// Between a label and the thing it labels.
    pub(crate) const TIGHT: f32 = 4.0;
    /// Between rows of the same group.
    pub(crate) const ROW: f32 = 8.0;
    /// Between groups.
    pub(crate) const GROUP: f32 = 20.0;
    /// Inside a panel, from its edge.
    pub(crate) const PANEL: i8 = 16;
}

/// Installs the style for a mode. Called once at startup and on every toggle.
pub(crate) fn apply(ctx: &Context, mode: Mode) {
    let palette = Palette::of(mode);
    let mut style = Style {
        visuals: visuals(mode, &palette),
        ..Style::default()
    };

    style.text_styles = [
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(13.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Heading,
            FontId::new(15.0, FontFamily::Proportional),
        ),
        // Numbers live in the monospace style so that columns of them line
        // up. A proportional font turns a table of singular values into a
        // ragged edge that is impossible to scan.
        (
            TextStyle::Monospace,
            FontId::new(12.0, FontFamily::Monospace),
        ),
    ]
    .into();

    style.spacing.item_spacing = Vec2::new(space::ROW, space::TIGHT + 2.0);
    style.spacing.button_padding = Vec2::new(10.0, 4.0);
    style.spacing.window_margin = Margin::same(space::PANEL);
    style.spacing.menu_margin = Margin::same(space::ROW as i8);
    style.spacing.indent = 16.0;
    style.spacing.interact_size = Vec2::new(0.0, 22.0);

    // Both themes get the same style: the mode is ours to switch, not the
    // system's to switch under us mid-session.
    ctx.set_style_of(Theme::Dark, style.clone());
    ctx.set_style_of(Theme::Light, style);
}

fn visuals(mode: Mode, palette: &Palette) -> Visuals {
    let mut visuals = match mode {
        Mode::Dark => Visuals::dark(),
        Mode::Light => Visuals::light(),
    };

    visuals.override_text_color = Some(palette.text);
    visuals.panel_fill = palette.surface;
    visuals.window_fill = palette.surface;
    visuals.extreme_bg_color = palette.background;
    visuals.faint_bg_color = palette.raised;
    visuals.code_bg_color = palette.raised;
    visuals.hyperlink_color = palette.accent;
    visuals.warn_fg_color = palette.medium;
    visuals.error_fg_color = palette.low;
    visuals.selection.bg_fill = palette.accent.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, palette.text);

    // No borders anywhere. Groups are separated by space; when space is
    // genuinely not enough, `palette.line` is drawn deliberately rather
    // than inherited from a widget default.
    visuals.window_stroke = Stroke::NONE;
    visuals.window_shadow = eframe::egui::epaint::Shadow::NONE;
    visuals.popup_shadow = eframe::egui::epaint::Shadow::NONE;
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(8);

    let radius = CornerRadius::same(6);
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = radius;
        widget.bg_stroke = Stroke::NONE;
        widget.expansion = 0.0;
    }
    visuals.widgets.noninteractive.bg_fill = palette.surface;
    visuals.widgets.noninteractive.weak_bg_fill = palette.surface;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.muted);
    visuals.widgets.inactive.bg_fill = palette.raised;
    visuals.widgets.inactive.weak_bg_fill = palette.raised;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.hovered.bg_fill = palette.line;
    visuals.widgets.hovered.weak_bg_fill = palette.line;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.active.bg_fill = palette.accent.gamma_multiply(0.30);
    visuals.widgets.active.weak_bg_fill = palette.accent.gamma_multiply(0.30);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, palette.text);

    visuals
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance.
    fn luminance(colour: Color32) -> f32 {
        let channel = |value: u8| {
            let value = f32::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(colour.r()) + 0.7152 * channel(colour.g()) + 0.0722 * channel(colour.b())
    }

    /// The WCAG contrast ratio between two colours, from 1 to 21.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// Every colour that carries text must be readable on the surface it
    /// is drawn on, in both themes.
    ///
    /// Not a formality. The σ table prints its spreads *in the colour of
    /// their classification*, at ten pixels — so a ramp chosen for how it
    /// looks against a dark panel and never checked against a light one
    /// makes the central number of the application unreadable for half its
    /// users. The threshold is WCAG AA for body text; the values were
    /// chosen by solving for it rather than by eye.
    #[test]
    fn every_colour_that_carries_text_is_legible() {
        for mode in [Mode::Dark, Mode::Light] {
            let palette = Palette::of(mode);
            for (name, colour) in [
                ("text", palette.text),
                ("muted", palette.muted),
                ("faint", palette.faint),
                ("accent", palette.accent),
                ("high", palette.high),
                ("medium", palette.medium),
                ("low", palette.low),
            ] {
                let ratio = contrast(colour, palette.surface);
                assert!(
                    ratio >= 4.5,
                    "{mode:?}: {name} on the panel is {ratio:.2}:1, below AA"
                );
            }
        }
    }

    /// A cloud has to be visible against the viewport, and the two clouds
    /// against each other.
    #[test]
    fn the_clouds_stand_out_from_the_viewport() {
        for mode in [Mode::Dark, Mode::Light] {
            let palette = Palette::of(mode);
            for (name, colour) in [
                ("point", palette.point),
                ("point_moving", palette.point_moving),
                ("high", palette.high),
                ("low", palette.low),
            ] {
                let ratio = contrast(colour, palette.background);
                assert!(
                    ratio >= 3.0,
                    "{mode:?}: {name} in the viewport is {ratio:.2}:1"
                );
            }
            // Every cloud in the series, not just the first two: the
            // fifth is as much a cloud as the first.
            for index in 0..6 {
                let ratio = contrast(palette.cloud(index), palette.background);
                assert!(
                    ratio >= 3.0,
                    "{mode:?}: cloud {index} in the viewport is {ratio:.2}:1"
                );
            }
            let apart = contrast(palette.point, palette.point_moving);
            assert!(
                apart >= 1.4,
                "{mode:?}: the two clouds are {apart:.2}:1 apart, which is not enough to \
                 tell which is which"
            );
        }
    }
}
