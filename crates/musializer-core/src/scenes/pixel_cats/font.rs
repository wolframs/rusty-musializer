//! A 5x7 pixel face, drawn here from a table rather than loaded.
//!
//! Every glyph is seven rows of five bits. Lowercase is set in capitals — the
//! genre's look, and what keeps a lyric legible at 7 px — and the handful of
//! accented letters a lyric commonly carries fold to their base letter. Anything
//! else becomes `?`, so nothing outside this table can ever reach the screen:
//! the "every drawn string is ASCII or our own glyphs" rule holds by
//! construction rather than by review.
//!
//! The table was written for this scene. It follows the conventions every 5x7
//! face shares — there are only so many ways to draw an `H` in 35 pixels — and
//! takes its own line where there is room for one (a rounded `S` and `3`, a
//! slab `D`, a `W` whose middle strokes meet low).

use super::raster::{Raster, Rgb};

/// Glyph cell width and height, in pixels at scale 1.
pub const GLYPH_WIDTH: i32 = 5;
pub const GLYPH_HEIGHT: i32 = 7;
/// Horizontal advance at scale 1: the glyph and one column of air.
pub const ADVANCE: i32 = 6;
/// A comfortable line pitch at scale 1.
pub const LINE: i32 = 9;

const fn row(text: &str) -> u8 {
    let bytes = text.as_bytes();
    let mut value = 0u8;
    let mut index = 0;
    while index < 5 {
        value <<= 1;
        if bytes[index] == b'#' {
            value |= 1;
        }
        index += 1;
    }
    value
}

const fn g(rows: [&str; 7]) -> [u8; 7] {
    [
        row(rows[0]),
        row(rows[1]),
        row(rows[2]),
        row(rows[3]),
        row(rows[4]),
        row(rows[5]),
        row(rows[6]),
    ]
}

/// A full cell, for bars and cursors.
pub const BLOCK: char = '\u{2588}';
/// A checkered half-tone cell.
pub const SHADE: char = '\u{2591}';
pub const HEART: char = '\u{2665}';
pub const NOTE: char = '\u{266a}';

