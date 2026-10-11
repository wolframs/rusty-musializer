//! Contact sheets: which moments a sheet shows, where each tile goes, and the
//! arithmetic that shrinks a frame into one (`--contact-sheet`, 2026-10-11).
//!
//! A contact sheet is how something with no window reviews a video. An export
//! answers "what does it look like" with four minutes of file; a sheet answers
//! it with one image that can be read in a glance — every section, with its
//! timecode, its scene and the line on screen written under it. It exists for
//! the same reason `--probe-shot` does: a surface nothing photographs does not
//! get reviewed.
//!
//! Everything here is a number, which is why it is here: stop times, tile
//! rectangles and a box filter are all things a unit test can pin with no
//! window. The application supplies the frames — rendered through the export's
//! own offline path, so a tile is the frame the video will contain — and the
//! one thing that needs a GPU-side resource, the label glyphs.

/// The most tiles one sheet may hold. Past this the tiles are too small to
/// judge anything from and the right tool is a clip.
pub const MAX_TILES: usize = 96;

/// Tiles when nothing says otherwise: one every ten seconds of a four-minute
/// track, and a sheet of six rows that stays under a 2000 px long edge.
pub const DEFAULT_TILES: usize = 24;

/// A tile's long edge in pixels. Four landscape tiles and their gutters come to
/// just under 2000 px, which is where image readers start downscaling.
pub const TILE_LONG_EDGE: u32 = 480;

/// The widest a sheet is laid out, in pixels. Columns are derived from it.
pub const SHEET_MAX_WIDTH: u32 = 2000;

/// Space between tiles and around the sheet.
pub const GUTTER: u32 = 8;

/// Height of the two-line label strip under each tile.
pub const LABEL_HEIGHT: u32 = 48;

/// Which moments a sheet shows (`--sheet-at`).
#[derive(Clone, Debug, PartialEq)]
pub enum SheetStops {
    /// `even:N` — N tiles, each at the centre of one of N equal slices.
    Even(usize),
    /// `segments` — one tile per scene section, at its midpoint.
    Segments,
    /// `lyrics` — one tile per lyric line, at its midpoint.
    Lyrics,
    /// `T,T,…` — exactly these moments, in seconds.
    Times(Vec<f64>),
}

impl Default for SheetStops {
    fn default() -> Self {
        Self::Even(DEFAULT_TILES)
    }
}

impl SheetStops {
    /// Parse a `--sheet-at` value.
    ///
    /// `parse_time` is injected so the time grammar is the caller's one
    /// definition (`1:13.5` and `73.5` both), not a second copy that could
    /// come to disagree with the lyric editor's.
    ///
    /// # Errors
    ///
    /// Names what was wrong: an empty spec, a tile count outside
    /// `1..=`[`MAX_TILES`], or a list entry that is not a time.
    pub fn parse(spec: &str, parse_time: impl Fn(&str) -> Option<f64>) -> Result<Self, String> {
        let spec = spec.trim();
        match spec {
            "" => Err("--sheet-at needs even:N, segments, lyrics, or a list of times".into()),
            "segments" => Ok(Self::Segments),
            "lyrics" => Ok(Self::Lyrics),
            "even" => Ok(Self::default()),
            _ => {
                if let Some(count) = spec.strip_prefix("even:") {
                    return match count.parse::<usize>() {
                        Ok(count) if (1..=MAX_TILES).contains(&count) => Ok(Self::Even(count)),
                        _ => Err(format!(
                            "--sheet-at even:{count} wants a tile count of 1..={MAX_TILES}"
                        )),
                    };
                }
                let mut times = Vec::new();
                for entry in spec.split(',') {
                    let entry = entry.trim();
                    let Some(time) = parse_time(entry) else {
                        return Err(format!(
                            "--sheet-at entry {entry:?} is not a time; write seconds (73.5) \
                             or m:ss.fff (1:13.5), separated by commas — or even:N, segments, \
                             lyrics"
                        ));
                    };
                    times.push(time);
                }
                if times.len() > MAX_TILES {
                    return Err(format!(
                        "--sheet-at lists {} times; a sheet holds at most {MAX_TILES}",
                        times.len()
                    ));
                }
                Ok(Self::Times(times))
            }
        }
    }

