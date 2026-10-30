//! Dumps the Rust Assist panel policy in the same format as
//! `tests/differential/assist_ui_oracle.c`, so a hand-transcribed module that
//! carries the whole panel's copy, geometry and guard ladder can be checked
//! against the frozen C mechanically rather than by eye.
//!
//! Run through `tools/differential_assist_ui.sh`.

use musializer_core::ui::assist_ui_state::{
    self, AssistJobState, AssistLyricReference, AssistMode, AssistPanelContent, AssistStartBlock,
    ASSIST_JOB_TIMEOUT_SECONDS,
};

fn bools(value: bool) -> &'static str {
    if value {
        "1"
    } else {
        "0"
    }
}

fn main() {
    for (index, mode) in AssistMode::ALL.into_iter().enumerate() {
        println!("mode {index} name|{}", mode.display_name());
        println!("mode {index} arg|{}", mode.argument());
        println!(
            "mode {index} uses_reference|{}",
            bools(mode.uses_lyric_reference())
        );
    }

    for (index, state) in AssistJobState::ALL.into_iter().enumerate() {
        println!("state {index} active|{}", bools(state.is_active()));
        println!(
            "state {index} expired|{}|{}|{}",
            bools(state.deadline_expired(10.0, 10.0 + ASSIST_JOB_TIMEOUT_SECONDS)),
            bools(state.deadline_expired(10.0, 10.0 + ASSIST_JOB_TIMEOUT_SECONDS - 0.001)),
            bools(state.deadline_expired(10.0, 9.0))
        );
        println!(
            "state {index} remaining|{}|{}|{}",
            g9(state.deadline_remaining(10.0, 10.0)),
            g9(state.deadline_remaining(10.0, 610.0)),
            g9(state.deadline_remaining(10.0, 10.0 + ASSIST_JOB_TIMEOUT_SECONDS + 1.0))
        );
    }

    for helper in 0..2 {
        for (state_index, state) in AssistJobState::ALL.into_iter().enumerate() {
            for pending in 0..2 {
                let block = assist_ui_state::start_block(helper != 0, state, pending != 0);
                println!(
                    "block {helper} {state_index} {pending} {}|{}",
                    block_index(block),
                    block.reason()
                );
            }
        }
    }

    for (state_index, state) in AssistJobState::ALL.into_iter().enumerate() {
        for confirm in 0..2 {
            for candidate in 0..2 {
                let content = assist_ui_state::panel_content(state, confirm != 0, candidate != 0);
                println!(
                    "content {state_index} {confirm} {candidate} {}",
                    content_index(content)
                );
            }
        }
    }

    for authorized in 0..8u32 {
        for available in 0..8u32 {
            println!(
                "changes {authorized} {available} {}",
                bools(assist_ui_state::result_has_changes(authorized, available))
            );
        }
    }
    for replaces in 0..2 {
        for active in 0..2 {
            for dirty in 0..2 {
                println!(
                    "draft {replaces} {active} {dirty} {}",
                    bools(assist_ui_state::candidate_conflicts_with_lyric_draft(
                        replaces != 0,
                        active != 0,
                        dirty != 0
                    ))
                );
            }
        }
    }

    for (index, reference) in AssistLyricReference::ALL.into_iter().enumerate() {
        println!("reference {index}|{}", reference.summary());
    }

    let paths = [
        "kitty.mp3",
        "/music/kitty.mp3",
        "/music/a.b.mp3",
        "kitty",
        ".mp3",
        "/music/.mp3",
        "/my.music/kitty",
        "C:\\my.music\\kitty",
        "/a/b/c.d/e",
        "x.",
        ".",
        "..",
    ];
    for path in paths {
        match assist_ui_state::lyric_sibling_path(path) {
            Some(sibling) => println!("sibling {path}|1|{sibling}"),
            None => println!("sibling {path}|0|"),
        }
    }
    println!(
        "sibling <empty>|{}|",
        bools(assist_ui_state::lyric_sibling_path("").is_some())
    );

    // `ui_layout` and `timeline_height` (`assist_ui_state.c:219-270`) left
    // the oracle half in WX-5d: the band they sized is gone from the window,
    // which lays Assist out in the right column. The script drops the
    // oracle's rows for them too. What replaced them is pinned below.
    println!("--- post-legacy (no oracle) ---");
    post_legacy();
}

