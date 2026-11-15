//! What a semantic listening call should cost, said before it is sent.
//!
//! The 2026-11-14 MiMo benchmark (`docs/MIMO_BENCHMARK_RESULTS.md`, "Cost")
//! found the money is in the **answer**, not the audio: a reasoning model
//! thinks before it answers, billed at the completion rate, while audio bills
//! at the prompt rate. An estimate built on audio seconds alone is wrong by an
//! order of magnitude, so this one is built on completion tokens as well: a
//! range from the span of answer lengths, and a ceiling from the completion cap
//! the helper sends, which is the most one call can cost.
//!
//! What differs per model (prices, the cap, the token model) is one row of
//! [`MODELS`]. A model with no row gets [`DEFAULT_MAX_COMPLETION_TOKENS`] and
//! no dollar figure, never a guessed one.
//!
//! Every number here has a twin in `tools/semantic_openrouter.py`
//! (`MODEL_PROFILES`), which records the same estimate beside what OpenRouter
//! reported. Both copies are pinned against each other by
//! `tests/test_cross_language_pins.py`; change both or neither.

/// The cap the helper sends for a model with no row (and for MiMo), reasoning
/// included (`DEFAULT_MAX_COMPLETION_TOKENS`).
pub const DEFAULT_MAX_COMPLETION_TOKENS: u32 = 16_000;

/// One model's prices and token model. Prices are USD per million tokens as
/// OpenRouter listed them on `prices_listed_on`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelCost {
    pub model_id: &'static str,
    pub prompt_usd_per_million: f64,
    pub audio_usd_per_million: f64,
    pub completion_usd_per_million: f64,
    pub prices_listed_on: &'static str,
    /// `max_tokens`, reasoning included.
    pub max_completion_tokens: u32,
    pub audio_tokens_per_second: f64,
    /// The request's text (prompt, schema) as prompt tokens.
    pub prompt_text_tokens: u32,
    pub typical_completion_tokens: [u32; 2],
}

/// A dated table, not a live quote.
///
/// - `xiaomi/mimo-v2.5` (2026-11-14, Venice, its only provider that day; no
///   audio price listed, so audio bills as prompt): audio at 7.4 tokens per
///   second and ~200 tokens of text from the benchmark's recorded usage;
///   answers of 2 044 and 2 407 tokens on this lane's request, 10 942 the
///   largest legitimate one.
/// - `google/gemini-3.8-flash` (2026-11-15, the highest price of the endpoints
///   a base slug routes to, Vertex us): Google's 32 audio tokens per second
///   (Vertex counted 25 on this request), 1 650 tokens of text, answers of
///   1 915–2 231 tokens on 100 s and 5 030 on 300 s. `docs/ASSIST_PIPELINE.md`,
///   "Gemini 3.8 Flash through OpenRouter".
#[rustfmt::skip]
pub const MODELS: &[ModelCost] = &[
    ModelCost { model_id: "xiaomi/mimo-v2.5", prompt_usd_per_million: 0.40, audio_usd_per_million: 0.40, completion_usd_per_million: 2.00, prices_listed_on: "2026-11-14", max_completion_tokens: 16_000, audio_tokens_per_second: 7.4, prompt_text_tokens: 200, typical_completion_tokens: [1_500, 11_000] },
    ModelCost { model_id: "google/gemini-3.8-flash", prompt_usd_per_million: 0.825, audio_usd_per_million: 0.825, completion_usd_per_million: 4.125, prices_listed_on: "2026-11-15", max_completion_tokens: 16_000, audio_tokens_per_second: 32.0, prompt_text_tokens: 1_650, typical_completion_tokens: [1_500, 8_000] },
];

/// One call's estimate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CostEstimate {
    pub input_tokens: u32,
    /// From the row's `typical_completion_tokens`.
    pub usd_typical: [f64; 2],
    /// A call that runs to the row's `max_completion_tokens`.
    pub usd_max: f64,
    pub max_completion_tokens: u32,
}

impl CostEstimate {
    /// The line the confirmation shows. The range is "about", rounded to the
    /// nearest thousandth; the maximum is a bound, so it is rounded **up**
    /// ([`ceil_thousandths`]): a capped MiMo call on 100 s costs 0.032376 USD
    /// and reads "at most 0.033 USD", never 0.032.
    ///
    /// Short enough for the column at its default width (2026-11-15: the
    /// earlier "Estimated cost: …, at most … (answer capped at 16000 tokens)"
    /// was cut off with an ellipsis there); the cap is said once, as a number.
    #[must_use]
    pub fn sentence(&self) -> String {
        format!(
            "Cost: about {:.3}\u{2013}{:.3} USD, at most {:.3} USD ({}-token cap)",
            self.usd_typical[0],
            self.usd_typical[1],
            ceil_thousandths(self.usd_max),
            self.max_completion_tokens,
        )
    }
}

