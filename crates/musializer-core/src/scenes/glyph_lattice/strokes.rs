//! A stroke font for setting the lyric line into the lattice.
//!
//! The lattice draws strokes, not filled shapes, so a letter here is a few
//! polylines on a 4-by-6 grid (y up, baseline at 0), with corners cut at 45
//! degrees so every letter shares the solids' faceted look. Capitals only: a
//! word lit in glyph cells reads as a sign, and capitals are what a sign
//! uses. Authored for this scene; nothing here is traced from a typeface.
//!
//! Characters outside the set fold to their unaccented capital where Latin-1
//! has one (`é` → `E`, `ß` → `SS`) and are otherwise dropped, so a line never
//! draws a box for a character it does not know.

/// One letter: its strokes and its width on the grid (the advance adds a gap).
struct Letter {
    width: f32,
    strokes: &'static [&'static [(f32, f32)]],
}

const GAP: f32 = 1.4;
const SPACE: f32 = 4.4;
/// Letter height on the grid.
pub const CAP: f32 = 6.0;
/// Baseline to baseline, on the grid.
const LEADING: f32 = 9.6;

#[rustfmt::skip]
fn letter(c: char) -> Option<Letter> {
    let l = |width: f32, strokes: &'static [&'static [(f32, f32)]]| Some(Letter { width, strokes });
    match c {
        'A' => l(4.0, &[&[(0.0, 0.0), (0.0, 4.0), (2.0, 6.0), (4.0, 4.0), (4.0, 0.0)], &[(0.0, 2.6), (4.0, 2.6)]]),
        'B' => l(4.0, &[&[(0.0, 3.2), (0.0, 6.0), (3.0, 6.0), (4.0, 5.0), (4.0, 4.2), (3.0, 3.2), (0.0, 3.2), (0.0, 0.0), (3.0, 0.0), (4.0, 1.0), (4.0, 2.2), (3.0, 3.2)]]),
        'C' => l(4.0, &[&[(4.0, 5.0), (3.0, 6.0), (1.0, 6.0), (0.0, 5.0), (0.0, 1.0), (1.0, 0.0), (3.0, 0.0), (4.0, 1.0)]]),
        'D' => l(4.0, &[&[(0.0, 0.0), (0.0, 6.0), (2.6, 6.0), (4.0, 4.6), (4.0, 1.4), (2.6, 0.0), (0.0, 0.0)]]),
        'E' => l(3.6, &[&[(3.6, 6.0), (0.0, 6.0), (0.0, 0.0), (3.6, 0.0)], &[(0.0, 3.1), (2.8, 3.1)]]),
        'F' => l(3.6, &[&[(3.6, 6.0), (0.0, 6.0), (0.0, 0.0)], &[(0.0, 3.1), (2.8, 3.1)]]),
        'G' => l(4.0, &[&[(4.0, 5.0), (3.0, 6.0), (1.0, 6.0), (0.0, 5.0), (0.0, 1.0), (1.0, 0.0), (3.0, 0.0), (4.0, 1.0), (4.0, 2.8), (2.2, 2.8)]]),
        'H' => l(4.0, &[&[(0.0, 0.0), (0.0, 6.0)], &[(4.0, 0.0), (4.0, 6.0)], &[(0.0, 3.0), (4.0, 3.0)]]),
        'I' => l(2.0, &[&[(0.0, 6.0), (2.0, 6.0)], &[(1.0, 6.0), (1.0, 0.0)], &[(0.0, 0.0), (2.0, 0.0)]]),
        'J' => l(4.0, &[&[(1.6, 6.0), (4.0, 6.0), (4.0, 1.0), (3.0, 0.0), (1.0, 0.0), (0.0, 1.0)]]),
        'K' => l(4.0, &[&[(0.0, 0.0), (0.0, 6.0)], &[(4.0, 6.0), (1.2, 3.0), (0.0, 3.0)], &[(1.2, 3.0), (4.0, 0.0)]]),
        'L' => l(3.6, &[&[(0.0, 6.0), (0.0, 0.0), (3.6, 0.0)]]),
        'M' => l(4.8, &[&[(0.0, 0.0), (0.0, 6.0), (2.4, 3.0), (4.8, 6.0), (4.8, 0.0)]]),
        'N' => l(4.0, &[&[(0.0, 0.0), (0.0, 6.0), (4.0, 0.0), (4.0, 6.0)]]),
        'O' => l(4.0, &[&[(1.0, 0.0), (3.0, 0.0), (4.0, 1.0), (4.0, 5.0), (3.0, 6.0), (1.0, 6.0), (0.0, 5.0), (0.0, 1.0), (1.0, 0.0)]]),
        'P' => l(4.0, &[&[(0.0, 0.0), (0.0, 6.0), (3.0, 6.0), (4.0, 5.0), (4.0, 3.8), (3.0, 2.8), (0.0, 2.8)]]),
        'Q' => l(4.0, &[&[(1.0, 0.0), (3.0, 0.0), (4.0, 1.0), (4.0, 5.0), (3.0, 6.0), (1.0, 6.0), (0.0, 5.0), (0.0, 1.0), (1.0, 0.0)], &[(2.4, 1.6), (4.2, -0.4)]]),
        'R' => l(4.0, &[&[(0.0, 0.0), (0.0, 6.0), (3.0, 6.0), (4.0, 5.0), (4.0, 3.8), (3.0, 2.8), (0.0, 2.8)], &[(2.2, 2.8), (4.0, 0.0)]]),
        'S' => l(4.0, &[&[(4.0, 5.0), (3.0, 6.0), (1.0, 6.0), (0.0, 5.0), (0.0, 4.0), (1.0, 3.0), (3.0, 3.0), (4.0, 2.0), (4.0, 1.0), (3.0, 0.0), (1.0, 0.0), (0.0, 1.0)]]),
        'T' => l(4.0, &[&[(0.0, 6.0), (4.0, 6.0)], &[(2.0, 6.0), (2.0, 0.0)]]),
        'U' => l(4.0, &[&[(0.0, 6.0), (0.0, 1.0), (1.0, 0.0), (3.0, 0.0), (4.0, 1.0), (4.0, 6.0)]]),
        'V' => l(4.0, &[&[(0.0, 6.0), (2.0, 0.0), (4.0, 6.0)]]),
        'W' => l(5.2, &[&[(0.0, 6.0), (1.2, 0.0), (2.6, 4.0), (4.0, 0.0), (5.2, 6.0)]]),
        'X' => l(4.0, &[&[(0.0, 6.0), (4.0, 0.0)], &[(4.0, 6.0), (0.0, 0.0)]]),
        'Y' => l(4.0, &[&[(0.0, 6.0), (2.0, 3.0), (4.0, 6.0)], &[(2.0, 3.0), (2.0, 0.0)]]),
        'Z' => l(4.0, &[&[(0.0, 6.0), (4.0, 6.0), (0.0, 0.0), (4.0, 0.0)]]),
        '0' => l(4.0, &[&[(1.0, 0.0), (3.0, 0.0), (4.0, 1.0), (4.0, 5.0), (3.0, 6.0), (1.0, 6.0), (0.0, 5.0), (0.0, 1.0), (1.0, 0.0)], &[(1.2, 1.6), (2.8, 4.4)]]),
        '1' => l(2.6, &[&[(0.0, 4.6), (1.6, 6.0), (1.6, 0.0)], &[(0.2, 0.0), (2.6, 0.0)]]),
        '2' => l(4.0, &[&[(0.0, 5.0), (1.0, 6.0), (3.0, 6.0), (4.0, 5.0), (4.0, 4.0), (0.0, 0.0), (4.0, 0.0)]]),
        '3' => l(4.0, &[&[(0.0, 5.0), (1.0, 6.0), (3.0, 6.0), (4.0, 5.0), (4.0, 4.0), (3.0, 3.0), (4.0, 2.0), (4.0, 1.0), (3.0, 0.0), (1.0, 0.0), (0.0, 1.0)], &[(1.2, 3.0), (3.0, 3.0)]]),
        '4' => l(4.0, &[&[(3.0, 0.0), (3.0, 6.0), (0.0, 2.0), (4.0, 2.0)]]),
        '5' => l(4.0, &[&[(4.0, 6.0), (0.0, 6.0), (0.0, 3.4), (3.0, 3.4), (4.0, 2.4), (4.0, 1.0), (3.0, 0.0), (0.0, 0.0)]]),
        '6' => l(4.0, &[&[(3.4, 6.0), (1.0, 6.0), (0.0, 5.0), (0.0, 1.0), (1.0, 0.0), (3.0, 0.0), (4.0, 1.0), (4.0, 2.2), (3.0, 3.2), (0.0, 3.2)]]),
        '7' => l(4.0, &[&[(0.0, 6.0), (4.0, 6.0), (1.4, 0.0)]]),
        '8' => l(4.0, &[&[(1.0, 3.2), (0.0, 4.2), (0.0, 5.0), (1.0, 6.0), (3.0, 6.0), (4.0, 5.0), (4.0, 4.2), (3.0, 3.2), (1.0, 3.2), (0.0, 2.2), (0.0, 1.0), (1.0, 0.0), (3.0, 0.0), (4.0, 1.0), (4.0, 2.2), (3.0, 3.2)]]),
        '9' => l(4.0, &[&[(4.0, 2.8), (1.0, 2.8), (0.0, 3.8), (0.0, 5.0), (1.0, 6.0), (3.0, 6.0), (4.0, 5.0), (4.0, 1.0), (3.0, 0.0), (0.6, 0.0)]]),
        '!' => l(0.8, &[&[(0.4, 6.0), (0.4, 1.9)], &[(0.4, 0.5), (0.4, 0.0)]]),
        '?' => l(4.0, &[&[(0.0, 5.0), (1.0, 6.0), (3.0, 6.0), (4.0, 5.0), (4.0, 4.0), (2.0, 2.6), (2.0, 1.9)], &[(2.0, 0.5), (2.0, 0.0)]]),
        '.' => l(0.8, &[&[(0.4, 0.5), (0.4, 0.0)]]),
        ',' => l(1.2, &[&[(0.9, 0.7), (0.2, -0.9)]]),
        ':' => l(0.8, &[&[(0.4, 4.0), (0.4, 3.5)], &[(0.4, 0.5), (0.4, 0.0)]]),
        ';' => l(1.2, &[&[(0.8, 4.0), (0.8, 3.5)], &[(0.9, 0.7), (0.2, -0.9)]]),
        '\'' => l(0.8, &[&[(0.4, 6.0), (0.4, 4.6)]]),
        '"' => l(2.0, &[&[(0.4, 6.0), (0.4, 4.6)], &[(1.6, 6.0), (1.6, 4.6)]]),
        '-' => l(3.0, &[&[(0.0, 3.0), (3.0, 3.0)]]),
        '+' => l(4.0, &[&[(0.0, 3.0), (4.0, 3.0)], &[(2.0, 1.0), (2.0, 5.0)]]),
        '=' => l(4.0, &[&[(0.0, 4.0), (4.0, 4.0)], &[(0.0, 2.0), (4.0, 2.0)]]),
        '/' => l(3.0, &[&[(0.0, 0.0), (3.0, 6.0)]]),
        '\\' => l(3.0, &[&[(0.0, 6.0), (3.0, 0.0)]]),
        '(' => l(1.8, &[&[(1.8, 6.4), (0.4, 5.0), (0.4, 1.0), (1.8, -0.4)]]),
        ')' => l(1.8, &[&[(0.0, 6.4), (1.4, 5.0), (1.4, 1.0), (0.0, -0.4)]]),
        '[' => l(1.8, &[&[(1.8, 6.4), (0.4, 6.4), (0.4, -0.4), (1.8, -0.4)]]),
        ']' => l(1.8, &[&[(0.0, 6.4), (1.4, 6.4), (1.4, -0.4), (0.0, -0.4)]]),
        '&' => l(4.4, &[&[(4.4, 0.0), (1.0, 4.2), (1.0, 5.2), (1.8, 6.0), (2.6, 6.0), (3.4, 5.2), (3.4, 4.4), (0.0, 2.0), (0.0, 1.0), (1.0, 0.0), (2.6, 0.0), (4.2, 1.8)]]),
        '#' => l(4.4, &[&[(1.4, 0.0), (1.8, 6.0)], &[(2.8, 0.0), (3.2, 6.0)], &[(0.2, 4.0), (4.4, 4.0)], &[(0.0, 2.0), (4.2, 2.0)]]),
        '*' => l(4.0, &[&[(2.0, 1.4), (2.0, 5.6)], &[(0.2, 4.6), (3.8, 2.4)], &[(0.2, 2.4), (3.8, 4.6)]]),
        '%' => l(4.0, &[&[(0.0, 0.0), (4.0, 6.0)], &[(0.6, 6.0), (0.6, 4.6)], &[(3.4, 1.4), (3.4, 0.0)]]),
        '<' => l(3.0, &[&[(3.0, 5.4), (0.0, 3.0), (3.0, 0.6)]]),
        '>' => l(3.0, &[&[(0.0, 5.4), (3.0, 3.0), (0.0, 0.6)]]),
        '@' => l(4.4, &[&[(3.2, 2.0), (3.2, 4.0), (1.6, 4.0), (1.2, 3.0), (1.6, 2.0), (4.4, 2.0), (4.4, 5.0), (3.4, 6.0), (1.0, 6.0), (0.0, 5.0), (0.0, 1.0), (1.0, 0.0), (3.6, 0.0)]]),
        _ => None,
    }
}

