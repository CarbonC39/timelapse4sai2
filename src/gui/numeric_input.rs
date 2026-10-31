use std::{fmt::Display, hash::Hash, ops::RangeInclusive, str::FromStr};

use eframe::egui;

/// Values supported by [`numeric_input`].
pub trait NumericInputValue: Copy + Display + FromStr + PartialOrd + PartialEq + 'static {
    fn is_finite(self) -> bool;
}

macro_rules! integer_value {
    ($($type:ty),* $(,)?) => {$(
        impl NumericInputValue for $type {
            fn is_finite(self) -> bool { true }
        }
    )*};
}

integer_value!(u8, u32, u64, i32);

impl NumericInputValue for f32 {
    fn is_finite(self) -> bool {
        f32::is_finite(self)
    }
}

impl NumericInputValue for f64 {
    fn is_finite(self) -> bool {
        f64::is_finite(self)
    }
}

#[derive(Clone, Default)]
struct InputState {
    text: String,
    last_value: String,
    editing: bool,
}

/// A text based numeric editor that keeps its draft separate from the value.
/// The value is changed only when a valid draft is submitted with Enter or blur.
pub fn numeric_input<T, I>(
    ui: &mut egui::Ui,
    id: I,
    value: &mut T,
    range: RangeInclusive<T>,
    unit: &str,
) -> egui::Response
where
    T: NumericInputValue,
    I: Hash,
{
    let widget_id = ui.make_persistent_id(id);
    let state_id = widget_id.with("numeric_input_state");
    let value_text = value.to_string();
    let mut state = ui
        .ctx()
        .data(|data| data.get_temp::<InputState>(state_id))
        .unwrap_or_else(|| InputState {
            text: value_text.clone(),
            last_value: value_text.clone(),
            editing: false,
        });

    if !state.editing && state.last_value != value_text {
        state.text.clone_from(&value_text);
        state.last_value.clone_from(&value_text);
    }

    let response = ui
        .horizontal(|ui| {
            let unit_width = if unit.is_empty() {
                0.0
            } else {
                ui.painter()
                    .layout_no_wrap(
                        unit.to_owned(),
                        ui.style().text_styles[&egui::TextStyle::Body].clone(),
                        ui.visuals().text_color(),
                    )
                    .size()
                    .x
            };
            let unit_gap = if unit.is_empty() {
                0.0
            } else {
                ui.spacing().item_spacing.x
            };
            let margin = egui::Margin::symmetric(4, 2);
            let input_width = 72.0_f32.min(
                (ui.available_width() - unit_width - unit_gap - margin.sum().x).max(35.0),
            );
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.text)
                    .id(widget_id)
                    .margin(margin)
                    .desired_width(input_width),
            );
            if !unit.is_empty() {
                ui.add(egui::Label::new(unit).wrap_mode(egui::TextWrapMode::Extend));
            }
            response
        })
        .inner;

    let enter = ui.input(|input| input.key_pressed(egui::Key::Enter))
        && (response.has_focus() || response.lost_focus());
    if response.lost_focus() || enter {
        // Enter can make TextEdit lose focus before its response is inspected;
        // submitting on any loss also covers a focus and submit in one frame.
        if commit(value, &state.text, &range) {
            state.last_value = value.to_string();
            state.text.clone_from(&state.last_value);
        } else if state.last_value != value_text {
            state.text.clone_from(&value_text);
            state.last_value.clone_from(&value_text);
        } else {
            state.text.clone_from(&value_text);
        }
        state.editing = false;
    } else {
        state.editing = response.has_focus();
    }

    ui.ctx().data_mut(|data| data.insert_temp(state_id, state));
    response
}

fn parse_in_range<T: NumericInputValue>(text: &str, range: &RangeInclusive<T>) -> Option<T> {
    let parsed = text.trim().parse::<T>().ok()?;
    (parsed.is_finite() && range.contains(&parsed)).then_some(parsed)
}

fn commit<T: NumericInputValue>(value: &mut T, draft: &str, range: &RangeInclusive<T>) -> bool {
    if let Some(parsed) = parse_in_range(draft, range) {
        *value = parsed;
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{InputState, commit, numeric_input, parse_in_range};
    use eframe::egui::{self, Event, Key, Modifiers, RawInput};

    fn frame(ctx: &egui::Context, events: Vec<Event>, value: &mut u32) -> egui::Id {
        let mut widget_id = None;
        let _ = ctx.run(
            RawInput {
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    widget_id = Some(numeric_input(ui, "headless_integer", value, 1..=120, "").id);
                });
            },
        );
        widget_id.expect("numeric input rendered")
    }

    fn key_event(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn parses_integer_and_float_drafts() {
        assert_eq!(parse_in_range::<u32>("60", &(1..=120)), Some(60));
        assert_eq!(parse_in_range::<f64>("7.5", &(0.0..=10.0)), Some(7.5));
    }

    #[test]
    fn rejects_invalid_and_out_of_range_drafts() {
        assert_eq!(parse_in_range::<u32>("oops", &(1..=120)), None);
        assert_eq!(parse_in_range::<u32>("121", &(1..=120)), None);
        assert_eq!(parse_in_range::<f32>("NaN", &(0.0..=10.0)), None);
        assert_eq!(parse_in_range::<f64>("11.0", &(0.0..=10.0)), None);
    }

    #[test]
    fn commit_accepts_valid_values_and_keeps_old_value_on_rejection() {
        let mut integer = 30_u32;
        assert!(commit(&mut integer, "60", &(1..=120)));
        assert_eq!(integer, 60);
        assert!(!commit(&mut integer, "not a number", &(1..=120)));
        assert_eq!(integer, 60);
        assert!(!commit(&mut integer, "121", &(1..=120)));
        assert_eq!(integer, 60);

        let mut float = 2.0_f64;
        assert!(commit(&mut float, "7.5", &(0.0..=10.0)));
        assert_eq!(float, 7.5);
    }

    #[test]
    fn text_edit_draft_stays_separate_until_enter_submits() {
        let ctx = egui::Context::default();
        let mut value = 30_u32;
        let widget_id = frame(&ctx, vec![], &mut value);
        ctx.memory_mut(|memory| memory.request_focus(widget_id));

        let state_id = widget_id.with("numeric_input_state");
        frame(
            &ctx,
            vec![
                key_event(Key::A, true, Modifiers::COMMAND),
                key_event(Key::Backspace, true, Modifiers::NONE),
            ],
            &mut value,
        );
        assert_eq!(value, 30);
        let draft = ctx.data(|data| data.get_temp::<InputState>(state_id).unwrap().text);
        assert!(
            draft.is_empty(),
            "Ctrl/Command+A followed by Backspace clears the draft"
        );

        frame(
            &ctx,
            vec![
                Event::Text("60".to_owned()),
                key_event(Key::Enter, true, Modifiers::NONE),
            ],
            &mut value,
        );
        assert_eq!(value, 60);
    }
}
