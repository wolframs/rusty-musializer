//! The in-app protocol runner (HX-2): markers, the question card, and the
//! session state machine.
//!
//! **Post-legacy product extension.** The state machine is pure and tested; the
//! two drawing functions are only pixels, matching the shell's split. `main.rs`
//! owns every side effect an answer causes — applying a snapshot, seeking,
//! appending the JSONL line — because those need the audio device and the
//! filesystem, and this module may hold neither.
//!
//! # Blinding
//!
//! Nothing this module draws ever names variant `a` or `b`. An A/B item's two
//! tunings are presented as "first look" and "second look" — the order the
//! session actually played them, which [`ProtocolSession::order`] records and
//! the answers file preserves for the agent that wrote the protocol. Which
//! label maps to which look exists only in the file, never on screen.

use std::path::PathBuf;

use musializer_core::feedback::{AnswerKind, AnswerRecord, Protocol, ProtocolItem, Variant};
use musializer_core::scene::settings::SettingsSnapshot;
use musializer_core::scene::SceneId;
use musializer_core::ui::timeline_view::TimelineView;
use musializer_core::ui::tune_explore::{RandomSource, SplitMix64};
use musializer_core::ui::workspace_layout::UiRect;
use musializer_runtime::font::UiFonts;
use raylib::prelude::{RaylibDraw, RaylibDrawHandle, Vector2};

use super::scale::UiScale;
use super::theme::{color, metric};
use super::widgets;

/// What the runner asks `main.rs` to do after a state transition: put this
/// tuning on screen and audition this window.
///
/// A value rather than a mutation for the shell's usual reason — the session
/// cannot reach the settings store, the analyzer or the stream, and returning
/// the effect makes every transition assertable without a window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Activation {
    /// The scene and tuning to apply before the window plays. `None` for an
    /// item with no `apply` block, which asks about the track as it is.
    pub apply: Option<(SceneId, SettingsSnapshot)>,
    /// Where the audition starts: `at_seconds - window.pre`, floored at zero.
    pub seek_to: f64,
    /// Where playback pauses itself: `at_seconds + window.post`.
    pub stop_at: f64,
}

/// One loaded protocol mid-session.
#[derive(Clone, Debug)]
pub struct ProtocolSession {
    pub protocol: Protocol,
    /// The protocol file's name, for the card and the report line.
    pub file_name: String,
    /// Where answers append: `<protocol stem>.answers.jsonl` beside the file.
    pub answers_path: PathBuf,
    /// Item ids with at least one recorded answer (from disk at load, then
    /// maintained as answers land).
    answered: Vec<String>,
    /// Index of the item on screen, `None` once every item is answered.
    pub current: Option<usize>,
    /// Which snapshot is applied right now.
    pub live: Variant,
    /// Every variant put on screen for the current item, in play order. This
    /// is the unblinding record; it goes into the answer line verbatim.
    pub order: Vec<Variant>,
    /// How many times the current item's window has been auditioned.
    pub auditions: u32,
    /// The last answer written this session, for the `protocol:` report line —
    /// recorded rather than recomputed, so the line states what was appended.
    pub last_answer: Option<AnswerRecord>,
    /// Where the running audition pauses itself: `at_seconds + window.post`.
    /// Set by every activation, cleared by the composition root when it fires.
    pub stop_at: Option<f64>,
}

impl ProtocolSession {
    /// Build a session over a parsed protocol and whatever answers already
    /// exist on disk. `answered` seeds from the log so a quit-and-relaunch
    /// resumes at the first unanswered item.
    #[must_use]
    pub fn new(
        protocol: Protocol,
        file_name: String,
        answers_path: PathBuf,
        already_answered: Vec<String>,
    ) -> Self {
        Self {
            protocol,
            file_name,
            answers_path,
            answered: already_answered,
            current: None,
            live: Variant::A,
            order: Vec::new(),
            auditions: 0,
            last_answer: None,
            stop_at: None,
        }
    }

    #[must_use]
    pub fn item(&self) -> Option<&ProtocolItem> {
        self.protocol.items.get(self.current?)
    }

    #[must_use]
    pub fn total(&self) -> usize {
        self.protocol.items.len()
    }

    #[must_use]
    pub fn answered_count(&self) -> usize {
        self.protocol
            .items
            .iter()
            .filter(|item| self.answered.contains(&item.id))
            .count()
    }

    #[must_use]
    pub fn is_answered(&self, id: &str) -> bool {
        self.answered.iter().any(|answered| answered == id)
    }