/// Dollars rounded up to the thousandth a maximum is shown at, tolerating
/// float noise below a millionth of a thousandth (0.033 computed as
/// 0.0330000000001 stays 0.033). The helper's `ceil_usd(value, 3)` in
/// `tools/semantic_openrouter.py` is the same rule; both are pinned at 100 s.
#[must_use]
pub fn ceil_thousandths(usd: f64) -> f64 {
    ((usd * 1000.0) - 1.0e-6).ceil().max(0.0) / 1000.0
}

#[must_use]
pub fn model(model_id: &str) -> Option<ModelCost> {
    MODELS.iter().copied().find(|row| row.model_id == model_id)
}

/// The completion cap the helper sends for this model.
#[must_use]
pub fn max_completion_tokens(model_id: &str) -> u32 {
    model(model_id).map_or(DEFAULT_MAX_COMPLETION_TOKENS, |row| {
        row.max_completion_tokens
    })
}

/// Audio tokens for `duration_seconds` at the row's rate, rounded up as the
/// helper rounds them.
#[must_use]
pub fn audio_tokens(row: &ModelCost, duration_seconds: f64) -> u32 {
    let audio = (duration_seconds.max(0.0) * row.audio_tokens_per_second).ceil();
    // A track is minutes long; the cast cannot truncate anything real, and a
    // non-finite duration never reaches it (see `semantic_estimate`).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let audio = audio.min(f64::from(u32::MAX / 2)) as u32;
    audio
}

/// The estimate for one semantic call, or `None` when the model has no listed
/// price or the duration is not a duration.
#[must_use]
pub fn semantic_estimate(model_id: &str, duration_seconds: f64) -> Option<CostEstimate> {
    if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return None;
    }
    let row = model(model_id)?;
    let audio = audio_tokens(&row, duration_seconds);
    let usd = |completion: u32| {
        (f64::from(audio) * row.audio_usd_per_million
            + f64::from(row.prompt_text_tokens) * row.prompt_usd_per_million
            + f64::from(completion) * row.completion_usd_per_million)
            / 1_000_000.0
    };
    Some(CostEstimate {
        input_tokens: audio.saturating_add(row.prompt_text_tokens),
        usd_typical: [
            usd(row.typical_completion_tokens[0]),
            usd(row.typical_completion_tokens[1]),
        ],
        usd_max: usd(row.max_completion_tokens),
        max_completion_tokens: row.max_completion_tokens,
    })
}

/// What the confirmation says for a model with no listed price: the cap still
/// bounds the answer, and the user is told there is no figure rather than shown
/// a guessed one.
#[must_use]
pub fn unpriced_sentence(model_id: &str) -> String {
    format!(
        "No price listed here for {model_id}; {}-token cap",
        max_completion_tokens(model_id)
    )
}