/// The column's Assist layout (WX-5d §3.4) over a width x body sweep, every
/// rect as integers. Diffed against `tests/differential/assist_ui_post_legacy.txt`.
fn post_legacy() {
    // The copy rows. They left the oracle half because the copy itself left
    // the C on 2026-08-18 (e8e2aa6, "Make Assist routing and Codex execution
    // truthful": the lyric route keeps audio local and names MMS), and the
    // harness, run on demand only, was not updated then. Pinned here instead,
    // so a copy change is still a visible diff.
    for (index, mode) in AssistMode::ALL.into_iter().enumerate() {
        println!("mode {index} badge|{}", mode.badge());
        println!("mode {index} workflow|{}", mode.workflow());
        println!("mode {index} boundary|{}", mode.data_boundary());
        println!("mode {index} empty|{}", mode.empty_result());
    }

    use musializer_core::ui::assist_ui_state::{
        assist_column_layout, card_boundary, card_height, open_body_height, open_card,
        AssistColumnSpec, OpenBodyLayout, OpenCardSpec,
    };
    use musializer_core::ui::workspace_layout::UiRect;

    let r = |rect: UiRect| {
        format!(
            "{},{},{},{}",
            g9(f64::from(rect.x)),
            g9(f64::from(rect.y)),
            g9(f64::from(rect.width)),
            g9(f64::from(rect.height))
        )
    };
    let o = |rect: Option<UiRect>| rect.map_or_else(|| "-".to_string(), r);

    for lines in 0..4 {
        println!("card_height {lines}|{}", g9(f64::from(card_height(lines))));
    }
    let bodies: [(&str, Option<OpenCardSpec>); 9] = [
        ("ready", None),
        (
            "confirm",
            Some(OpenCardSpec::Confirmation {
                consent_lines: 2,
                reference_lines: None,
                route_lines: 0,
            }),
        ),
        (
            "confirm-reference",
            Some(OpenCardSpec::Confirmation {
                consent_lines: 2,
                reference_lines: Some(3),
                route_lines: 0,
            }),
        ),
        (
            "confirm-routes",
            Some(OpenCardSpec::Confirmation {
                consent_lines: 3,
                reference_lines: Some(2),
                route_lines: 8,
            }),
        ),
        ("running", Some(OpenCardSpec::Running { lines: 2 })),
        ("cancelling", Some(OpenCardSpec::Cancelling { lines: 2 })),
        (
            "candidate",
            Some(OpenCardSpec::Candidate {
                lanes: 3,
                extra_line: true,
                reason_lines: 1,
                review_height: 0,
            }),
        ),
        (
            "candidate-review",
            Some(OpenCardSpec::Candidate {
                lanes: 1,
                extra_line: false,
                reason_lines: 0,
                review_height: 82,
            }),
        ),
        ("empty", Some(OpenCardSpec::Empty { lines: 2 })),
    ];
    for (name, body) in bodies {
        if let Some(body) = body {
            println!(
                "open_body_height {name}|{}",
                g9(f64::from(open_body_height(body)))
            );
        }
    }

    // The column's widths: the 320 floor, 1280's 358, the 460 ceiling, and a
    // user-dragged 300; the 1280x720 body's height (634) and 1920's (994).
    for (width, height) in [
        (300.0f32, 554.0f32),
        (320.0, 554.0),
        (358.0, 634.0),
        (460.0, 994.0),
    ] {
        let body = UiRect::new(1280.0 - width, 86.0, width, height);
        for (name, open) in bodies {
            for (mode_index, mode) in AssistMode::ALL.into_iter().enumerate() {
                if open.is_none() && mode_index > 0 {
                    continue;
                }
                let spec = AssistColumnSpec {
                    intro_lines: 2,
                    description_lines: [2, 1, 2, 1],
                    open: open.map(|open| (mode, open)),
                };
                for offset in [0.0f32, 1.0e9] {
                    let layout = assist_column_layout(body, &spec, offset);
                    let cards: Vec<String> = layout
                        .cards
                        .iter()
                        .map(|card| {
                            format!(
                                "{}/{}/{}/{}",
                                r(card.card),
                                r(card.title),
                                r(card.boundary),
                                r(card.divider)
                            )
                        })
                        .collect();
                    let open_rects = match layout.open.map(|(_, body)| body) {
                        None => "-".to_string(),
                        Some(OpenBodyLayout::Confirmation {
                            boundary_head,
                            consent,
                            reference_head,
                            reference,
                            choose,
                            clear,
                            routes,
                            start,
                            cancel,
                            deadline,
                        }) => format!(
                            "confirm {} {} {} {} {} {} {} {} {} {}",
                            r(boundary_head),
                            r(consent),
                            o(reference_head),
                            o(reference),
                            o(choose),
                            o(clear),
                            o(routes),
                            r(start),
                            r(cancel),
                            r(deadline)
                        ),
                        Some(OpenBodyLayout::Running {
                            status,
                            sweep,
                            text,
                            cancel_job,
                        }) => format!(
                            "running {} {} {} {}",
                            r(status),
                            r(sweep),
                            r(text),
                            r(cancel_job)
                        ),
                        Some(OpenBodyLayout::Cancelling { text }) => {
                            format!("cancelling {}", r(text))
                        }
                        Some(OpenBodyLayout::Candidate {
                            validated,
                            lanes,
                            extra,
                            reason,
                            review,
                            apply,
                            discard,
                            links,
                        }) => format!(
                            "candidate {} {} {} {} {} {} {} {} {} {}",
                            r(validated),
                            r(lanes),
                            o(extra),
                            o(reason),
                            o(review),
                            r(apply),
                            r(discard),
                            r(links[0]),
                            r(links[1]),
                            r(links[2])
                        ),
                        Some(OpenBodyLayout::Empty { text, links }) => format!(
                            "empty {} {} {} {}",
                            r(text),
                            r(links[0]),
                            r(links[1]),
                            r(links[2])
                        ),
                    };
                    println!(
                        "column {} {} {name} {mode_index} {}|{}|{}|{}|{} {} {}|{} {}|{}|{}|{}",
                        g9(f64::from(width)),
                        g9(f64::from(height)),
                        g9(f64::from(offset.min(1.0))),
                        r(layout.intro),
                        cards.join(" "),
                        open_rects,
                        r(layout.auto_head),
                        r(layout.auto_switch),
                        r(layout.auto_sentence),
                        r(layout.viewport),
                        r(layout.footer),
                        r(layout.status),
                        g9(f64::from(layout.content_height)),
                        g9(f64::from(layout.offset)),
                    );
                }
            }
        }
    }

    for (index, content) in AssistPanelContent::ALL.into_iter().enumerate() {
        for mode in AssistMode::ALL {
            println!(
                "open_card {index} {}|{}",
                mode.argument(),
                open_card(content, mode, AssistMode::Mimo).map_or("-", AssistMode::argument)
            );
        }
    }
    for mode in AssistMode::ALL {
        for audio in [false, true] {
            let (word, sends) = card_boundary(mode, audio);
            println!(
                "boundary {} {}|{word}|{}",
                mode.argument(),
                bools(audio),
                bools(sends)
            );
        }
    }
}