    /// The first unanswered item at or after `from`, wrapping — where a fresh
    /// session starts and where an answer advances to.
    #[must_use]
    pub fn next_unanswered(&self, from: usize) -> Option<usize> {
        let total = self.protocol.items.len();
        (0..total)
            .map(|offset| (from + offset) % total)
            .find(|&index| !self.is_answered(&self.protocol.items[index].id))
    }

    /// Which snapshot an A/B item shows first.
    ///
    /// Derived from the item's seed and id rather than always `a`, so "first
    /// look" does not become a synonym for `a` across a whole session — that
    /// would quietly unblind an operator who noticed. Deterministic, so a
    /// probe run pins it.
    fn first_variant(item: &ProtocolItem) -> Variant {
        let Some(apply) = &item.apply else {
            return Variant::A;
        };
        if !apply.is_ab() {
            return Variant::A;
        }
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in item.id.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let mut rng = SplitMix64::new(apply.seed.unwrap_or(0) ^ hash);
        if rng.next_u64() & 1 == 0 {
            Variant::A
        } else {
            Variant::B
        }
    }

    fn activation(&self, item: &ProtocolItem, variant: Variant) -> Activation {
        Activation {
            apply: item
                .apply
                .as_ref()
                .and_then(|apply| Some((apply.scene, apply.snapshot(variant)?))),
            seek_to: (item.at_seconds - item.window.pre).max(0.0),
            stop_at: item.at_seconds + item.window.post,
        }
    }

    /// Make `index` the current item and start its first audition.
    pub fn activate(&mut self, index: usize) -> Option<Activation> {
        let item = self.protocol.items.get(index)?;
        let variant = Self::first_variant(item);
        let activation = self.activation(item, variant);
        self.current = Some(index);
        self.live = variant;
        self.order = if item.apply.is_some() {
            vec![variant]
        } else {
            Vec::new()
        };
        self.auditions = 1;
        self.stop_at = Some(activation.stop_at);
        Some(activation)
    }

    /// Make the item named `id` current (the probe path — by id, never by
    /// pixel, which is GX-1's whole argument).
    pub fn activate_id(&mut self, id: &str) -> Option<Activation> {
        let index = self.protocol.items.iter().position(|item| item.id == id)?;
        self.activate(index)
    }

    /// Play the current window again.
    pub fn replay(&mut self) -> Option<Activation> {
        let item = self.protocol.items.get(self.current?)?;
        let activation = self.activation(item, self.live);
        self.auditions = self.auditions.saturating_add(1);
        self.stop_at = Some(activation.stop_at);
        Some(activation)
    }

    /// Swap to the other look and re-audition. `None` when the current item
    /// has fewer than two snapshots — the card offers no flip there.
    pub fn flip(&mut self) -> Option<Activation> {
        let item = self.protocol.items.get(self.current?)?;
        let apply = item.apply.as_ref()?;
        if !apply.is_ab() {
            return None;
        }
        let other = self.live.other();
        let activation = self.activation(item, other);
        self.live = other;
        self.order.push(other);
        self.auditions = self.auditions.saturating_add(1);
        self.stop_at = Some(activation.stop_at);
        Some(activation)
    }

    /// Record a `1`-`4` answer for the current item.
    ///
    /// Returns the record for `main.rs` to stamp (wall clock, playhead) and
    /// append — the session is only marked answered once the caller has it,
    /// so a failed append can be retried without lying about progress.
    pub fn answer_choice(&mut self, choice: u8) -> Option<AnswerRecord> {
        let item = self.protocol.items.get(self.current?)?;
        if item.kind == AnswerKind::Text {
            return None;
        }
        let index = usize::from(choice.checked_sub(1)?);
        let option = item.options.get(index)?;
        let mut record = AnswerRecord::new(&item.id, option);
        record.choice = Some(choice);
        record.variant_order = self.order.clone();
        record.auditions = self.auditions;
        Some(record)
    }

    /// The caller appended `record` successfully; count its item as answered.
    pub fn mark_answered(&mut self, record: AnswerRecord) {
        if !self.is_answered(&record.item_id) {
            self.answered.push(record.item_id.clone());
        }
        self.last_answer = Some(record);
    }

