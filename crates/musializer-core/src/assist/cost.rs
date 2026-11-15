//! What a MiMo feelings call should cost, said before it is sent.
//!
//! The 2026-11-14 benchmark (`docs/MIMO_BENCHMARK_RESULTS.md`, "Cost") found
//! the money is in the **answer**, not the audio: MiMo reasons before it
//! answers, 600–9 200 reasoning tokens per call billed at the completion rate,
//! while audio bills at 7.4 tokens per second at the prompt rate. A whole-track
//! call costs about 0.01–0.02 USD. An estimate built on audio seconds is wrong
//! by an order of magnitude, so this one is built on completion tokens: a range
//! from the measured span of answer lengths, and a ceiling from the completion
//! cap the helper sends, which is the most one call can cost.
//!
//! Every constant here has a twin in `tools/semantic_openrouter.py`, which records
//! the same estimate beside what OpenRouter reported. Both copies are pinned
//! against each other by `tests/test_cross_language_pins.py`; change both or
//! neither.

/// The helper's `max_tokens`, reasoning included (`MAX_COMPLETION_TOKENS`).
pub const SEMANTIC_MAX_COMPLETION_TOKENS: u32 = 16_000;

/// Calibrated from the benchmark's recorded `usage.prompt_tokens`.
pub const AUDIO_TOKENS_PER_SECOND: f64 = 7.4;

/// The request's text as prompt tokens: the 2026-11-14 capped call reported 928
/// prompt tokens for 100 s, 740 of them audio at the rate above.
pub const PROMPT_TEXT_TOKENS: u32 = 200;

/// The span of whole-excerpt answers seen so far: 2 044 and 2 407 tokens on
/// this lane's capped request (two calls, 2026-11-14), 5 110 on the benchmark's
/// shape of it, 10 942 the largest legitimate answer.
pub const TYPICAL_COMPLETION_TOKENS_LOW: u32 = 1_500;
pub const TYPICAL_COMPLETION_TOKENS_HIGH: u32 = 11_000;

/// The date the prices below were read off OpenRouter's endpoint list.
pub const PRICES_LISTED_ON: &str = "2026-11-14";

/// USD per million tokens. Audio has no separate price on this model's only
/// provider and bills as prompt tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelPrice {
    pub model_id: &'static str,
    pub prompt_usd_per_million: f64,
    pub completion_usd_per_million: f64,
}

/// A dated table, not a live quote. A model missing from it gets no dollar
/// figure rather than a guessed one.
#[rustfmt::skip]
pub const PRICES: &[ModelPrice] = &[
    ModelPrice { model_id: "xiaomi/mimo-v2.5", prompt_usd_per_million: 0.40, completion_usd_per_million: 2.00 },
];

/// One call's estimate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CostEstimate {
    pub input_tokens: u32,
    /// From [`TYPICAL_COMPLETION_TOKENS_LOW`] and `_HIGH`.
    pub usd_typical: [f64; 2],
    /// A call that runs to [`SEMANTIC_MAX_COMPLETION_TOKENS`].
    pub usd_max: f64,
}