    /// The moments to render, in seconds, ascending.
    ///
    /// `window` is `(start, length)` — the `--render-window` — and restricts
    /// every form to that span, so a sheet of a clip is a sheet *of the clip*.
    /// `segments` and `lyrics` are `(start, end)` spans from the track.
    ///
    /// # Errors
    ///
    /// An unusable duration or window, a listed time outside the span, a form
    /// with nothing to show (`segments` on a track with no plan), or more than
    /// [`MAX_TILES`] moments.
    pub fn resolve(
        &self,
        duration_seconds: f64,
        window: Option<(f64, f64)>,
        segments: &[(f64, f64)],
        lyrics: &[(f64, f64)],
    ) -> Result<Vec<f64>, String> {
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            return Err("the track has no usable duration".into());
        }
        let (start, end) = match window {
            None => (0.0, duration_seconds),
            Some((start, length)) => {
                let end = (start + length).min(duration_seconds);
                if !(start.is_finite() && length.is_finite()) || start < 0.0 || end <= start {
                    return Err(format!(
                        "the render window {start}+{length} s does not overlap the track"
                    ));
                }
                (start, end)
            }
        };
        let inside = |time: f64| time >= start && time < end;
        let midpoints = |spans: &[(f64, f64)]| -> Vec<f64> {
            spans
                .iter()
                // The midpoint of the part that is inside the window, so a
                // section the window cuts through is still represented.
                .filter(|(from, to)| *to > start && *from < end)
                .map(|(from, to)| (from.max(start) + to.min(end)) * 0.5)
                .collect()
        };
        let mut times = match self {
            Self::Even(count) => {
                let slice = (end - start) / *count as f64;
                (0..*count)
                    .map(|index| start + (index as f64 + 0.5) * slice)
                    .collect()
            }
            Self::Segments => {
                let times = midpoints(segments);
                if times.is_empty() {
                    return Err("--sheet-at segments: the track has no scene plan here".into());
                }
                times
            }
            Self::Lyrics => {
                let times = midpoints(lyrics);
                if times.is_empty() {
                    return Err("--sheet-at lyrics: no lyric line is on screen here".into());
                }
                times
            }
            Self::Times(times) => {
                if let Some(outside) = times.iter().find(|time| !inside(**time)) {
                    return Err(format!(
                        "--sheet-at time {outside} s is outside {start}..{end} s"
                    ));
                }
                times.clone()
            }
        };
        if times.is_empty() || times.len() > MAX_TILES {
            return Err(format!(
                "a sheet holds 1..={MAX_TILES} tiles, this one would have {}; \
                 narrow it with --render-window or list the times",
                times.len()
            ));
        }
        times.sort_by(f64::total_cmp);
        Ok(times)
    }
}

/// Where everything on a sheet goes. All in pixels, origin top-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SheetLayout {
    pub width: u32,
    pub height: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    pub columns: usize,
    pub rows: usize,
}

impl SheetLayout {
    /// Lay out `tiles` frames of `frame_width` x `frame_height`.
    ///
    /// The tile's **long** edge is fixed, not its width, so a 9:16 project gets
    /// tiles the same size as a 16:9 one and simply more of them per row. A
    /// fixed width would make portrait tiles 3.2 times the area and a sheet of
    /// twenty-four of them eight thousand pixels tall.
    ///
    /// `None` for no tiles or a degenerate frame.
    #[must_use]
    pub fn plan(tiles: usize, frame_width: u32, frame_height: u32) -> Option<Self> {
        if tiles == 0 || tiles > MAX_TILES || frame_width == 0 || frame_height == 0 {
            return None;
        }
        let long = frame_width.max(frame_height);
        let scale = f64::from(TILE_LONG_EDGE) / f64::from(long);
        let tile_width = ((f64::from(frame_width) * scale).round() as u32).max(1);
        let tile_height = ((f64::from(frame_height) * scale).round() as u32).max(1);
        let fit = ((SHEET_MAX_WIDTH - GUTTER) / (tile_width + GUTTER)).max(1) as usize;
        let columns = fit.min(tiles);
        let rows = tiles.div_ceil(columns);
        Some(Self {
            width: GUTTER + columns as u32 * (tile_width + GUTTER),
            height: GUTTER + rows as u32 * (tile_height + LABEL_HEIGHT + GUTTER),
            tile_width,
            tile_height,
            columns,
            rows,
        })
    }