    /// Where the current look sits in the flip sequence, for the card: "first
    /// look" / "second look". Never `a`/`b`.
    #[must_use]
    pub fn look_label(&self) -> Option<&'static str> {
        let first = *self.order.first()?;
        Some(if self.live == first {
            "first look"
        } else {
            "second look"
        })
    }

    /// The `protocol:` report line's payload.
    ///
    /// `last=` states what was actually appended this session — id, the exact
    /// variant order, and the chosen key — which is what the gate compares
    /// against the JSONL it reads back.
    #[must_use]
    pub fn describe(&self) -> String {
        let current = match self
            .current
            .and_then(|index| self.protocol.items.get(index))
        {
            Some(item) => item.id.as_str(),
            None => "none",
        };
        let look = self.look_label().unwrap_or("-");
        let last = match &self.last_answer {
            None => "none".to_string(),
            Some(record) => format!(
                "{}:{}:{}",
                record.item_id,
                order_token(&record.variant_order),
                record
                    .choice
                    .map_or_else(|| "text".to_string(), |c| c.to_string()),
            ),
        };
        format!(
            "{} items={} answered={} current={current} look={look} auditions={} last={last}",
            self.file_name,
            self.total(),
            self.answered_count(),
            self.auditions,
        )
    }
}

/// A variant order as a compact token: `[B, A, B]` -> `b,a,b`; empty -> `-`.
#[must_use]
pub fn order_token(order: &[Variant]) -> String {
    if order.is_empty() {
        return "-".to_string();
    }
    order
        .iter()
        .map(|variant| variant.token())
        .collect::<Vec<_>>()
        .join(",")
}

// -- drawing -------------------------------------------------------------------

/// Protocol markers on the waveform strip: bottom-anchored, so they cannot be
/// confused with the top-anchored event lollipops — position is the channel,
/// the same argument CX-1 made for proposals. Answered items dim; the current
/// item carries a full-height line.
pub(crate) fn draw_markers(
    d: &mut RaylibDrawHandle<'_>,
    session: &ProtocolSession,
    view: &TimelineView,
    strip: UiRect,
    duration: f64,
    ui_scale: UiScale,
) {
    let mut clip = widgets::begin_scissor(d, strip, ui_scale);
    for (index, item) in session.protocol.items.iter().enumerate() {
        if item.at_seconds < 0.0 || item.at_seconds > duration {
            continue;
        }
        let x = view.x_at(item.at_seconds, f64::from(strip.x), f64::from(strip.width)) as f32;
        if x < strip.x - 8.0 || x > strip.x + strip.width + 8.0 {
            continue;
        }
        let is_current = session.current == Some(index);
        let answered = session.is_answered(&item.id);
        let base = color::notice_info_on_dark();
        let tint = if is_current {
            base
        } else if answered {
            widgets::alpha(base, 0.35)
        } else {
            widgets::alpha(base, 0.8)
        };
        let bottom = strip.y + strip.height;
        if is_current {
            clip.draw_line_ex(
                Vector2::new(x, strip.y),
                Vector2::new(x, bottom),
                2.0,
                widgets::alpha(base, 0.65),
            );
        }
        // An upward-pointing pennant off the lane's bottom edge.
        clip.draw_triangle(
            Vector2::new(x, bottom - MARKER_HEIGHT),
            Vector2::new(x - MARKER_HALF_WIDTH, bottom),
            Vector2::new(x + MARKER_HALF_WIDTH, bottom),
            tint,
        );
    }
}

const MARKER_HEIGHT: f32 = 9.0;
const MARKER_HALF_WIDTH: f32 = 5.0;