/// The C's enumerator order, which the harness compares as an integer.
fn block_index(block: AssistStartBlock) -> usize {
    AssistStartBlock::ALL
        .iter()
        .position(|candidate| *candidate == block)
        .expect("every block is in ALL")
}

fn content_index(content: AssistPanelContent) -> usize {
    AssistPanelContent::ALL
        .iter()
        .position(|candidate| *candidate == content)
        .expect("every body is in ALL")
}

/// Formats like C's `%.9g`: nine significant digits, no trailing zeros, no
/// unnecessary decimal point. Copied from `settings_dump` rather than shared,
/// because examples have no common module and one copy per harness is cheaper
/// than a crate-level helper nothing else wants.
fn g9(value: f64) -> String {
    if value == 0.0 {
        // C prints "0" for both zeroes; Rust would print "-0" for the negative.
        return "0".to_string();
    }
    let formatted = format!("{:.*e}", 8, value);
    let (mantissa, exponent) = formatted.split_once('e').expect("scientific form");
    let exponent: i32 = exponent.parse().expect("an exponent");
    // %g uses scientific notation below -4 or at or above the precision.
    if !(-4..9).contains(&exponent) {
        let mantissa = trim_zeros(mantissa);
        return format!(
            "{mantissa}e{}{:02}",
            if exponent < 0 { '-' } else { '+' },
            exponent.abs()
        );
    }
    let decimals = (8 - exponent).max(0) as usize;
    trim_zeros(&format!("{value:.decimals$}")).to_string()
}

fn trim_zeros(text: &str) -> &str {
    if !text.contains('.') {
        return text;
    }
    text.trim_end_matches('0').trim_end_matches('.')
}
