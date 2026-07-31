//! The transport row's control vocabulary: an icon, a text fallback, and the
//! tooltip that names both.
//!
//! **Not a port.** The frozen C's chrome is text-labelled throughout; this row
//! draws [`Icon`]s so that eleven controls fit where six text buttons used to.
//! That trade is only acceptable because of the two things this module keeps
//! together with the glyph:
//!
//! - **A tooltip that spells the control out, with its keyboard shortcut.**
//!   Nothing here is available only as a picture, and the shortcut had nowhere
//!   else to be written down once the labels went.
//! - **A text fallback.** The icon face is the one face whose absence cannot be
//!   approximated — raylib's default has no Private Use Area coverage, so an icon
//!   drawn through it is an empty box. When [`Faces::icons_available`] is false
//!   the row draws [`Control::text`] instead, which is the same interface with
//!   worse density rather than a row of blanks.
//!
//! Keeping all three in one table is the point: a control added with an icon and
//! no tooltip would be a mystery button, and one added with no fallback would be
//! invisible on a machine where the atlas failed.

use musializer_runtime::font::{Face, Faces, Icon};

/// One control in the transport row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Control {
    pub icon: Icon,
    /// Drawn instead of the icon when the icon face is unavailable. Deliberately
    /// terse — this row is tight enough that the fallback has to be, too.
    pub text: &'static str,
    /// The tooltip, including the keyboard shortcut in square brackets where the
    /// control has one. The brackets are the convention the whole row follows, so
    /// a control without them is visibly a control without a shortcut.
    pub tip: &'static str,
}

impl Control {
    const fn new(icon: Icon, text: &'static str, tip: &'static str) -> Self {
        Self { icon, text, tip }
    }
}

pub const PLAY: Control = Control::new(Icon::Play, "Play", "Play [Space]");
pub const PAUSE: Control = Control::new(Icon::Pause, "Pause", "Pause [Space]");

pub const SEEK_START: Control = Control::new(Icon::StepBackward, "|<", "Jump to start [Home]");
pub const SEEK_BACK: Control = Control::new(
    Icon::ChevronLeft,
    "-1s",
    "Back 1 s  [\u{2190}]   \u{00b7}  Ctrl 0.1 s  \u{00b7}  Shift 10 s",
);
pub const SEEK_FORWARD: Control = Control::new(
    Icon::ChevronRight,
    "+1s",
    "Forward 1 s  [\u{2192}]  \u{00b7}  Ctrl 0.1 s  \u{00b7}  Shift 10 s",
);

pub const TUNE: Control = Control::new(Icon::Sliders, "Tune", "Tuning inspector [T]");
pub const EXPORT: Control = Control::new(Icon::Film, "Export", "Render to video");
pub const LYRICS: Control = Control::new(Icon::FileText, "Lyrics", "Lyrics editor");
pub const ASSIST: Control = Control::new(Icon::Magic, "Assist", "Analysis assist");

pub const READOUT: Control = Control::new(Icon::Info, "Info", "Diagnostic readout [H]");
pub const MUTE: Control = Control::new(Icon::VolumeUp, "Mute", "Mute [M]");
pub const UNMUTE: Control = Control::new(Icon::VolumeOff, "Sound", "Unmute [M]");
pub const FULLSCREEN: Control = Control::new(Icon::Expand, "Full", "Fullscreen [F]");
pub const WINDOWED: Control = Control::new(Icon::Compress, "Exit", "Leave fullscreen [F / Esc]");

/// The speaker icon for a volume level, so the glyph itself reports roughly how
/// loud the output is.
///
/// Three states rather than a continuous meter because that is what the face has,
/// and because the slider beside it already carries the precise value. The
/// boundary is at a third rather than a half: the volume curve is applied
/// linearly to amplitude, so a third of the slider is already well down in
/// perceived loudness and the "quiet" glyph earns its range.
#[must_use]
pub fn volume_icon(volume: f32, muted: bool) -> Icon {
    if muted || volume <= 0.0 {
        Icon::VolumeOff
    } else if volume < 0.34 {
        Icon::VolumeDown
    } else {
        Icon::VolumeUp
    }
}

/// The face a control's glyph should be drawn with, and the string to draw.
///
/// One function so that no caller can pick the icon face and then draw the text
/// fallback through it, or the reverse — which would render either an empty box or
/// a word in a font with no letters.
#[must_use]
pub fn glyph<'a>(fonts: &'a Faces, control: &Control) -> (&'a Face, String) {
    if fonts.icons_available() {
        (fonts.icons(), control.icon.glyph().to_string())
    } else {
        (fonts.ui(), control.text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_volume_glyph_tracks_the_level_and_mute_wins() {
        assert_eq!(volume_icon(1.0, false), Icon::VolumeUp);
        assert_eq!(volume_icon(0.5, false), Icon::VolumeUp);
        assert_eq!(volume_icon(0.2, false), Icon::VolumeDown);
        assert_eq!(volume_icon(0.0, false), Icon::VolumeOff);
        // Muted at full volume still reads as silent, because it is. A speaker
        // glyph showing bars while nothing is audible is the control lying.
        assert_eq!(volume_icon(1.0, true), Icon::VolumeOff);
    }

    /// Every control names itself in its tooltip, and every control that has a
    /// keyboard binding says so there.
    ///
    /// Worth asserting rather than eyeballing: the tooltip is now the *only* place
    /// a shortcut is written down, so one omitted is a binding nobody can discover.
    #[test]
    fn every_control_has_a_tooltip_and_a_text_fallback() {
        let all = [
            PLAY,
            PAUSE,
            SEEK_START,
            SEEK_BACK,
            SEEK_FORWARD,
            TUNE,
            EXPORT,
            LYRICS,
            ASSIST,
            READOUT,
            MUTE,
            UNMUTE,
            FULLSCREEN,
            WINDOWED,
        ];
        for control in all {
            assert!(!control.tip.is_empty(), "{control:?} has no tooltip");
            assert!(!control.text.is_empty(), "{control:?} has no text fallback");
            // The fallback is drawn in a row sized for icons, so a long word would
            // be ellipsized into uselessness rather than merely tight.
            assert!(
                control.text.chars().count() <= 6,
                "{:?}'s fallback is too long for the row it replaces an icon in",
                control.text
            );
        }
        for control in [
            PLAY,
            PAUSE,
            SEEK_START,
            SEEK_BACK,
            SEEK_FORWARD,
            TUNE,
            READOUT,
            MUTE,
            UNMUTE,
            FULLSCREEN,
            WINDOWED,
        ] {
            assert!(
                control.tip.contains('['),
                "{:?} has a keyboard binding but does not name it",
                control.text
            );
        }
    }
}