/// The question card, centred over the preview's bottom edge.
///
/// Keyboard-first on purpose: the operator is listening, and HX-2's loop is
/// "press play, listen, press a number". The card names every key it takes.
/// No widget claims a pointer here, so it cannot steal a press from the
/// preview underneath it.
pub(crate) fn draw_card(
    d: &mut RaylibDrawHandle<'_>,
    font: &UiFonts,
    session: &ProtocolSession,
    preview: UiRect,
) {
    if preview.is_empty() {
        return;
    }
    let width = CARD_WIDTH.min(preview.width - 24.0);
    if width <= 120.0 {
        return;
    }
    let text_width = width - CARD_PADDING * 2.0;

    let (title, question, rows, hint): (String, String, Vec<String>, String) = match session.item()
    {
        None => (
            format!(
                "Listening session complete \u{00b7} {} of {} answered",
                session.answered_count(),
                session.total()
            ),
            format!("Answers are in {}", session.answers_path.display()),
            Vec::new(),
            "Every answer is already on disk \u{2014} quitting loses nothing".to_string(),
        ),
        Some(item) => {
            let progress = format!(
                "Question {} of {} \u{00b7} {} answered",
                session.current.unwrap_or(0) + 1,
                session.total(),
                session.answered_count(),
            );
            let rows = item
                .options
                .iter()
                .enumerate()
                .map(|(index, option)| format!("{} \u{00b7} {option}", index + 1))
                .collect::<Vec<_>>();
            let has_flip = item.apply.as_ref().is_some_and(|apply| apply.is_ab());
            let mut hint = String::from("R replay \u{00b7} Space pause");
            if has_flip {
                let look = session.look_label().unwrap_or("first look");
                hint = format!("B other look (now: {look}) \u{00b7} {hint}");
            }
            if item.kind == AnswerKind::Text {
                hint = format!("typed answers are not built yet \u{2014} N skips \u{00b7} {hint}");
            } else {
                hint = format!("1-{} answer \u{00b7} {hint} \u{00b7} N skip", rows.len());
            }
            (progress, item.question.clone(), rows, hint)
        }
    };

    let measure = |text: &str, size: f32| widgets::measure(font, text, size);
    let question_lines =
        musializer_core::ui::notice::wrap_detail(&question, text_width, 3, |text| {
            measure(text, metric::UI_FONT_LABEL)
        });

    let mut height = CARD_PADDING + metric::UI_FONT_CAPTION + 6.0;
    height += question_lines.len() as f32 * (metric::UI_FONT_LABEL + 4.0) + 4.0;
    if !rows.is_empty() {
        height += rows.len() as f32 * ROW_HEIGHT + 4.0;
    }
    height += metric::UI_FONT_CAPTION + CARD_PADDING;

    let x = preview.x + (preview.width - width) * 0.5;
    let y = preview.y + preview.height - height - 12.0;
    if y < preview.y {
        return;
    }
    let card = UiRect::new(x, y, width, height);
    widgets::fill(d, card, color::ui_overlay_surface());
    widgets::fill(
        d,
        UiRect::new(card.x, card.y, 3.0, card.height),
        color::notice_info_on_dark(),
    );

    let mut cursor = card.y + CARD_PADDING;
    widgets::draw_text(
        d,
        font,
        &title,
        card.x + CARD_PADDING,
        cursor,
        metric::UI_FONT_CAPTION,
        color::notice_info_on_dark(),
    );
    cursor += metric::UI_FONT_CAPTION + 6.0;
    for line in &question_lines {
        widgets::draw_text(
            d,
            font,
            line,
            card.x + CARD_PADDING,
            cursor,
            metric::UI_FONT_LABEL,
            color::ui_overlay_ink(),
        );
        cursor += metric::UI_FONT_LABEL + 4.0;
    }
    cursor += 4.0;
    for row in &rows {
        widgets::draw_text(
            d,
            font,
            row,
            card.x + CARD_PADDING + 8.0,
            cursor,
            metric::UI_FONT_VALUE,
            color::ui_overlay_ink(),
        );
        cursor += ROW_HEIGHT;
    }
    if !rows.is_empty() {
        cursor += 4.0;
    }
    widgets::draw_text(
        d,
        font,
        &hint,
        card.x + CARD_PADDING,
        cursor,
        metric::UI_FONT_CAPTION,
        color::ui_overlay_muted(),
    );
}

const CARD_WIDTH: f32 = 520.0;
const CARD_PADDING: f32 = 12.0;
const ROW_HEIGHT: f32 = 22.0;

#[cfg(test)]
mod tests {
    use super::*;
    use musializer_core::feedback::{AnswerKind, Apply, Window};
    use musializer_core::project::sha256;
    use musializer_core::scene::settings::SceneSettings;

    fn ab_protocol() -> Protocol {
        let a = SceneSettings::default()
            .capture(SceneId::SongAtlas)
            .unwrap();
        let mut b = a;
        b.values[0] = 2.0;
        Protocol {
            title: "test".to_string(),
            audio_path: "x.wav".to_string(),
            audio_sha256: sha256::digest_hex(b"x"),
            items: vec![
                ProtocolItem {
                    id: "one".to_string(),
                    at_seconds: 10.0,
                    window: Window {
                        pre: 2.0,
                        post: 6.0,
                    },
                    question: "?".to_string(),
                    kind: AnswerKind::Choice,
                    options: vec!["keep".into(), "fixable".into(), "reject".into()],
                    apply: Some(Apply {
                        scene: SceneId::SongAtlas,
                        seed: Some(7),
                        a,
                        b: Some(b),
                    }),
                },
                ProtocolItem {
                    id: "two".to_string(),
                    at_seconds: 20.0,
                    window: Window {
                        pre: 1.0,
                        post: 4.0,
                    },
                    question: "?".to_string(),
                    kind: AnswerKind::Choice,
                    options: vec!["yes".into(), "no".into()],
                    apply: None,
                },
            ],
        }
    }

