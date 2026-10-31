//! Plain terminal presentation and numbered control selection for WIT panels.
use crate::runtime::panel_types::*;
use anyhow::{Context, Result, bail, ensure};
use unicode_width::UnicodeWidthStr;

// Escape terminal controls, but retain text line breaks. Never emit plugin ANSI sequences.
fn plain(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c == '\n' || !c.is_control() {
            out.push(c);
        } else {
            out.extend(c.escape_default());
        }
    }
    out
}
fn inline(s: &str) -> String {
    plain(s).replace('\n', "\\n")
}
fn styled(s: &Styled) -> String {
    let prefix = match s.tone {
        Tone::Normal | Tone::Dim => "",
        Tone::Accent => "Note: ",
        Tone::Warning => "Warning: ",
        Tone::Error => "Error: ",
        Tone::Done => "Done: ",
    };
    format!("{prefix}{}", plain(&s.text))
}
pub fn status(s: &Status) -> String {
    let state = match s.state {
        WidgetState::Idle => "Idle",
        WidgetState::Busy => "Busy",
        WidgetState::Attention => "Attention",
        WidgetState::Error => "Error",
    };
    let progress = s
        .progress
        .map(|p| format!(" — {:.0}%", p * 100.0))
        .unwrap_or_default();
    format!(
        "Status: {state}{progress} — {}",
        s.lines
            .iter()
            .map(|s| inline(s))
            .collect::<Vec<_>>()
            .join(" — ")
    )
}
#[derive(Clone, Copy)]
enum Control<'a> {
    Button(&'a Button),
    Checkbox(&'a Checkbox),
    Text(&'a TextInput),
    Number(&'a NumberInput),
    Choice(&'a Choice),
}
impl Control<'_> {
    fn id(&self) -> &str {
        match self {
            Self::Button(c) => &c.id,
            Self::Checkbox(c) => &c.id,
            Self::Text(c) => &c.id,
            Self::Number(c) => &c.id,
            Self::Choice(c) => &c.id,
        }
    }
    fn label(&self) -> &str {
        match self {
            Self::Button(c) => &c.label,
            Self::Checkbox(c) => &c.label,
            Self::Text(c) => &c.label,
            Self::Number(c) => &c.label,
            Self::Choice(c) => &c.label,
        }
    }
    fn value(&self) -> Option<Value> {
        match self {
            Self::Button(_) => None,
            Self::Checkbox(c) => Some(Value::Checked(c.checked)),
            Self::Text(c) => Some(Value::Text(c.value.clone())),
            Self::Number(c) => Some(Value::Number(c.value)),
            Self::Choice(c) => Some(Value::Selected(c.selected)),
        }
    }
}
fn controls(items: &[Item]) -> Vec<Control<'_>> {
    let mut result = vec![];
    for item in items {
        match item {
            Item::Button(c) => result.push(Control::Button(c)),
            Item::Checkbox(c) => result.push(Control::Checkbox(c)),
            Item::TextInput(c) => result.push(Control::Text(c)),
            Item::NumberInput(c) => result.push(Control::Number(c)),
            Item::Choice(c) => result.push(Control::Choice(c)),
            Item::Row(cells) => {
                for c in cells {
                    match c {
                        Cell::Button(c) => result.push(Control::Button(c)),
                        Cell::Checkbox(c) => result.push(Control::Checkbox(c)),
                        Cell::Text(_) => {}
                    }
                }
            }
            _ => {}
        }
    }
    result
}
fn control(c: Control<'_>, number: &mut usize) -> String {
    *number += 1;
    let head = format!("[{}] {} (id={})", number, inline(c.label()), inline(c.id()));
    match c {
        Control::Button(b) => format!(
            "{head} — button{}",
            if b.enabled { "" } else { " (disabled)" }
        ),
        Control::Checkbox(c) => format!("{head} = {} [checkbox; true/false]", c.checked),
        Control::Text(c) => format!(
            "{head} = {} [text{}; max {} chars]",
            inline(&c.value),
            if c.multiline { ", multiline" } else { "" },
            c.max_chars
        ),
        Control::Number(c) => format!(
            "{head} = {} {} [number; {}..{}; step {}]",
            c.value,
            inline(&c.unit),
            c.min,
            c.max,
            c.step
        ),
        Control::Choice(c) => format!(
            "{head} = {} [choice: {}]",
            c.selected.saturating_add(1),
            c.options
                .iter()
                .enumerate()
                .map(|(n, s)| format!("{}: {}", n + 1, inline(s)))
                .collect::<Vec<_>>()
                .join("; ")
        ),
    }
}
fn cell(c: &Cell, number: &mut usize) -> String {
    match c {
        Cell::Text(s) => styled(s),
        Cell::Button(c) => control(Control::Button(c), number),
        Cell::Checkbox(c) => control(Control::Checkbox(c), number),
    }
}
/// Adjacent WIT rows form a table. Multiline cells retain their column alignment.
pub fn render(items: &[Item]) -> String {
    let mut out = String::new();
    let mut number = 0;
    let mut pos = 0;
    while pos < items.len() {
        if matches!(items[pos], Item::Row(_)) {
            let mut rows = vec![];
            while let Some(Item::Row(cells)) = items.get(pos) {
                rows.push(
                    cells
                        .iter()
                        .map(|c| cell(c, &mut number))
                        .collect::<Vec<_>>(),
                );
                pos += 1;
            }
            let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
            let mut widths = vec![0; columns];
            for row in &rows {
                for (i, value) in row.iter().enumerate() {
                    widths[i] = widths[i].max(
                        value
                            .split('\n')
                            .map(UnicodeWidthStr::width)
                            .max()
                            .unwrap_or(0),
                    );
                }
            }
            for row in rows {
                let lines = row
                    .iter()
                    .map(|v| v.split('\n').collect::<Vec<_>>())
                    .collect::<Vec<_>>();
                let height = lines.iter().map(Vec::len).max().unwrap_or(1);
                for line in 0..height {
                    for (i, parts) in lines.iter().enumerate() {
                        let value = parts.get(line).copied().unwrap_or("");
                        out.push_str(value);
                        if i + 1 < lines.len() {
                            out.push_str(&" ".repeat(widths[i].saturating_sub(value.width())));
                            out.push_str("  |  ");
                        }
                    }
                    out.push('\n');
                }
            }
            continue;
        }
        let text = match &items[pos] {
            Item::Heading(s) => format!("\n== {} ==", plain(s)),
            Item::Text(s) => styled(s),
            Item::Status(s) => status(s),
            Item::Separator => "----------------------------------------".into(),
            Item::Button(c) => control(Control::Button(c), &mut number),
            Item::Checkbox(c) => control(Control::Checkbox(c), &mut number),
            Item::TextInput(c) => control(Control::Text(c), &mut number),
            Item::NumberInput(c) => control(Control::Number(c), &mut number),
            Item::Choice(c) => control(Control::Choice(c), &mut number),
            Item::Row(_) => unreachable!(),
        };
        out.push_str(&text);
        out.push('\n');
        pos += 1;
    }
    out
}
/// Translate a human selection into the same event used by the JSON interface.
/// Input changes include every current input, as required by the WIT action contract.
pub fn action(items: &[Item], line: &str) -> Result<Action> {
    let (selector, supplied) = line
        .trim()
        .split_once('=')
        .map_or((line.trim(), None), |(a, b)| (a.trim(), Some(b.trim())));
    let all = controls(items);
    let matches: Vec<_> = if let Ok(n) = selector.parse::<usize>() {
        all.get(n.wrapping_sub(1)).into_iter().copied().collect()
    } else {
        let by_id: Vec<_> = all.iter().copied().filter(|c| c.id() == selector).collect();
        if by_id.is_empty() {
            all.iter()
                .copied()
                .filter(|c| c.label().eq_ignore_ascii_case(selector))
                .collect()
        } else {
            by_id
        }
    };
    ensure!(
        matches.len() == 1,
        "Choose one control by number, ID or unique label"
    );
    let c = matches[0];
    let changed = match c {
        Control::Button(b) => {
            ensure!(b.enabled, "Button is disabled");
            ensure!(supplied.is_none(), "Buttons take no value");
            None
        }
        Control::Checkbox(b) => Some(Value::Checked(match supplied {
            None => !b.checked,
            Some("true" | "yes" | "on") => true,
            Some("false" | "no" | "off") => false,
            _ => bail!("Checkbox needs true or false"),
        })),
        Control::Text(t) => {
            let s = supplied.context("Use number=value or name=value")?;
            let s = if s.starts_with('"') {
                serde_json::from_str::<String>(s).context("Use a JSON string for quoted text")?
            } else {
                s.into()
            };
            ensure!(
                s.len() <= 4096 && s.chars().count() <= t.max_chars as usize,
                "Text exceeds input limit"
            );
            ensure!(
                t.multiline || !s.contains(['\r', '\n']),
                "Input accepts one line"
            );
            Some(Value::Text(s))
        }
        Control::Number(n) => {
            let v: f64 = supplied
                .context("Use number=value or name=value")?
                .parse()
                .context("Input needs a number")?;
            ensure!(
                v.is_finite() && v >= n.min && v <= n.max,
                "Number must be finite and within {}..{}",
                n.min,
                n.max
            );
            Some(Value::Number(v))
        }
        Control::Choice(c) => {
            let s = supplied.context("Use number=option or name=option")?;
            let index = if let Ok(n) = s.parse::<usize>() {
                n.checked_sub(1)
            } else {
                let found: Vec<_> = c
                    .options
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| o.eq_ignore_ascii_case(s))
                    .collect();
                ensure!(
                    found.len() == 1,
                    "Choose a unique option name or its number"
                );
                Some(found[0].0)
            }
            .context("Choice numbers start at 1")?;
            ensure!(index < c.options.len(), "Unknown choice option");
            Some(Value::Selected(u32::try_from(index)?))
        }
    };
    let mut inputs = vec![];
    for input in all {
        if let Some(value) = if input.id() == c.id() {
            changed.clone().or_else(|| input.value())
        } else {
            input.value()
        } {
            inputs.push((input.id().to_owned(), value));
        }
    }
    ensure!(inputs.len() <= 2000, "Too many event inputs");
    Ok(Action {
        id: c.id().to_owned(),
        inputs,
        camera: None,
    })
}

/// Names the control an action came from, so a number typed after a refresh shows what it hit.
pub fn chosen(items: &[Item], id: &str) -> Option<String> {
    let all = controls(items);
    let index = all.iter().position(|c| c.id() == id)?;
    Some(format!(
        "Chose [{}] {}",
        index + 1,
        inline(all[index].label())
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn chosen_names_the_control_number_and_label() {
        let items = [Item::Button(button())];
        assert_eq!(chosen(&items, "go").as_deref(), Some("Chose [1] Go"));
        assert_eq!(chosen(&items, "gone"), None);
    }
    fn text(s: &str) -> Styled {
        Styled {
            text: s.into(),
            tone: Tone::Normal,
        }
    }
    fn button() -> Button {
        Button {
            id: "go".into(),
            label: "Go".into(),
            enabled: true,
        }
    }
    fn inputs() -> Vec<Item> {
        vec![
            Item::NumberInput(NumberInput {
                id: "size".into(),
                label: "Size".into(),
                value: 10.0,
                min: 1.0,
                max: 1000.0,
                step: 1.0,
                unit: "mm".into(),
            }),
            Item::TextInput(TextInput {
                id: "title".into(),
                label: "Title".into(),
                value: "part".into(),
                multiline: true,
                max_chars: 20,
            }),
            Item::Choice(Choice {
                id: "printer".into(),
                label: "Printer".into(),
                options: vec!["Bench".into(), "Shelf".into()],
                selected: 0,
            }),
            Item::Row(vec![
                Cell::Button(button()),
                Cell::Checkbox(Checkbox {
                    id: "check".into(),
                    label: "Checked".into(),
                    checked: false,
                }),
            ]),
        ]
    }
    #[test]
    fn every_node_and_tone_is_readable() {
        let mut items = vec![Item::Heading("Example".into())];
        for (tone, label) in [
            (Tone::Normal, "plain"),
            (Tone::Dim, "dim"),
            (Tone::Accent, "accent"),
            (Tone::Warning, "careful"),
            (Tone::Error, "failed"),
            (Tone::Done, "finished"),
        ] {
            items.push(Item::Text(Styled {
                text: label.into(),
                tone,
            }));
        }
        items.extend(inputs());
        items.extend([
            Item::Button(button()),
            Item::Checkbox(Checkbox {
                id: "extra".into(),
                label: "Extra".into(),
                checked: true,
            }),
            Item::Status(Status {
                lines: vec!["Printing".into(), "90 s left".into()],
                progress: Some(0.25),
                state: WidgetState::Busy,
            }),
            Item::Separator,
        ]);
        let shown = render(&items);
        for expected in [
            "== Example ==",
            "plain\ndim\nNote: accent\nWarning: careful\nError: failed\nDone: finished",
            "[1] Size (id=size) = 10 mm",
            "[2] Title (id=title) = part",
            "[3] Printer (id=printer) = 1 [choice: 1: Bench; 2: Shelf]",
            "[4] Go (id=go) — button",
            "[5] Checked (id=check) = false",
            "[6] Go",
            "[7] Extra",
            "Status: Busy — 25% — Printing — 90 s left",
            "----------------------------------------",
        ] {
            assert!(shown.contains(expected), "missing {expected}: {shown}");
        }
        for (state, name) in [
            (WidgetState::Idle, "Idle"),
            (WidgetState::Attention, "Attention"),
            (WidgetState::Error, "Error"),
        ] {
            assert_eq!(
                status(&Status {
                    lines: vec![],
                    progress: None,
                    state
                }),
                format!("Status: {name} — ")
            );
        }
    }
    #[test]
    fn tables_align_unicode_and_multiline_cells_without_truncating() {
        let items = vec![
            Item::Row(vec![
                Cell::Text(text("名\ne\u{301}")),
                Cell::Text(text("Value")),
            ]),
            Item::Row(vec![Cell::Text(text("abc")), Cell::Text(text("42\n43"))]),
        ];
        assert_eq!(
            render(&items),
            "名   |  Value\ne\u{301}    |  \nabc  |  42\n     |  43\n"
        );
        let long = "🛠界e\u{301}".repeat(4096);
        assert!(render(&[Item::Text(text(&long))]).contains(&long));
        assert_eq!(render(&[Item::Row(vec![])]), "\n");
        assert_eq!(
            render(&[Item::Text(text("- one\n- two\u{1b}[31m\t"))]),
            "- one\n- two\\u{1b}[31m\\t\n"
        );
    }
    #[test]
    fn selectors_include_row_controls_and_all_input_values() {
        let items = inputs();
        for selector in ["4", "go", "Go"] {
            let a = action(&items, selector).unwrap();
            assert_eq!(a.id, "go");
            assert_eq!(a.inputs.len(), 4);
            assert!(a.camera.is_none());
            assert!(matches!(a.inputs[0].1, Value::Number(10.0)));
        }
        for selector in ["1=12", "size=12", "SIZE=12"] {
            let a = action(&items, selector).unwrap();
            assert_eq!(a.id, "size");
            assert!(matches!(a.inputs[0].1, Value::Number(12.0)));
        }
        assert!(
            matches!(action(&items, "title=\"a\\nb\"").unwrap().inputs[1].1, Value::Text(ref s) if s == "a\nb")
        );
        for selector in ["3=2", "Printer=Shelf"] {
            assert!(matches!(
                action(&items, selector).unwrap().inputs[2].1,
                Value::Selected(1)
            ));
        }
        assert!(matches!(
            action(&items, "5").unwrap().inputs[3].1,
            Value::Checked(true)
        ));
        assert!(matches!(
            action(&items, "check=false").unwrap().inputs[3].1,
            Value::Checked(false)
        ));
    }
    #[test]
    fn invalid_selections_never_produce_actions() {
        let mut items = inputs();
        for line in [
            "0",
            "999",
            "missing",
            "size",
            "size=NaN",
            "size=1001",
            "size=0",
            "go=yes",
            "printer=0",
            "printer=3",
            "printer=missing",
            "check=maybe",
            "title=abcdefghijklmnopqrstu",
        ] {
            assert!(action(&items, line).is_err(), "accepted {line}");
        }
        let mut disabled = button();
        disabled.enabled = false;
        assert!(render(&[Item::Button(disabled.clone())]).contains("button (disabled)"));
        assert!(action(&[Item::Button(disabled)], "1").is_err());
        items.push(Item::Button(button()));
        assert!(action(&items, "Go").is_err());
        assert!(action(&items, "4").is_ok());
        items.push(Item::TextInput(TextInput {
            id: "single".into(),
            label: "Single".into(),
            value: String::new(),
            multiline: false,
            max_chars: 20,
        }));
        assert!(action(&items, "single=\"a\\nb\"").is_err());
        let crowded = vec![
            Item::Row(vec![
                Cell::Checkbox(Checkbox {
                    id: "check".into(),
                    label: "Check".into(),
                    checked: false,
                });
                32
            ]);
            63
        ];
        assert!(
            action(&crowded, "1")
                .unwrap_err()
                .to_string()
                .contains("Too many event inputs")
        );
    }
}
