use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Fits text to terminal cells, reserving one cell for an ellipsis when needed.
pub fn truncate_cells(value: &str, limit: usize) -> String {
    if limit == 0 {
        return String::new();
    }
    if value.width() <= limit {
        return value.into();
    }
    format!("{}…", prefix_cells(value, limit - 1))
}

/// Preserves both ends of identifiers within a terminal-cell budget.
pub fn compact_middle(value: &str, limit: usize) -> String {
    if limit == 0 {
        return String::new();
    }
    if value.width() <= limit {
        return value.into();
    }
    let available = limit - 1;
    let left = prefix_cells(value, available.div_ceil(2));
    let right = suffix_cells(value, available / 2);
    format!("{left}…{right}")
}

/// Keeps the rightmost complete graphemes within a terminal-cell budget.
pub fn tail_cells(value: &str, limit: usize) -> String {
    suffix_cells(value, limit).to_owned()
}

/// Wraps complete graphemes without truncation, preserving explicit empty lines.
/// LF and CRLF delimit source lines; bare carriage returns are preserved.
///
/// Zero width returns one empty line. An indivisible grapheme wider than `width`
/// is preserved on its own line and may exceed the cell budget.
pub fn wrap_cells(value: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut remaining = value;
    while let Some((source, rest)) = remaining.split_once('\n') {
        let source = source.strip_suffix('\r').unwrap_or(source);
        lines.extend(wrap_source_line(source, width));
        remaining = rest;
    }
    lines.extend(wrap_source_line(remaining, width));
    lines
}

/// Counts exactly the rows produced by [`wrap_cells`].
pub fn wrapped_cell_line_count(value: &str, width: usize) -> usize {
    wrap_cells(value, width).len()
}

fn wrap_source_line(source: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0;
    for grapheme in source.graphemes(true) {
        let grapheme_width = grapheme.width();
        if !line.is_empty() && line_width + grapheme_width > width {
            lines.push(std::mem::take(&mut line));
            line_width = 0;
        }
        if grapheme_width > width {
            lines.push(grapheme.to_owned());
            continue;
        }
        line.push_str(grapheme);
        line_width += grapheme_width;
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

fn prefix_cells(value: &str, limit: usize) -> &str {
    let mut end = 0;
    for (index, grapheme) in value.grapheme_indices(true) {
        let next = index + grapheme.len();
        if value[..next].width() > limit {
            break;
        }
        end = next;
    }
    &value[..end]
}

fn suffix_cells(value: &str, limit: usize) -> &str {
    let mut start = value.len();
    for (index, _) in value.grapheme_indices(true).rev() {
        if value[index..].width() > limit {
            break;
        }
        start = index;
    }
    &value[start..]
}