    fn session() -> ProtocolSession {
        ProtocolSession::new(
            ab_protocol(),
            "test.protocol.json".to_string(),
            PathBuf::from("test.answers.jsonl"),
            Vec::new(),
        )
    }

    #[test]
    fn activation_covers_the_window_from_pre_seconds_before() {
        let mut session = session();
        let activation = session.activate(0).unwrap();
        assert_eq!(activation.seek_to, 8.0);
        assert_eq!(activation.stop_at, 16.0);
        assert!(activation.apply.is_some());
        assert_eq!(session.auditions, 1);
        assert_eq!(session.order.len(), 1);
    }

    #[test]
    fn a_flip_alternates_and_records_the_order_verbatim() {
        let mut session = session();
        session.activate(0).unwrap();
        let first = session.live;
        let flip = session.flip().unwrap();
        assert_eq!(session.live, first.other());
        assert_eq!(session.order, vec![first, first.other()]);
        // The flip's activation applies the *other* snapshot.
        let (_, snapshot) = flip.apply.unwrap();
        let item = ab_protocol();
        let expected = item.items[0]
            .apply
            .unwrap()
            .snapshot(first.other())
            .unwrap();
        assert_eq!(snapshot, expected);

        session.flip().unwrap();
        assert_eq!(session.order, vec![first, first.other(), first]);
        assert_eq!(session.auditions, 3);
    }

    #[test]
    fn a_single_snapshot_item_offers_no_flip() {
        let mut session = session();
        session.activate(1).unwrap();
        assert!(session.flip().is_none());
        assert!(session.order.is_empty(), "no apply block, no variant order");
    }

    #[test]
    fn the_first_variant_is_seeded_not_always_a() {
        // Sweep ids: both variants must occur as the opener somewhere, or
        // "first look" is a synonym for `a` and the blind is soft.
        let protocol = ab_protocol();
        let item = &protocol.items[0];
        let mut saw = [false, false];
        for n in 0..32 {
            let mut item = item.clone();
            item.id = format!("probe-{n}");
            match ProtocolSession::first_variant(&item) {
                Variant::A => saw[0] = true,
                Variant::B => saw[1] = true,
            }
        }
        assert!(saw[0] && saw[1]);
        // And it is deterministic.
        assert_eq!(
            ProtocolSession::first_variant(item),
            ProtocolSession::first_variant(item)
        );
    }

    #[test]
    fn answering_carries_the_order_and_the_option_label() {
        let mut session = session();
        session.activate(0).unwrap();
        session.flip().unwrap();
        let record = session.answer_choice(2).unwrap();
        assert_eq!(record.item_id, "one");
        assert_eq!(record.answer, "fixable");
        assert_eq!(record.choice, Some(2));
        assert_eq!(record.variant_order.len(), 2);
        assert_eq!(record.auditions, 2);

        // Out-of-range keys do nothing rather than guess.
        assert!(session.answer_choice(0).is_none());
        assert!(session.answer_choice(4).is_none());
    }

    #[test]
    fn progress_resumes_from_the_answers_already_on_disk() {
        let mut session = ProtocolSession::new(
            ab_protocol(),
            "test.protocol.json".to_string(),
            PathBuf::from("x"),
            vec!["one".to_string()],
        );
        assert_eq!(session.answered_count(), 1);
        assert_eq!(session.next_unanswered(0), Some(1));
        session.activate(1).unwrap();
        let record = session.answer_choice(1).unwrap();
        session.mark_answered(record);
        assert_eq!(session.answered_count(), 2);
        assert_eq!(session.next_unanswered(0), None);
    }

    #[test]
    fn the_describe_line_names_the_last_answer_exactly() {
        let mut session = session();
        session.activate(0).unwrap();
        session.flip().unwrap();
        let record = session.answer_choice(3).unwrap();
        let expected_order = order_token(&record.variant_order);
        session.mark_answered(record);
        let line = session.describe();
        assert!(line.starts_with("test.protocol.json items=2 answered=1"));
        assert!(
            line.ends_with(&format!("last=one:{expected_order}:3")),
            "{line}"
        );
    }
}