    /// Top-left of tile `index`'s picture.
    #[must_use]
    pub fn tile_origin(&self, index: usize) -> (u32, u32) {
        let column = (index % self.columns) as u32;
        let row = (index / self.columns) as u32;
        (
            GUTTER + column * (self.tile_width + GUTTER),
            GUTTER + row * (self.tile_height + LABEL_HEIGHT + GUTTER),
        )
    }

    /// Top-left of tile `index`'s label strip, directly under its picture.
    #[must_use]
    pub fn label_origin(&self, index: usize) -> (u32, u32) {
        let (x, y) = self.tile_origin(index);
        (x, y + self.tile_height)
    }
}

/// Shrink a **bottom-up** RGBA frame into a **top-down** RGBA tile by area
/// average.
///
/// Bottom-up in, because that is what the offline target's readback is — the
/// same layout the encoder is fed, and the reason a still once shipped as a
/// vertical mirror (`write_resolved_png`). The flip happens here, once, in the
/// one place a test can see it.
///
/// A plain average of sRGB code values, deliberately *not* the linear-light
/// resolve the export uses (`render::resolve`): this makes a 480 px thumbnail
/// for judging composition, and the frame it shrinks has already been resolved
/// in linear light. Alpha is forced opaque — a sheet is a picture of the frame,
/// and a translucent tile over the sheet's ground would be a picture of both.
///
/// `None` when a buffer or a dimension does not match.
#[must_use]
pub fn shrink_bottom_up_rgba(
    source: &[u8],
    source_width: usize,
    source_height: usize,
    tile_width: usize,
    tile_height: usize,
) -> Option<Vec<u8>> {
    if source_width == 0
        || source_height == 0
        || tile_width == 0
        || tile_height == 0
        || source.len() != source_width * source_height * 4
    {
        return None;
    }
    let mut tile = vec![0u8; tile_width * tile_height * 4];
    for y in 0..tile_height {
        let y0 = y * source_height / tile_height;
        let y1 = ((y + 1) * source_height / tile_height).clamp(y0 + 1, source_height);
        for x in 0..tile_width {
            let x0 = x * source_width / tile_width;
            let x1 = ((x + 1) * source_width / tile_width).clamp(x0 + 1, source_width);
            let mut sum = [0u32; 3];
            for sy in y0..y1 {
                // Top-down tile row `sy` is bottom-up source row `h - 1 - sy`.
                let row = (source_height - 1 - sy) * source_width;
                for sx in x0..x1 {
                    let offset = (row + sx) * 4;
                    sum[0] += u32::from(source[offset]);
                    sum[1] += u32::from(source[offset + 1]);
                    sum[2] += u32::from(source[offset + 2]);
                }
            }
            let count = ((y1 - y0) * (x1 - x0)) as u32;
            let out = (y * tile_width + x) * 4;
            tile[out] = ((sum[0] + count / 2) / count) as u8;
            tile[out + 1] = ((sum[1] + count / 2) / count) as u8;
            tile[out + 2] = ((sum[2] + count / 2) / count) as u8;
            tile[out + 3] = 255;
        }
    }
    Some(tile)
}

/// Text a label can actually draw: the sheet's labels go through raylib's
/// built-in face, which is Latin-1 and stops. Anything past it becomes `?`
/// rather than an empty box that reads as a rendering fault, and control
/// characters become spaces so a lyric with a line break stays one line.
#[must_use]
pub fn label_text(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\u{20}'..='\u{7e}' | '\u{a1}'..='\u{ff}' => character,
            control if control.is_control() || control.is_whitespace() => ' ',
            _ => '?',
        })
        .collect()
}