#[rustfmt::skip]
fn bits(c: char) -> [u8; 7] {
    match c {
        'A' => g([".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"]),
        'B' => g(["####.", "#...#", "#...#", "####.", "#...#", "#...#", "####."]),
        'C' => g([".###.", "#...#", "#....", "#....", "#....", "#...#", ".###."]),
        'D' => g(["###..", "#..#.", "#...#", "#...#", "#...#", "#..#.", "###.."]),
        'E' => g(["#####", "#....", "#....", "####.", "#....", "#....", "#####"]),
        'F' => g(["#####", "#....", "#....", "####.", "#....", "#....", "#...."]),
        'G' => g([".###.", "#...#", "#....", "#.###", "#...#", "#...#", ".###."]),
        'H' => g(["#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"]),
        'I' => g([".###.", "..#..", "..#..", "..#..", "..#..", "..#..", ".###."]),
        'J' => g(["...##", "....#", "....#", "....#", "#...#", "#...#", ".###."]),
        'K' => g(["#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#"]),
        'L' => g(["#....", "#....", "#....", "#....", "#....", "#....", "#####"]),
        'M' => g(["#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#"]),
        'N' => g(["#...#", "#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#"]),
        'O' => g([".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."]),
        'P' => g(["####.", "#...#", "#...#", "####.", "#....", "#....", "#...."]),
        'Q' => g([".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#"]),
        'R' => g(["####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#"]),
        'S' => g([".###.", "#...#", "#....", ".###.", "....#", "#...#", ".###."]),
        'T' => g(["#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."]),
        'U' => g(["#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."]),
        'V' => g(["#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#.."]),
        'W' => g(["#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#"]),
        'X' => g(["#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#"]),
        'Y' => g(["#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#.."]),
        'Z' => g(["#####", "....#", "...#.", "..#..", ".#...", "#....", "#####"]),
        '0' => g([".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###."]),
        '1' => g(["..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###."]),
        '2' => g([".###.", "#...#", "....#", "..##.", ".#...", "#....", "#####"]),
        '3' => g([".###.", "#...#", "....#", "..##.", "....#", "#...#", ".###."]),
        '4' => g(["...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#."]),
        '5' => g(["#####", "#....", "####.", "....#", "....#", "#...#", ".###."]),
        '6' => g(["..##.", ".#...", "#....", "####.", "#...#", "#...#", ".###."]),
        '7' => g(["#####", "....#", "...#.", "..#..", "..#..", "..#..", "..#.."]),
        '8' => g([".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###."]),
        '9' => g([".###.", "#...#", "#...#", ".####", "....#", "...#.", ".##.."]),
        ' ' => [0; 7],
        '.' => g([".....", ".....", ".....", ".....", ".....", "..#..", "..#.."]),
        ',' => g([".....", ".....", ".....", ".....", "..#..", "..#..", ".#..."]),
        '!' => g(["..#..", "..#..", "..#..", "..#..", "..#..", ".....", "..#.."]),
        '?' => g([".###.", "#...#", "....#", "..##.", "..#..", ".....", "..#.."]),
        ':' => g([".....", "..#..", "..#..", ".....", "..#..", "..#..", "....."]),
        ';' => g([".....", "..#..", "..#..", ".....", "..#..", "..#..", ".#..."]),
        '-' => g([".....", ".....", ".....", ".###.", ".....", ".....", "....."]),
        '_' => g([".....", ".....", ".....", ".....", ".....", ".....", "#####"]),
        '=' => g([".....", ".....", "#####", ".....", "#####", ".....", "....."]),
        '+' => g([".....", "..#..", "..#..", "#####", "..#..", "..#..", "....."]),
        '*' => g([".....", "#.#.#", ".###.", "#####", ".###.", "#.#.#", "....."]),
        '^' => g(["..#..", ".#.#.", "#...#", ".....", ".....", ".....", "....."]),
        '/' => g(["....#", "....#", "...#.", "..#..", ".#...", "#....", "#...."]),
        '\\' => g(["#....", "#....", ".#...", "..#..", "...#.", "....#", "....#"]),
        '|' => g(["..#..", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."]),
        '(' => g(["...#.", "..#..", ".#...", ".#...", ".#...", "..#..", "...#."]),
        ')' => g([".#...", "..#..", "...#.", "...#.", "...#.", "..#..", ".#..."]),
        '[' => g([".###.", ".#...", ".#...", ".#...", ".#...", ".#...", ".###."]),
        ']' => g([".###.", "...#.", "...#.", "...#.", "...#.", "...#.", ".###."]),
        '{' => g(["...##", "..#..", "..#..", "##...", "..#..", "..#..", "...##"]),
        '}' => g(["##...", "..#..", "..#..", "...##", "..#..", "..#..", "##..."]),
        '<' => g(["...#.", "..#..", ".#...", "#....", ".#...", "..#..", "...#."]),
        '>' => g([".#...", "..#..", "...#.", "....#", "...#.", "..#..", ".#..."]),
        '#' => g([".#.#.", ".#.#.", "#####", ".#.#.", "#####", ".#.#.", ".#.#."]),
        '$' => g(["..#..", ".####", "#.#..", ".###.", "..#.#", "####.", "..#.."]),
        '%' => g(["##...", "##..#", "...#.", "..#..", ".#...", "#..##", "...##"]),
        '&' => g([".##..", "#..#.", "#.#..", ".#...", "#.#.#", "#..#.", ".##.#"]),
        '@' => g([".###.", "#...#", "#.###", "#.#.#", "#.###", "#....", ".####"]),
        '~' => g([".....", ".....", ".#...", "#.#.#", "...#.", ".....", "....."]),
        '\'' => g(["..#..", "..#..", ".....", ".....", ".....", ".....", "....."]),
        '"' => g([".#.#.", ".#.#.", ".....", ".....", ".....", ".....", "....."]),
        '`' => g([".#...", "..#..", ".....", ".....", ".....", ".....", "....."]),
        BLOCK => [0b11111; 7],
        SHADE => g(["#.#.#", ".#.#.", "#.#.#", ".#.#.", "#.#.#", ".#.#.", "#.#.#"]),
        HEART => g([".....", ".#.#.", "#####", "#####", ".###.", "..#..", "....."]),
        NOTE => g(["...#.", "...##", "...#.", "...#.", ".###.", "####.", ".##.."]),
        _ => bits('?'),
    }
}

/// The glyph a character is drawn as: capitals, digits, the punctuation in the
/// table, and this face's own few symbols. Accented Latin letters fold to their
/// base letter; typographic quotes and dashes to their ASCII forms.
#[must_use]
pub fn fold(c: char) -> char {
    let c = match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'A',
        'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => 'E',
        'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => 'I',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => 'O',
        'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => 'U',
        'ñ' | 'Ñ' => 'N',
        'ç' | 'Ç' => 'C',
        'ý' | 'ÿ' | 'Ý' => 'Y',
        'ß' => 'S',
        '\u{2018}' | '\u{2019}' | '\u{2032}' => '\'',
        '\u{201c}' | '\u{201d}' | '\u{2033}' => '"',
        '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
        '\u{2026}' => '.',
        '\t' | '\n' | '\r' | '\u{a0}' => ' ',
        other => other,
    };
    match c {
        'a'..='z' => c.to_ascii_uppercase(),
        'A'..='Z' | '0'..='9' | ' ' | BLOCK | SHADE | HEART | NOTE => c,
        c if c.is_ascii_punctuation() => c,
        _ => '?',
    }
}

/// The string a lyric is set as: every character folded.
#[must_use]
pub fn folded(text: &str) -> String {
    text.chars().map(fold).collect()
}

/// Width in pixels of `count` glyphs at horizontal scale `sx`.
#[must_use]
pub fn width_of(count: usize, sx: i32) -> i32 {
    if count == 0 {
        0
    } else {
        count as i32 * ADVANCE * sx - sx
    }
}

/// Width in pixels of `text` at horizontal scale `sx`.
#[must_use]
pub fn width(text: &str, sx: i32) -> i32 {
    width_of(text.chars().count(), sx)
}

/// Draws one glyph with its top-left at `(x, y)`, each bit a `sx` by `sy`
/// block. Returns whether any of it landed inside the raster.
pub fn glyph(raster: &mut Raster, c: char, x: i32, y: i32, sx: i32, sy: i32, color: Rgb) -> bool {
    let (sx, sy) = (sx.max(1), sy.max(1));
    if x >= raster.width()
        || y >= raster.height()
        || x + GLYPH_WIDTH * sx <= 0
        || y + GLYPH_HEIGHT * sy <= 0
    {
        return false;
    }
    let rows = bits(fold(c));
    let mut drew = false;
    for (index, bits) in rows.iter().enumerate() {
        for column in 0..GLYPH_WIDTH {
            if bits & (0b10000 >> column) != 0 {
                raster.rect(x + column * sx, y + index as i32 * sy, sx, sy, color);
                drew = true;
            }
        }
    }
    drew
}

/// Draws `text` with its top-left at `(x, y)`. Returns how many non-space
/// glyphs landed — the number the report line counts as "drew text".
pub fn text(
    raster: &mut Raster,
    text: &str,
    x: i32,
    y: i32,
    sx: i32,
    sy: i32,
    color: Rgb,
) -> usize {
    let mut drawn = 0;
    let mut pen = x;
    for c in text.chars() {
        if c != ' ' && glyph(raster, c, pen, y, sx, sy, color) {
            drawn += 1;
        }
        pen += ADVANCE * sx.max(1);
    }
    drawn
}

/// [`text`] with a hard drop shadow one block down and right.
#[allow(clippy::too_many_arguments)]
pub fn shadowed(
    raster: &mut Raster,
    content: &str,
    x: i32,
    y: i32,
    sx: i32,
    sy: i32,
    color: Rgb,
    shadow: Rgb,
) -> usize {
    let (dx, dy) = ((sx / 2).max(1), (sy / 2).max(1));
    text(raster, content, x + dx, y + dy, sx, sy, shadow);
    text(raster, content, x, y, sx, sy, color)
}

/// Rows of a word-wrapped line, each with the character offset it starts at in
/// the folded text, so a typewriter count maps onto the wrapped rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    /// Index of this row's first character in the folded source.
    pub start: usize,
}