/// Folds a character to the capitals this font has: uppercase, accents off,
/// typographic quotes and dashes to their plain forms.
fn fold(c: char) -> &'static str {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => "A",
        'æ' | 'Æ' => "AE",
        'ç' | 'Ç' => "C",
        'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => "E",
        'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => "I",
        'ñ' | 'Ñ' => "N",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => "O",
        'œ' | 'Œ' => "OE",
        'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => "U",
        'ý' | 'ÿ' | 'Ý' => "Y",
        'ß' => "SS",
        '‘' | '’' | '`' | '´' => "'",
        '“' | '”' | '„' => "\"",
        '–' | '—' | '‐' => "-",
        '…' => "...",
        _ => "",
    }
}

/// Every character a line is set in after folding: the font's own set, with
/// anything else dropped.
fn normalized(text: &str) -> Vec<char> {
    let mut out = Vec::new();
    for c in text.chars() {
        if c.is_whitespace() {
            out.push(' ');
            continue;
        }
        let folded = fold(c);
        if !folded.is_empty() {
            out.extend(folded.chars());
            continue;
        }
        for upper in c.to_uppercase() {
            if letter(upper).is_some() {
                out.push(upper);
            }
        }
    }
    out
}

/// One stroke of a laid-out line, in grid units: x right, y up, the block
/// centred on the origin.
#[derive(Clone, Debug, PartialEq)]
pub struct SetStroke {
    pub points: Vec<[f32; 2]>,
    /// Index of the letter it belongs to, in reading order.
    pub letter: usize,
}

