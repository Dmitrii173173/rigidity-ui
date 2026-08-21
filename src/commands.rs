//! The command palette.
//!
//! One list, opened with ⌘K, holding everything the application can be
//! asked to do. It exists so that the panels do not have to: a button for
//! every action would fill the inspector with things nobody presses twice,
//! and a menu bar would be a second place to look. Anything that is not a
//! parameter lives here and nowhere else.
//!
//! It is also how the keyboard covers the whole application. Every command
//! is reachable by typing part of its name and pressing return, whether or
//! not it has a shortcut of its own.

use eframe::egui::{
    Align, Area, Color32, Context, CornerRadius, Frame, Id, Key, Layout, Margin, Order, RichText,
    ScrollArea, TextEdit, pos2,
};

use crate::engine::Demo;
use crate::theme::{Palette, space};

/// Something the application can be asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    /// Choose a file.
    Open,
    /// Load a built-in scene and its displaced copy.
    Demo(Demo),
    /// Analyse or register, depending on what is loaded.
    Run,
    /// Stop whatever is running.
    Cancel,
    /// Frame everything.
    Fit,
    /// Exchange source and target.
    Swap,
    /// Put the reproducing command line on the clipboard.
    Copy,
    /// Light or dark.
    Theme,
    /// Colour the source by its point-to-plane residual.
    Residuals,
    /// Forget both clouds.
    Clear,
}

impl Command {
    /// What the row says.
    pub(crate) fn label(self) -> String {
        match self {
            Self::Open => "open a point cloud…".to_owned(),
            Self::Demo(demo) => format!("demo scene: {}", demo.label()),
            Self::Run => "run".to_owned(),
            Self::Cancel => "stop".to_owned(),
            Self::Fit => "fit the view".to_owned(),
            Self::Swap => "swap source and target".to_owned(),
            Self::Copy => "copy the command line".to_owned(),
            Self::Theme => "switch theme".to_owned(),
            Self::Residuals => "colour by residual".to_owned(),
            Self::Clear => "clear both clouds".to_owned(),
        }
    }

    /// The key that does the same thing without opening this list.
    pub(crate) fn shortcut(self) -> &'static str {
        match self {
            Self::Open => "⌘O",
            Self::Run => "space",
            Self::Cancel => "esc",
            Self::Fit => "F",
            _ => "",
        }
    }
}

/// The palette's own state.
#[derive(Default)]
pub(crate) struct Commands {
    open: bool,
    /// True on the frame it opens, so the field can take focus once.
    opening: bool,
    query: String,
    selected: usize,
}

impl Commands {
    /// Whether the list is showing.
    ///
    /// While it is, the application's own shortcuts stand down: `f` means
    /// the letter f.
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    /// Opens or closes the list.
    pub(crate) fn toggle(&mut self) {
        self.open = !self.open;
        self.opening = self.open;
        self.query.clear();
        self.selected = 0;
    }

    /// Closes it.
    pub(crate) fn close(&mut self) {
        self.open = false;
    }

    /// Draws the list and returns whatever was chosen.
    pub(crate) fn show(
        &mut self,
        ctx: &Context,
        palette: &Palette,
        available: &[Command],
    ) -> Option<Command> {
        if !self.open {
            return None;
        }

        let matching: Vec<Command> = available
            .iter()
            .copied()
            .filter(|command| {
                let query = self.query.trim().to_lowercase();
                query.is_empty() || command.label().to_lowercase().contains(&query)
            })
            .collect();
        self.selected = self.selected.min(matching.len().saturating_sub(1));

        let (up, down, enter, escape) = ctx.input(|input| {
            (
                input.key_pressed(Key::ArrowUp),
                input.key_pressed(Key::ArrowDown),
                input.key_pressed(Key::Enter),
                input.key_pressed(Key::Escape),
            )
        });
        if escape {
            self.close();
            return None;
        }
        if down && !matching.is_empty() {
            self.selected = (self.selected + 1) % matching.len();
        }
        if up && !matching.is_empty() {
            self.selected = (self.selected + matching.len() - 1) % matching.len();
        }

        const WIDTH: f32 = 420.0;
        let screen = ctx.content_rect();
        let mut chosen = enter
            .then(|| matching.get(self.selected).copied())
            .flatten();

        Area::new(Id::new("commands"))
            .order(Order::Foreground)
            .fixed_pos(pos2(
                (screen.width() - WIDTH) * 0.5,
                (screen.height() * 0.18).round(),
            ))
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                Frame::NONE
                    .fill(palette.raised)
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(space::ROW as i8))
                    .show(ui, |ui| {
                        let field = ui.add(
                            TextEdit::singleline(&mut self.query)
                                .hint_text("type a command")
                                .desired_width(f32::INFINITY)
                                .frame(Frame::NONE),
                        );
                        if self.opening {
                            field.request_focus();
                            self.opening = false;
                        }
                        ui.add_space(space::TIGHT);

                        ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                            for (index, command) in matching.iter().enumerate() {
                                let picked = self.selected == index;
                                let row = Frame::NONE
                                    .fill(if picked {
                                        palette.line
                                    } else {
                                        Color32::TRANSPARENT
                                    })
                                    .corner_radius(CornerRadius::same(6))
                                    .inner_margin(Margin::symmetric(space::ROW as i8, 4))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(command.label())
                                                    .color(if picked {
                                                        palette.text
                                                    } else {
                                                        palette.muted
                                                    })
                                                    .size(12.0),
                                            );
                                            ui.with_layout(
                                                Layout::right_to_left(Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(command.shortcut())
                                                            .color(palette.faint)
                                                            .size(10.0)
                                                            .monospace(),
                                                    );
                                                },
                                            );
                                        });
                                    });
                                let response = ui.interact(
                                    row.response.rect,
                                    Id::new(("command", index)),
                                    eframe::egui::Sense::click(),
                                );
                                if response.clicked() {
                                    chosen = Some(*command);
                                }
                                if response.hovered() {
                                    self.selected = index;
                                }
                            }
                            if matching.is_empty() {
                                ui.add_space(space::ROW);
                                ui.label(
                                    RichText::new("nothing matches")
                                        .color(palette.faint)
                                        .size(11.0),
                                );
                                ui.add_space(space::ROW);
                            }
                        });
                    });
            });

        // A click outside the list dismisses it, which is what every other
        // palette does and therefore what the hand expects.
        if ctx.input(|input| input.pointer.any_click())
            && !ctx.is_pointer_over_egui()
            && chosen.is_none()
        {
            self.close();
        }
        if chosen.is_some() {
            self.close();
        }
        chosen
    }
}