/// The confirmation's cost line for a semantic route: the estimate, or why
/// there is no figure. Every variant names the cap, which holds either way.
#[must_use]
pub fn semantic_line(model_id: &str, duration_seconds: f64) -> String {
    match (
        model(model_id),
        semantic_estimate(model_id, duration_seconds),
    ) {
        (None, _) => unpriced_sentence(model_id),
        (Some(row), None) => format!(
            "Cost: needs the track's length; {}-token cap",
            row.max_completion_tokens
        ),
        (Some(_), Some(estimate)) => estimate.sentence(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIMO: &str = "xiaomi/mimo-v2.5";
    const GEMINI: &str = "google/gemini-3.8-flash";

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    /// Pinned numbers, not properties: 200 s is 1 480 audio tokens plus 200 of
    /// text, so 1 680 at 0.40 USD/M and 1 500 / 11 000 / 16 000 at 2.00 USD/M.
    #[test]
    fn a_three_minute_twenty_track_costs_cents_not_tenths() {
        let estimate = semantic_estimate(MIMO, 200.0).expect("priced");
        assert_eq!(estimate.input_tokens, 1_680);
        assert!(close(estimate.usd_typical[0], 0.003_672), "{estimate:?}");
        assert!(close(estimate.usd_typical[1], 0.022_672), "{estimate:?}");
        assert!(close(estimate.usd_max, 0.032_672), "{estimate:?}");
        assert_eq!(
            estimate.sentence(),
            "Cost: about 0.004\u{2013}0.023 USD, at most 0.033 USD (16000-token cap)"
        );
    }

    /// Gemini at 300 s: 9 600 audio tokens (32 per second) plus 1 650 of text
    /// at 0.825 USD/M, and 1 500 / 8 000 / 16 000 completion tokens at 4.125.
    #[test]
    fn gemini_on_a_five_minute_track() {
        let estimate = semantic_estimate(GEMINI, 300.0).expect("priced");
        assert_eq!(estimate.input_tokens, 11_250);
        assert!(close(estimate.usd_typical[0], 0.015_468_75), "{estimate:?}");
        assert!(close(estimate.usd_typical[1], 0.042_281_25), "{estimate:?}");
        assert!(close(estimate.usd_max, 0.075_281_25), "{estimate:?}");
        assert_eq!(
            estimate.sentence(),
            "Cost: about 0.015\u{2013}0.042 USD, at most 0.076 USD (16000-token cap)"
        );
    }

    /// The three real calls of 2026-11-15 (OpenRouter's `usage.cost`): two
    /// excerpts of 100 s at 0.01148025 and 0.01029525 USD, and 300 s at
    /// 0.0257265. Each lies inside its range and under its ceiling.
    #[test]
    fn geminis_recorded_calls_fall_inside_the_range() {
        for (seconds, reported) in [
            (100.0, 0.011_480_25),
            (100.0, 0.010_295_25),
            (300.0, 0.025_726_5),
        ] {
            let estimate = semantic_estimate(GEMINI, seconds).expect("priced");
            assert!(
                estimate.usd_typical[0] <= reported && reported <= estimate.usd_typical[1],
                "{seconds} s: {reported} outside {estimate:?}"
            );
            assert!(reported < estimate.usd_max);
        }
    }

    /// A maximum is rounded up at the precision shown: 100 s is 940 prompt
    /// tokens, so a capped call is 0.000376 + 0.032 = 0.032376 USD, which a
    /// nearest rounding showed as "at most 0.032 USD". The helper's failure
    /// sentence pins the same figure (`tests/test_semantic_openrouter.py`).
    #[test]
    fn at_most_is_rounded_up_never_down() {
        let estimate = semantic_estimate(MIMO, 100.0).expect("priced");
        assert_eq!(estimate.input_tokens, 940);
        assert!((estimate.usd_max - 0.032_376).abs() < 1e-12, "{estimate:?}");
        assert_eq!(
            estimate.sentence(),
            "Cost: about 0.003\u{2013}0.022 USD, at most 0.033 USD (16000-token cap)"
        );
        // Exact thousandths stay where they are; noise above one does not
        // add a thousandth.
        assert_eq!(ceil_thousandths(0.033), 0.033);
        assert_eq!(ceil_thousandths(0.032_000_000_000_1), 0.032);
        assert_eq!(ceil_thousandths(0.032_001), 0.033);
        assert_eq!(ceil_thousandths(0.0), 0.0);
    }

    /// The benchmark's real whole-excerpt call: 100 s, 940 prompt tokens and
    /// 5 110 completion tokens, reported at 0.010596 USD. Its completion falls
    /// inside the range, so its cost has to as well.
    #[test]
    fn the_benchmarks_recorded_call_falls_inside_the_range() {
        let estimate = semantic_estimate(MIMO, 100.0).expect("priced");
        assert!(estimate.usd_typical[0] <= 0.010_596 && 0.010_596 <= estimate.usd_typical[1]);
        // And this lane's own two capped calls on the same excerpt: 928 prompt
        // tokens and 2 407 / 2 044 completion tokens, reported at 0.0051852 and
        // 0.00417248 USD.
        for reported in [0.005_185_2, 0.004_172_48] {
            assert!(estimate.usd_typical[0] <= reported && reported <= estimate.usd_typical[1]);
        }
        // The runaway (0.131 USD) is past the ceiling, which is the cap's point.
        assert!(estimate.usd_max < 0.131);
    }

    /// The order-of-magnitude error the estimate exists for: audio alone, at
    /// the prompt rate, is under a sixth of the low end.
    #[test]
    fn the_audio_is_a_small_part_of_the_price() {
        let estimate = semantic_estimate(MIMO, 200.0).expect("priced");
        let audio_only = 1_480.0 * 0.40 / 1_000_000.0;
        assert!(audio_only * 6.0 < estimate.usd_typical[0]);
    }

    #[test]
    fn an_unlisted_model_or_a_non_duration_has_no_figure() {
        assert_eq!(semantic_estimate("acme/never-priced", 200.0), None);
        assert_eq!(semantic_estimate(MIMO, 0.0), None);
        assert_eq!(semantic_estimate(MIMO, f64::NAN), None);
        assert_eq!(semantic_estimate(GEMINI, f64::INFINITY), None);
        assert_eq!(max_completion_tokens("acme/never-priced"), 16_000);
        assert!(unpriced_sentence("acme/never-priced").contains("16000-token cap"));
        assert_eq!(
            semantic_line("acme/never-priced", 200.0),
            unpriced_sentence("acme/never-priced")
        );
        let no_length = semantic_line(GEMINI, f64::NAN);
        assert!(
            no_length.contains("needs the track's length"),
            "{no_length}"
        );
        assert!(!no_length.contains("USD"), "{no_length}");
    }

    /// The line fits the confirmation at the column's default width: a priced
    /// line, at any track length up to an hour, stays at 64 characters or
    /// fewer.
    #[test]
    fn the_line_is_short_enough_for_the_column() {
        for model_id in [MIMO, GEMINI] {
            for seconds in [30.0, 200.0, 3_600.0] {
                let line = semantic_line(model_id, seconds);
                let length = line.chars().count();
                assert!(length <= 64, "{length} chars: {line}");
            }
        }
    }
}