/// A line set in rows, centred on the origin.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetLine {
    pub strokes: Vec<SetStroke>,
    pub letters: usize,
    pub rows: usize,
    /// The block's extent on the grid.
    pub width: f32,
    pub height: f32,
}

fn word_width(word: &[char]) -> f32 {
    let mut width = 0.0;
    for (i, c) in word.iter().enumerate() {
        if let Some(letter) = letter(*c) {
            width += letter.width;
            if i + 1 < word.len() {
                width += GAP;
            }
        }
    }
    width
}

/// Wraps `words` greedily into rows no wider than `limit` grid units.
fn wrap(words: &[Vec<char>], limit: f32) -> Vec<Vec<usize>> {
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut width = 0.0;
    for (index, word) in words.iter().enumerate() {
        let w = word_width(word);
        match rows.last_mut() {
            Some(row) if width + SPACE + w <= limit => {
                row.push(index);
                width += SPACE + w;
            }
            _ => {
                rows.push(vec![index]);
                width = w;
            }
        }
    }
    rows
}

/// Sets `text` into at most four rows inside a box `aspect` times as wide as
/// it is tall, choosing the row count that lets the letters be largest.
/// Returns the layout and the grid-to-box scale (box height 1).
#[must_use]
pub fn set_line(text: &str, aspect: f32) -> (SetLine, f32) {
    let chars = normalized(text);
    let words: Vec<Vec<char>> = chars
        .split(|c| *c == ' ')
        .filter(|word| !word.is_empty())
        .map(<[char]>::to_vec)
        .collect();
    if words.is_empty() {
        return (SetLine::default(), 1.0);
    }
    let total: f32 = words.iter().map(|w| word_width(w)).sum::<f32>()
        + SPACE * (words.len().saturating_sub(1)) as f32;
    let widest = words.iter().map(|w| word_width(w)).fold(0.0f32, f32::max);
    let mut best: Option<(f32, Vec<Vec<usize>>)> = None;
    for count in 1..=4usize {
        let limit = (total / count as f32).max(widest) + 0.01;
        let rows = wrap(&words, limit);
        if rows.len() > 4 {
            continue;
        }
        let width = rows
            .iter()
            .map(|row| {
                row.iter().map(|&i| word_width(&words[i])).sum::<f32>()
                    + SPACE * (row.len().saturating_sub(1)) as f32
            })
            .fold(0.0f32, f32::max);
        let height = CAP + LEADING * (rows.len() - 1) as f32;
        let scale = (aspect / width.max(1.0)).min(1.0 / height);
        // A row is worth adding only if it makes the letters clearly larger.
        if best.as_ref().is_none_or(|(s, _)| scale > *s * 1.12) {
            best = Some((scale, rows));
        }
    }
    let Some((scale, rows)) = best else {
        return (SetLine::default(), 1.0);
    };
    let height = CAP + LEADING * (rows.len() - 1) as f32;
    let mut line = SetLine {
        rows: rows.len(),
        height,
        ..SetLine::default()
    };
    for (row_index, row) in rows.iter().enumerate() {
        let row_width = row.iter().map(|&i| word_width(&words[i])).sum::<f32>()
            + SPACE * (row.len().saturating_sub(1)) as f32;
        line.width = line.width.max(row_width);
        let baseline = height * 0.5 - CAP - LEADING * row_index as f32;
        let mut x = -row_width * 0.5;
        for (word_index, &word) in row.iter().enumerate() {
            if word_index > 0 {
                x += SPACE;
            }
            for c in &words[word] {
                let Some(glyph) = letter(*c) else { continue };
                for stroke in glyph.strokes {
                    line.strokes.push(SetStroke {
                        points: stroke
                            .iter()
                            .map(|(px, py)| [x + px, baseline + py])
                            .collect(),
                        letter: line.letters,
                    });
                }
                line.letters += 1;
                x += glyph.width + GAP;
            }
            x -= GAP;
        }
    }
    (line, scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_letter_and_digit_has_strokes_inside_its_cell() {
        for c in ('A'..='Z').chain('0'..='9') {
            let glyph = letter(c).unwrap_or_else(|| panic!("{c} missing"));
            assert!(!glyph.strokes.is_empty());
            for stroke in glyph.strokes {
                assert!(stroke.len() >= 2, "{c}: a stroke needs two points");
                for (x, y) in *stroke {
                    assert!((-0.01..=glyph.width + 0.3).contains(x), "{c} x {x}");
                    assert!((-1.0..=6.5).contains(y), "{c} y {y}");
                }
            }
        }
    }

    #[test]
    fn accents_fold_and_unknowns_drop() {
        assert_eq!(
            normalized("Café ß"),
            vec!['C', 'A', 'F', 'E', ' ', 'S', 'S']
        );
        assert_eq!(normalized("a→b"), vec!['A', 'B']);
        assert_eq!(normalized("don’t"), vec!['D', 'O', 'N', '\'', 'T']);
    }

    #[test]
    fn a_long_line_wraps_and_stays_centred() {
        let (short, short_scale) = set_line("HI", 1.6);
        assert_eq!(short.rows, 1);
        let (long, long_scale) = set_line(
            "every word of this line has to fit inside the frame somehow",
            1.6,
        );
        assert!(long.rows >= 2, "rows {}", long.rows);
        assert!(long_scale < short_scale);
        assert!(long.width * long_scale <= 1.6 + 1.0e-3);
        assert!(long.height * long_scale <= 1.0 + 1.0e-3);
        let xs: Vec<f32> = long
            .strokes
            .iter()
            .flat_map(|s| s.points.iter().map(|p| p[0]))
            .collect();
        let (lo, hi) = xs
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(*x), hi.max(*x)));
        assert!((lo + hi).abs() < 0.6, "centred: {lo}..{hi}");
        assert_eq!(set_line("   ", 1.6).0.strokes.len(), 0);
    }
}