/// Cut `text` to at most `max_chars`, ending in `...` when something was lost.
#[must_use]
pub fn truncate_label(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    if max_chars <= 3 {
        return text.chars().take(max_chars).collect();
    }
    let mut cut: String = text.chars().take(max_chars - 3).collect();
    cut.truncate(cut.trim_end().len());
    cut.push_str("...");
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seconds(text: &str) -> Option<f64> {
        text.parse().ok()
    }

    #[test]
    fn the_spec_has_four_forms_and_refuses_the_rest_by_name() {
        assert_eq!(
            SheetStops::parse("even:12", seconds),
            Ok(SheetStops::Even(12))
        );
        assert_eq!(
            SheetStops::parse("even", seconds),
            Ok(SheetStops::Even(DEFAULT_TILES))
        );
        assert_eq!(
            SheetStops::parse("segments", seconds),
            Ok(SheetStops::Segments)
        );
        assert_eq!(SheetStops::parse("lyrics", seconds), Ok(SheetStops::Lyrics));
        assert_eq!(
            SheetStops::parse("0, 12.5,73", seconds),
            Ok(SheetStops::Times(vec![0.0, 12.5, 73.0]))
        );
        assert!(SheetStops::parse("even:0", seconds)
            .unwrap_err()
            .contains("1..=96"));
        assert!(SheetStops::parse("even:97", seconds).is_err());
        assert!(SheetStops::parse("", seconds).is_err());
        let typo = SheetStops::parse("0,soon", seconds).unwrap_err();
        assert!(typo.contains("\"soon\""), "{typo}");
    }

    #[test]
    fn even_tiles_sit_at_the_centre_of_equal_slices() {
        let times = SheetStops::Even(4).resolve(240.0, None, &[], &[]).unwrap();
        assert_eq!(times, vec![30.0, 90.0, 150.0, 210.0]);
        // Never the first or the last instant: frame zero has heard no audio
        // and the last frame is usually silence.
        let dense = SheetStops::Even(96).resolve(240.0, None, &[], &[]).unwrap();
        assert!(dense[0] > 0.0 && *dense.last().unwrap() < 240.0);
    }

    #[test]
    fn a_window_restricts_every_form_to_itself() {
        let window = Some((60.0, 20.0));
        assert_eq!(
            SheetStops::Even(2)
                .resolve(240.0, window, &[], &[])
                .unwrap(),
            vec![65.0, 75.0]
        );
        // A section the window cuts through is shown at the middle of the part
        // inside it; sections outside it are not shown at all.
        let segments = [(0.0, 50.0), (50.0, 70.0), (70.0, 240.0)];
        assert_eq!(
            SheetStops::Segments
                .resolve(240.0, window, &segments, &[])
                .unwrap(),
            vec![65.0, 75.0]
        );
        let outside = SheetStops::Times(vec![10.0])
            .resolve(240.0, window, &[], &[])
            .unwrap_err();
        assert!(outside.contains("outside 60..80"), "{outside}");
        // A window that runs past the end is clipped, not refused.
        assert_eq!(
            SheetStops::Even(1)
                .resolve(240.0, Some((230.0, 60.0)), &[], &[])
                .unwrap(),
            vec![235.0]
        );
    }

    #[test]
    fn a_form_with_nothing_to_show_says_so() {
        assert!(SheetStops::Segments
            .resolve(240.0, None, &[], &[])
            .unwrap_err()
            .contains("no scene plan"));
        assert!(SheetStops::Lyrics
            .resolve(240.0, None, &[], &[])
            .unwrap_err()
            .contains("no lyric line"));
        let lyrics = [(23.0, 27.0), (27.0, 32.0)];
        assert_eq!(
            SheetStops::Lyrics
                .resolve(240.0, None, &[], &lyrics)
                .unwrap(),
            vec![25.0, 29.5]
        );
        // Listed times come back ascending whatever order they were typed in.
        assert_eq!(
            SheetStops::Times(vec![73.0, 5.0])
                .resolve(240.0, None, &[], &[])
                .unwrap(),
            vec![5.0, 73.0]
        );
    }

    #[test]
    fn landscape_lays_out_four_across_under_two_thousand_pixels() {
        let layout = SheetLayout::plan(24, 1920, 1080).unwrap();
        assert_eq!((layout.tile_width, layout.tile_height), (480, 270));
        assert_eq!((layout.columns, layout.rows), (4, 6));
        assert_eq!(layout.width, 8 + 4 * 488);
        assert_eq!(layout.height, 8 + 6 * (270 + 48 + 8));
        assert!(layout.width <= SHEET_MAX_WIDTH);
        assert_eq!(layout.tile_origin(0), (8, 8));
        assert_eq!(layout.tile_origin(5), (8 + 488, 8 + 326));
        assert_eq!(layout.label_origin(5), (8 + 488, 8 + 326 + 270));
    }

    #[test]
    fn the_long_edge_is_fixed_so_portrait_gets_more_columns_not_bigger_tiles() {
        let portrait = SheetLayout::plan(24, 1080, 1920).unwrap();
        assert_eq!((portrait.tile_width, portrait.tile_height), (270, 480));
        assert_eq!(portrait.columns, 7);
        assert!(portrait.width <= SHEET_MAX_WIDTH);
        let square = SheetLayout::plan(9, 1080, 1080).unwrap();
        assert_eq!((square.tile_width, square.tile_height), (480, 480));
        // Fewer tiles than a row holds: the sheet shrinks to fit them.
        let few = SheetLayout::plan(2, 1920, 1080).unwrap();
        assert_eq!((few.columns, few.rows), (2, 1));
        assert!(SheetLayout::plan(0, 1920, 1080).is_none());
        assert!(SheetLayout::plan(MAX_TILES + 1, 1920, 1080).is_none());
    }

    #[test]
    fn every_tile_and_label_lies_inside_the_sheet_and_none_overlap() {
        for (tiles, width, height) in [(24, 1920, 1080), (7, 1080, 1920), (96, 1280, 720)] {
            let layout = SheetLayout::plan(tiles, width, height).unwrap();
            let mut seen = Vec::new();
            for index in 0..tiles {
                let (x, y) = layout.tile_origin(index);
                assert!(x + layout.tile_width + GUTTER <= layout.width);
                assert!(y + layout.tile_height + LABEL_HEIGHT + GUTTER <= layout.height);
                assert!(!seen.contains(&(x, y)), "tile {index} lands on another");
                seen.push((x, y));
            }
        }
    }

    #[test]
    fn shrinking_averages_blocks_and_puts_the_bottom_row_last() {
        // A 4x2 bottom-up frame: the *first* row in memory is the bottom of the
        // picture. Bottom row red, top row blue.
        let red = [200u8, 0, 0, 255];
        let blue = [0u8, 0, 100, 255];
        let mut source = Vec::new();
        for _ in 0..4 {
            source.extend_from_slice(&red);
        }
        for _ in 0..4 {
            source.extend_from_slice(&blue);
        }
        let tile = shrink_bottom_up_rgba(&source, 4, 2, 2, 2).unwrap();
        // Top-down out: blue first, red last. The mirrored still would have
        // passed a "two runs agree" check; this cannot.
        assert_eq!(&tile[0..4], &blue);
        assert_eq!(&tile[4..8], &blue);
        assert_eq!(&tile[8..12], &red);
        assert_eq!(&tile[12..16], &red);

        // A 2x1 pair of unlike pixels averages to their mean, rounded.
        let pair = [10u8, 20, 30, 0, 21, 41, 61, 0];
        assert_eq!(
            shrink_bottom_up_rgba(&pair, 2, 1, 1, 1).unwrap(),
            // (10+21)/2 = 15.5 -> 16, (20+41)/2 = 30.5 -> 31, (30+61)/2 = 45.5 -> 46;
            // alpha is forced opaque whatever came in.
            vec![16, 31, 46, 255]
        );
    }

    #[test]
    fn shrinking_refuses_a_buffer_that_is_not_the_frame_it_claims() {
        assert!(shrink_bottom_up_rgba(&[0; 15], 2, 2, 1, 1).is_none());
        assert!(shrink_bottom_up_rgba(&[0; 16], 2, 2, 0, 1).is_none());
        assert!(shrink_bottom_up_rgba(&[], 0, 0, 1, 1).is_none());
        // Enlarging is not this function's job, but it must not read past the
        // end: every destination pixel samples at least one source pixel.
        let one = [9u8, 8, 7, 255];
        let grown = shrink_bottom_up_rgba(&one, 1, 1, 3, 2).unwrap();
        assert_eq!(grown.len(), 3 * 2 * 4);
        assert!(grown.chunks_exact(4).all(|pixel| pixel == [9, 8, 7, 255]));
    }

    #[test]
    fn labels_stay_inside_the_built_in_face() {
        assert_eq!(label_text("haul away"), "haul away");
        assert_eq!(label_text("line\none\ttwo"), "line one two");
        assert_eq!(label_text("café"), "café");
        assert_eq!(label_text("夜 night ★"), "? night ?");
        assert_eq!(truncate_label("haul away", 20), "haul away");
        assert_eq!(truncate_label("the clock stopped dead", 12), "the clock...");
        assert_eq!(truncate_label("abcdef", 3), "abc");
        // Counts characters, not bytes, so a Latin-1 line is not cut early.
        assert_eq!(truncate_label("ééééé", 5), "ééééé");
    }
}