impl CostEstimate {
    /// The line the confirmation shows. The range is "about", rounded to the
    /// nearest thousandth; the maximum is a bound, so it is rounded **up**
    /// ([`ceil_thousandths`]): a capped call on 100 s costs 0.032376 USD and
    /// reads "at most 0.033 USD", never 0.032.
    #[must_use]
    pub fn sentence(&self) -> String {
        format!(
            "Estimated cost: about {:.3}\u{2013}{:.3} USD, at most {:.3} USD (answer capped at {} tokens)",
            self.usd_typical[0],
            self.usd_typical[1],
            ceil_thousandths(self.usd_max),
            SEMANTIC_MAX_COMPLETION_TOKENS,
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
pub fn price(model_id: &str) -> Option<ModelPrice> {
    PRICES.iter().copied().find(|row| row.model_id == model_id)
}

/// Prompt tokens for `duration_seconds` of audio plus the request's text,
/// rounded up as the helper rounds them.
#[must_use]
pub fn input_tokens(duration_seconds: f64) -> u32 {
    let audio = (duration_seconds.max(0.0) * AUDIO_TOKENS_PER_SECOND).ceil();
    // A track is minutes long; the cast cannot truncate anything real, and a
    // non-finite duration never reaches it (see `semantic_estimate`).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let audio = audio.min(f64::from(u32::MAX / 2)) as u32;
    audio.saturating_add(PROMPT_TEXT_TOKENS)
}

/// The estimate for one semantic call, or `None` when the model has no listed
/// price or the duration is not a duration.
#[must_use]
pub fn semantic_estimate(model_id: &str, duration_seconds: f64) -> Option<CostEstimate> {
    if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return None;
    }
    let price = price(model_id)?;
    let input = input_tokens(duration_seconds);
    let usd = |completion: u32| {
        (f64::from(input) * price.prompt_usd_per_million
            + f64::from(completion) * price.completion_usd_per_million)
            / 1_000_000.0
    };
    Some(CostEstimate {
        input_tokens: input,
        usd_typical: [
            usd(TYPICAL_COMPLETION_TOKENS_LOW),
            usd(TYPICAL_COMPLETION_TOKENS_HIGH),
        ],
        usd_max: usd(SEMANTIC_MAX_COMPLETION_TOKENS),
    })
}

/// What the confirmation says for a model with no listed price: the cap still
/// bounds the answer, and the user is told there is no figure rather than shown
/// a guessed one.
#[must_use]
pub fn unpriced_sentence(model_id: &str) -> String {
    format!(
        "No price is listed here for {model_id}; its answer is capped at {SEMANTIC_MAX_COMPLETION_TOKENS} tokens"
    )
}

/// The confirmation's cost line for a semantic route: the estimate, or why
/// there is no figure. Every variant names the cap, which holds either way.
#[must_use]
pub fn semantic_line(model_id: &str, duration_seconds: f64) -> String {
    match (price(model_id), semantic_estimate(model_id, duration_seconds)) {
        (None, _) => unpriced_sentence(model_id),
        (Some(_), None) => format!(
            "Estimated cost: needs the track's length; the answer is capped at {SEMANTIC_MAX_COMPLETION_TOKENS} tokens"
        ),
        (Some(_), Some(estimate)) => estimate.sentence(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pinned numbers, not properties: 200 s is 1 480 audio tokens plus 200 of
    /// text, so 1 680 at 0.40 USD/M and 1 500 / 11 000 / 16 000 at 2.00 USD/M.
    #[test]
    fn a_three_minute_twenty_track_costs_cents_not_tenths() {
        let estimate = semantic_estimate("xiaomi/mimo-v2.5", 200.0).expect("priced");
        assert_eq!(estimate.input_tokens, 1_680);
        let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
        assert!(close(estimate.usd_typical[0], 0.003_672), "{estimate:?}");
        assert!(close(estimate.usd_typical[1], 0.022_672), "{estimate:?}");
        assert!(close(estimate.usd_max, 0.032_672), "{estimate:?}");
        assert_eq!(
            estimate.sentence(),
            "Estimated cost: about 0.004\u{2013}0.023 USD, at most 0.033 USD (answer capped at 16000 tokens)"
        );
    }

    /// A maximum is rounded up at the precision shown: 100 s is 940 prompt
    /// tokens, so a capped call is 0.000376 + 0.032 = 0.032376 USD, which a
    /// nearest rounding showed as "at most 0.032 USD". The helper's failure
    /// sentence pins the same figure (`tests/test_semantic_openrouter.py`).
    #[test]
    fn at_most_is_rounded_up_never_down() {
        let estimate = semantic_estimate("xiaomi/mimo-v2.5", 100.0).expect("priced");
        assert_eq!(estimate.input_tokens, 940);
        assert!((estimate.usd_max - 0.032_376).abs() < 1e-12, "{estimate:?}");
        assert_eq!(
            estimate.sentence(),
            "Estimated cost: about 0.003\u{2013}0.022 USD, at most 0.033 USD (answer capped at 16000 tokens)"
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
        let estimate = semantic_estimate("xiaomi/mimo-v2.5", 100.0).expect("priced");
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

    /// The order-of-magnitude error the change exists for: audio alone, at the
    /// prompt rate, is under a sixth of the low end.
    #[test]
    fn the_audio_is_a_small_part_of_the_price() {
        let estimate = semantic_estimate("xiaomi/mimo-v2.5", 200.0).expect("priced");
        let audio_only = 1_480.0 * 0.40 / 1_000_000.0;
        assert!(audio_only * 6.0 < estimate.usd_typical[0]);
    }

    #[test]
    fn an_unlisted_model_or_a_non_duration_has_no_figure() {
        assert_eq!(semantic_estimate("acme/never-priced", 200.0), None);
        assert_eq!(semantic_estimate("xiaomi/mimo-v2.5", 0.0), None);
        assert_eq!(semantic_estimate("xiaomi/mimo-v2.5", f64::NAN), None);
        assert_eq!(semantic_estimate("xiaomi/mimo-v2.5", f64::INFINITY), None);
        assert!(unpriced_sentence("acme/never-priced").contains("16000 tokens"));
        assert_eq!(
            semantic_line("acme/never-priced", 200.0),
            unpriced_sentence("acme/never-priced")
        );
        let no_length = semantic_line("xiaomi/mimo-v2.5", f64::NAN);
        assert!(
            no_length.contains("needs the track's length"),
            "{no_length}"
        );
        assert!(!no_length.contains("USD"), "{no_length}");
    }
}
