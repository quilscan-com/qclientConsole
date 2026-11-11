use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::{text::wrap_cells, theme};

pub(crate) const MIN_LABEL_WIDTH: usize = 12;

pub(crate) struct DetailField {
    label: &'static str,
    value: String,
}

impl DetailField {
    pub(crate) fn new(label: &'static str, value: impl Into<String>) -> Self {
        Self {
            label,
            value: value.into(),
        }
    }
}

/// Generates complete detail rows before a caller selects its viewport.
pub(crate) fn render_fields(
    fields: impl IntoIterator<Item = DetailField>,
    width: u16,
) -> Vec<Line<'static>> {
    let fields = fields.into_iter().collect::<Vec<_>>();
    let label_width = fields
        .iter()
        .map(|field| field.label.width())
        .max()
        .unwrap_or_default()
        .max(MIN_LABEL_WIDTH);
    let prefix_width = label_width + 2;
    let value_width = usize::from(width).saturating_sub(prefix_width);
    fields
        .into_iter()
        .flat_map(|field| {
            let padding = " ".repeat(label_width.saturating_sub(field.label.width()));
            let prefix = format!("{}{padding}: ", field.label);
            wrap_cells(&field.value, value_width)
                .into_iter()
                .enumerate()
                .map(move |(index, value)| {
                    Line::from(vec![
                        Span::styled(
                            if index == 0 {
                                prefix.clone()
                            } else {
                                " ".repeat(prefix_width)
                            },
                            theme::muted(),
                        ),
                        Span::raw(value),
                    ])
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use ratatui::{
        buffer::Buffer,
        layout::Rect,
        text::Line,
        widgets::{Paragraph, Widget},
    };
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;

    use super::{render_fields, DetailField};

    fn row_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn detail_fields_align_unicode_keys_and_values_in_real_buffer() {
        for width in [42, 60, 80, 120] {
            for label in ["Filter", "Prover", "Key🧪e\u{301}"] {
                let lines = render_fields([DetailField::new(label, "Value👩🏽‍💻e\u{301}")], width);
                let row = row_text(&lines[0]);
                let (key, value) = row.split_once(": ").expect("fixed field separator");
                assert_eq!(key.trim_end(), label);
                assert_eq!(key.width(), 12);
                assert_eq!(value, "Value👩🏽‍💻e\u{301}");
                let area = Rect::new(0, 0, width, lines.len() as u16);
                let mut buffer = Buffer::empty(area);
                Paragraph::new(lines).render(area, &mut buffer);
                assert_eq!(buffer[(12, 0)].symbol(), ":");
                assert_eq!(buffer[(13, 0)].symbol(), " ");
                assert_eq!(buffer[(14, 0)].symbol(), "V");
                assert_eq!(buffer[(19, 0)].symbol(), "👩🏽‍💻");
                assert_eq!(buffer[(21, 0)].symbol(), "e\u{301}");
            }
        }
    }

    #[test]
    fn detail_fields_preserve_every_grapheme_with_hanging_indent() {
        let value = "Value👩🏽‍💻e\u{301} ascii ".repeat(30);
        for width in [42, 60, 80, 120] {
            let lines = render_fields([DetailField::new("Prover", &value)], width);
            let area = Rect::new(0, 0, width, lines.len() as u16);
            let mut buffer = Buffer::empty(area);
            Paragraph::new(lines.clone()).render(area, &mut buffer);
            let mut rebuilt = String::new();
            for (index, line) in lines.iter().enumerate() {
                let row = row_text(line);
                assert!(row.width() <= usize::from(width), "{row:?}");
                assert!(!row.contains('…'));
                let part = if index == 0 {
                    row.split_once(": ").unwrap().1
                } else {
                    for x in 0..14 {
                        assert_eq!(buffer[(x, index as u16)].symbol(), " ");
                    }
                    row.strip_prefix(&" ".repeat(14)).expect("hanging indent")
                };
                rebuilt.push_str(part);
                assert!(
                    rebuilt.len() == value.len()
                        || value
                            .grapheme_indices(true)
                            .any(|(offset, _)| offset == rebuilt.len())
                );
            }
            assert_eq!(rebuilt, value);
        }
    }

    #[test]
    fn detail_fields_preserve_explicit_empty_lines() {
        let lines = render_fields(
            [DetailField::new("Status", "First line\r\n\nLast line\n")],
            42,
        );
        assert_eq!(
            lines.iter().map(row_text).collect::<Vec<_>>(),
            vec![
                format!("Status{}: First line", " ".repeat(6)),
                " ".repeat(14),
                format!("{}Last line", " ".repeat(14)),
                " ".repeat(14),
            ]
        );
    }
}