/// Greedy word wrap at `columns` characters. A word longer than a row is broken
/// across rows rather than dropped; leading spaces of a row are swallowed into
/// the break, so a row never starts with air.
#[must_use]
pub fn wrap(text: &str, columns: usize) -> Vec<Row> {
    let columns = columns.max(1);
    let chars: Vec<char> = text.chars().collect();
    let mut rows = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        while start < chars.len() && chars[start] == ' ' {
            start += 1;
        }
        if start >= chars.len() {
            break;
        }
        let end = (start + columns).min(chars.len());
        let cut = if end == chars.len() || chars[end] == ' ' {
            end
        } else {
            match chars[start..end].iter().rposition(|&c| c == ' ') {
                Some(space) if space > 0 => start + space,
                _ => end,
            }
        };
        let row: String = chars[start..cut].iter().collect();
        rows.push(Row {
            text: row.trim_end().to_string(),
            start,
        });
        start = cut;
    }
    rows
}

/// The largest scale in `1..=max_scale` at which `text` wraps into at most
/// `max_rows` rows inside `width` by `height` pixels, with the rows. Scale 1 is
/// returned even when it does not fit, so a line is never simply absent.
#[must_use]
pub fn fit(
    text: &str,
    width: i32,
    height: i32,
    max_rows: usize,
    max_scale: i32,
) -> (i32, Vec<Row>) {
    for scale in (1..=max_scale.max(1)).rev() {
        let columns = ((width + scale) / (ADVANCE * scale)).max(1) as usize;
        let rows = wrap(text, columns);
        let tall = rows.len() as i32 * LINE * scale - (LINE - GLYPH_HEIGHT) * scale;
        let longest = rows
            .iter()
            .map(|row| row.text.chars().count())
            .max()
            .unwrap_or(0);
        let longest_word = text
            .split(' ')
            .map(|w| w.chars().count())
            .max()
            .unwrap_or(0);
        if rows.len() <= max_rows && tall <= height && longest_word <= columns && longest <= columns
        {
            return (scale, rows);
        }
    }
    let columns = ((width + 1) / ADVANCE).max(1) as usize;
    (1, wrap(text, columns))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_fits_its_cell_and_lowercase_is_set_in_capitals() {
        for c in (' '..='~').chain([BLOCK, SHADE, HEART, NOTE]) {
            let rows = bits(fold(c));
            assert!(rows.iter().all(|row| *row < 32), "{c:?}");
        }
        assert_eq!(fold('a'), 'A');
        assert_eq!(fold('é'), 'E');
        assert_eq!(fold('\u{2019}'), '\'');
        assert_eq!(fold('\u{4e2d}'), '?');
        assert_eq!(bits(fold('a')), bits('A'));
        // Every visible ASCII character has ink, and space has none.
        for c in '!'..='~' {
            assert!(bits(fold(c)).iter().any(|row| *row != 0), "{c:?} is blank");
        }
        assert!(bits(' ').iter().all(|row| *row == 0));
    }

    #[test]
    fn a_folded_string_is_ascii_or_this_faces_own_glyphs() {
        let text = folded("Ça va? «Ünïcödé» 中文 — ok…");
        assert!(text
            .chars()
            .all(|c| c.is_ascii() || [BLOCK, SHADE, HEART, NOTE].contains(&c)));
    }

    #[test]
    fn wrapping_breaks_at_spaces_and_never_loses_a_character() {
        let rows = wrap("I FOUND A TERMINAL BEHIND THE DRUMS", 12);
        assert_eq!(
            rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
            ["I FOUND A", "TERMINAL", "BEHIND THE", "DRUMS"]
        );
        assert!(rows.iter().all(|row| row.text.chars().count() <= 12));
        let long = wrap("SUPERCALIFRAGILISTIC", 8);
        assert_eq!(long.len(), 3);
        assert_eq!(long[1].start, 8);
        assert!(wrap("", 8).is_empty());
        assert!(wrap("    ", 8).is_empty());
    }

    #[test]
    fn the_fitted_scale_is_the_largest_that_fits() {
        let (scale, rows) = fit("SUDO MEOW", 320, 60, 2, 6);
        assert!(scale >= 4, "{scale}");
        assert!(rows.len() <= 2);
        for row in &rows {
            assert!(width(&row.text, scale) <= 320);
        }
        let (small, _) = fit(
            "A VERY LONG LINE THAT HAS TO WRAP OVER A FEW ROWS",
            120,
            40,
            3,
            4,
        );
        assert_eq!(small, 1);
    }

    #[test]
    fn drawing_counts_the_glyphs_that_landed() {
        let mut raster = Raster::new(40, 10);
        assert_eq!(text(&mut raster, "HI !", 0, 0, 1, 1, Rgb::WHITE), 3);
        assert!(raster.pixels().contains(&Rgb::WHITE));
        assert_eq!(text(&mut raster, "OFF", 100, 100, 1, 1, Rgb::WHITE), 0);
        assert_eq!(width("HI", 1), 11);
        assert_eq!(width("HI", 2), 22);
    }
}
