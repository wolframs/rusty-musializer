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
        println!("mode {index} badge|{}", mode.badge());
        println!("mode {index} workflow|{}", mode.workflow());
        println!("mode {index} boundary|{}", mode.data_boundary());
        println!("mode {index} empty|{}", mode.empty_result());
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

    let widths = [480.0f32, 620.0, 700.0, 759.0, 760.0, 948.0, 1268.0, 1908.0];
    for width in widths {
        for (content_i, content) in AssistPanelContent::ALL.into_iter().enumerate() {
            for reference in 0..2 {
                let layout = assist_ui_state::ui_layout(width, content, reference != 0);
                println!(
                    "layout {} {content_i} {reference}|{}|{}|{}|{}|{}|{}|{}|{}",
                    g9(f64::from(width)),
                    layout.mode_columns,
                    layout.mode_rows,
                    g9(f64::from(layout.mode_top)),
                    g9(f64::from(layout.mode_row_height)),
                    g9(f64::from(layout.status_y)),
                    g9(f64::from(layout.content_y)),
                    g9(f64::from(layout.reference_y)),
                    g9(f64::from(layout.required_height)),
                );
            }
        }
    }

    let screens = [640.0f32, 720.0, 1080.0, 300.0, 0.0, -1.0];
    let panels = [178.0f32, 240.0, 274.0, 0.0, -5.0];
    for screen in screens {
        for panel in panels {
            println!(
                "timeline {} {}|{}",
                g9(f64::from(screen)),
                g9(f64::from(panel)),
                g9(f64::from(assist_ui_state::timeline_height(
                    screen, 50.0, panel
                )))
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
