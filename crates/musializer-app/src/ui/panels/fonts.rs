//! The font browser pane, and the faces it adds.
//!
//! **Owner: Agent K.** C sources: `font_catalogue.c`, `font_import_state.c`,
//! `plug.c:373-426`, `:1547-1935`, and `draw_font_consent` / `draw_font_browser`
//! (`lyrics_editor_ui.c:617-833`).
//!
//! This is a **pane inside the lyrics editor**, not a panel of its own — the
//! oracle drives it with `p->lyric_editor.font_pane` (`plug.c:3786`), which is
//! why there is no `UiPanel` variant for it and why it is called from
//! [`super::lyrics`].
//!
//! # What is here, and what is not
//!
//! Only pixels and raylib input. Every decision the pane makes lives somewhere
//! already testable without a window:
//!
//! - the rectangles and the size threshold: `font_import_state::BrowserLayout`
//! - the query, the scroll window, the selection, the consent flag:
//!   `font_import_state::BrowserView`
//! - which body to draw: `font_import_state::panel`
//! - the child process, the digests, the catalogue reader:
//!   [`FontImporter`]
//!
//! # The consent is not a preference
//!
//! `BrowserView::network_allowed` starts false in every run, is never written
//! anywhere, and there is no path that restores it from a file. That is the
//! oracle's rule (`plug.c:1542-1548`: "this one is asked again every run,
//! because 'I wanted a font earlier' is not standing permission to contact
//! anyone later") and the consent copy promises it in as many words. Do not
//! "improve" it into a saved setting.

// Nothing in this module has a caller yet, and both of its two entry points are
// in files this agent may not edit: `Shell::font_browser_pane` is called from
// Agent I's lyrics editor, and `FontBrowser` is constructed and polled by
// `main.rs`. The scaffold carried the same allow on the stub for the same
// reason. The Agent K note in `REWRITE_PLAN.md` has both diffs; **delete this
// attribute when they land** — after that, a dead function here is a real one.
#![allow(
    dead_code,
    reason = "the two call sites are in shared files this fan-out agent may not edit"
)]

use musializer_core::ui::font_import_state::{
    self as font_state, BrowserLayout, BrowserView, FontImportPanel,
};
use musializer_core::ui::workspace_layout::UiRect;
use musializer_runtime::font::UiFonts;
use musializer_runtime::process::font_import::{
    describe_scripts, CatalogueEntry, FontImportOutcome, FontImporter, ImportManifest,
};
use raylib::prelude::{RaylibDraw, RaylibDrawHandle};

use super::super::shell::{Shell, ShellCommand, ShellInput};
use super::super::theme::color;
use super::super::widgets::{self, ButtonStyle, Widgets};

/// Widget id namespace for this pane.
///
/// A local constant rather than an entry in [`widgets::id`], because that module
/// is a shared file this fan-out's agents may not edit. `0x464F_4E54` is `FONT`
/// in ASCII, which cannot collide with the small integers every other namespace
/// uses — and a colliding id would let one widget release another's press, which
/// is the whole reason namespaces exist.
const ID: u32 = 0x464F_4E54;

/// Where the helper's per-job directories go (`plug.c:1624`, `"./build/fonts"`).
const WORKSPACE_ROOT: &str = "./build/fonts";

/// The font browser's state between frames, and the importer behind it.
///
/// One object rather than fields scattered across the shell, because the pane is
/// the only reader of any of it and because the `Drop` that reaps an abandoned
/// helper needs somewhere to live.
pub struct FontBrowser {
    view: BrowserView,
    importer: FontImporter,
    /// A verified download waiting to be rasterized and written into the current
    /// track.
    ///
    /// The pane cannot do either: it holds `&Faces` and `&Workspace`, both
    /// shared. So the last two steps of `font_job_finish_fetch`
    /// (`plug.c:1762-1798`) are handed out through [`Self::take_import`] and done
    /// where those resources are owned, which is `main.rs`.
    pending_import: Option<Box<ImportManifest>>,
}

impl Default for FontBrowser {
    fn default() -> Self {
        Self::new()
    }
}

impl FontBrowser {
    #[must_use]
    pub(crate) fn new() -> Self {
        // `GetApplicationDirectory()` is the directory of the running
        // executable, not the working directory — the distinction
        // `find_font_helper` depends on (`plug.c:1559`), because a source run
        // out of `./build` finds the helper one level up. `current_exe` is the
        // same fact without needing a window, so the importer stays
        // constructible in a headless test.
        let application = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        Self {
            view: BrowserView::new(),
            importer: FontImporter::new(&application, WORKSPACE_ROOT.into()),
            pending_import: None,
        }
    }

    /// Advances the helper (`poll_font_job`, `plug.c:1811-1861`).
    ///
    /// **Call this every frame, open or not.** A download that finished behind a
    /// closed pane still has to be reaped and still has to publish its result;
    /// polling only while the browser is visible would leave it permanently
    /// "in progress" and keep the child alive until exit.
    pub(crate) fn poll(&mut self, track_present: bool) {
        match self.importer.poll(track_present) {
            Some(FontImportOutcome::Imported(manifest)) => {
                self.pending_import = Some(manifest);
            }
            // A failure is already a sentence in `status()`, which the pane
            // draws. There is nothing else to carry out.
            Some(FontImportOutcome::Catalogue | FontImportOutcome::Failed) | None => {}
        }
    }

    /// Takes a verified download, if one is waiting.
    ///
    /// Both digests have already been re-checked against the files on disk
    /// (`plug.c:1750-1761`), so the caller may rasterize and record without
    /// hashing again. What the caller still owes is the rest of
    /// `font_job_finish_fetch`: make this the track's caption face, record the
    /// two runtime paths, mark the project dirty. Those happen in one step in
    /// the C for a reason — a project that names an imported face it does not
    /// carry fails its own validation.
    pub(crate) fn take_import(&mut self) -> Option<Box<ImportManifest>> {
        self.pending_import.take()
    }

    /// Loads a family list from a file rather than from the network
    /// (`--ui-probe fonts=PATH`).
    ///
    /// Invented: nothing in the oracle reaches the browsing state without
    /// contacting Google, and no check in this repository is allowed to do that.
    /// It grants no consent — reading a local file is not contacting anybody.
    pub(crate) fn load_catalogue_from_file(
        &mut self,
        path: &std::path::Path,
    ) -> Result<(), String> {
        self.importer
            .load_catalogue_from_file(path)
            .map_err(|error| error.to_string())
    }

    /// Grants this run's network consent, exactly as pressing the consent button
    /// does. `--ui-probe` uses it to photograph the browsing body rather than the
    /// question.
    pub(crate) fn allow_network(&mut self) {
        self.view.allow_network();
    }

    /// Forgets the chosen family. Called when the project's imported face is
    /// removed, because a selection naming a face nobody carries is how a second
    /// press downloads something nobody asked for.
    pub(crate) fn clear_selection(&mut self) {
        self.view.clear_selection();
    }

    /// One line of evidence for the slice report.
    ///
    /// Existence would be "the browser is there". This is the state it is in,
    /// which is what a capture script can assert on: whether consent has been
    /// asked, how many families are loaded, which body is drawn, and whether the
    /// helper this installation would need is even present.
    #[must_use]
    pub(crate) fn describe(&self) -> String {
        format!(
            "consent={}, families={}, panel={}, helper={}",
            if self.view.network_allowed() {
                "granted"
            } else {
                "not asked"
            },
            self.importer.catalogue().map_or(0, |list| list.len()),
            panel_name(self.importer.panel(self.view.network_allowed())),
            if self.importer.helper_available() {
                "found"
            } else {
                "missing"
            },
        )
    }
}

impl Shell {
    /// The font browser, inside the lyrics editor.
    ///
    /// Called by [`super::lyrics`] when its font pane is showing, so the two
    /// never fight over the same rectangle.
    ///
    /// `browser` is a separate argument rather than a field of `Shell` only
    /// because `shell.rs` is a shared file this fan-out's agents may not edit.
    /// The Agent K note in `REWRITE_PLAN.md` carries the two-line diff that makes
    /// it `self.font_browser` and drops this parameter.
    pub(crate) fn font_browser_pane(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        area: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        // Nothing here asks the application for anything: every control acts on
        // the importer the pane owns, and the one result that has to leave goes
        // through `take_import`. `commands` stays because the caption style
        // form's "Remove" button will need it.
        let _ = commands;
        // Split borrow rather than a method call: `draw_font_browser` needs the
        // widget set and the browser at once, and both are fields of `self`.
        let Shell {
            widgets,
            font_browser,
            ..
        } = self;
        draw_font_browser(widgets, d, input, font_browser, area);
    }
}

/// `draw_font_browser` (`lyrics_editor_ui.c:650-833`).
fn draw_font_browser(
    widgets: &mut Widgets,
    d: &mut RaylibDrawHandle<'_>,
    input: &ShellInput<'_>,
    browser: &mut FontBrowser,
    area: UiRect,
) {
    let font = input.fonts.ui();
    let track_present = input.workspace.current().is_some();
    // Polled here as well as from the application loop. The C polls once per
    // `plug_update`; doing it here too is idempotent and is what keeps the
    // browser live in a build whose `main.rs` has not yet taken the per-frame
    // call — see the note in `REWRITE_PLAN.md`.
    browser.poll(track_present);

    let allowed = browser.view.network_allowed();
    let Some(layout) = BrowserLayout::measure(area.x, area.y, area.width, area.height) else {
        // A control that does not fit is not drawn, and a sentence saying why
        // beats a truncated browser — and beats a blank region by more.
        widgets::draw_text(
            d,
            font,
            "Enlarge the window to browse caption faces.",
            area.x,
            area.y,
            15.0,
            color::ui_muted(),
        );
        return;
    };

    match browser.importer.panel(allowed) {
        FontImportPanel::Consent => return draw_font_consent(widgets, d, font, browser, area),
        panel @ (FontImportPanel::Loading
        | FontImportPanel::Fetching
        | FontImportPanel::Cancelling) => {
            return draw_font_progress(widgets, d, font, browser, area, panel)
        }
        FontImportPanel::Failed => {
            return draw_font_failure(widgets, d, font, browser, area, allowed)
        }
        FontImportPanel::Browsing => {}
    }

    // `is_none_or` is stable only since 1.82 and this workspace's MSRV is 1.80.
    if browser
        .importer
        .catalogue()
        .map_or(true, |list| list.is_empty())
    {
        widgets::draw_text(
            d,
            font,
            "No families are loaded.",
            area.x,
            area.y,
            15.0,
            color::ui_muted(),
        );
        return;
    }

    draw_search_field(widgets, d, font, browser, layout);
    let (shown, matched) = draw_family_list(widgets, d, font, browser, layout);
    draw_count(d, font, area, shown, matched);
    draw_action_row(widgets, d, font, browser, layout, allowed, track_present);
}

/// The search field and its keyboard (`lyrics_editor_ui.c:713-726`).
fn draw_search_field(
    widgets: &mut Widgets,
    d: &mut RaylibDrawHandle<'_>,
    font: &UiFonts,
    browser: &mut FontBrowser,
    layout: BrowserLayout,
) {
    let _ = widgets;
    let search = rect(layout.search);
    let scale = super::super::scale::UiScale::new(font.scale()).unwrap_or_default();
    let mouse = scale.mouse(d);
    if d.is_mouse_button_pressed(raylib::consts::MouseButton::MOUSE_BUTTON_LEFT) {
        // Focus follows the press, and a press anywhere else takes it away. The
        // field is not a widget with an id: it claims no press, so a click that
        // lands on it must not also be able to release a button underneath.
        browser.view.query_active = search.contains_point(mouse.x, mouse.y);
    }
    widgets::fill(d, search, color::ui_raised());
    d.draw_rectangle_lines_ex(
        widgets::rectangle(search),
        if browser.view.query_active { 2.0 } else { 1.0 },
        if browser.view.query_active {
            color::accent()
        } else {
            color::ui_rule()
        },
    );
    let empty = browser.view.query().is_empty();
    widgets::draw_text(
        d,
        font,
        if empty {
            "Search families"
        } else {
            browser.view.query()
        },
        search.x + 8.0,
        search.y + 6.0,
        15.0,
        if empty {
            color::ui_muted()
        } else {
            color::ui_ink()
        },
    );
    read_query_keys(d, &mut browser.view);
}

/// The scrolling family list (`lyrics_editor_ui.c:728-805`).
///
/// Returns `(shown, matched)`: how many rows this frame drew, and how many
/// families the query matched in total. The difference is what the count line
/// exists to state.
fn draw_family_list(
    widgets: &mut Widgets,
    d: &mut RaylibDrawHandle<'_>,
    font: &UiFonts,
    browser: &mut FontBrowser,
    layout: BrowserLayout,
) -> (usize, usize) {
    let list = rect(layout.list);
    let scale = super::super::scale::UiScale::new(font.scale()).unwrap_or_default();
    let mouse = scale.mouse(d);
    let wheel = d.get_mouse_wheel_move();
    let query = browser.view.query().to_string();
    let selected_family = browser.view.selected().map(str::to_string);

    let Some(catalogue) = browser.importer.catalogue() else {
        return (0, 0);
    };
    let matched = catalogue.filter(&query, 0, &mut []).matched;
    if list.contains_point(mouse.x, mouse.y) {
        browser.view.scroll(wheel);
    }
    browser.view.clamp_window(matched, layout.visible_rows);
    let first = browser.view.first();

    // Only the visible window is collected, so scrolling a catalogue of eighteen
    // hundred families never walks more than a screenful
    // (`lyrics_editor_ui.c:753-767`).
    let catalogue = browser.importer.catalogue().expect("checked above");
    let rows: Vec<(usize, &CatalogueEntry)> = catalogue
        .entries()
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.matches(&query))
        .skip(first)
        .take(layout.visible_rows)
        .collect();

    let mut chosen = None;
    for (row, (index, entry)) in rows.iter().enumerate() {
        let boundary = UiRect::new(
            list.x,
            list.y + row as f32 * font_state::BROWSER_ROW_HEIGHT,
            list.width,
            font_state::BROWSER_ROW_HEIGHT - 2.0,
        );
        let selected = selected_family.as_deref() == Some(entry.family.as_str());
        // Drawn by hand rather than through `text_button` because the row
        // carries two texts with different alignments — the family on the left,
        // its category and script coverage on the right — and a button label is
        // one centred string.
        let state = widgets.button(d, widgets::widget_id(ID, 64 + *index as u32), boundary);
        // The C tints a selected row with a bare `GetColor(0xE7ECFAFF)`
        // (`lyrics_editor_ui.c:779`) — a literal that is not in `ui_palette.h`
        // and is therefore invisible to the contrast suite that header exists
        // for. The palette's accent is used instead, with the white label the
        // rest of this shell pairs with it; that pair *is* contrast-checked.
        let background = if selected {
            color::accent()
        } else if state.hovered {
            color::track_button_hoverover()
        } else {
            color::ui_raised()
        };
        widgets::fill(d, boundary, background);
        widgets::draw_text(
            d,
            font,
            &entry.family,
            boundary.x + 8.0,
            boundary.y + 4.0,
            15.0,
            if selected {
                color::white()
            } else {
                color::ui_ink()
            },
        );
        let note = format!(
            "{}  -  {}",
            entry.category.display_name(),
            describe_scripts(entry.scripts)
        );
        let note_width = widgets::measure(font, &note, 12.0);
        // Drawn only where it fits, so a narrow pane loses the note rather than
        // printing it through the family name.
        let note_x = boundary.x + boundary.width - note_width - 8.0;
        if note_x > boundary.x + 160.0 {
            widgets::draw_text(
                d,
                font,
                &note,
                note_x,
                boundary.y + 6.0,
                12.0,
                if selected {
                    color::white()
                } else {
                    color::ui_muted()
                },
            );
        }
        if state.clicked {
            chosen = Some(entry.family.clone());
        }
    }
    let shown = rows.len();
    if let Some(family) = chosen {
        browser.view.select(&family);
    }
    (shown, matched)
}

/// "N of M", so a filtered list says how much it is not showing
/// (`lyrics_editor_ui.c:806-810`).
fn draw_count(
    d: &mut RaylibDrawHandle<'_>,
    font: &UiFonts,
    area: UiRect,
    shown: usize,
    matched: usize,
) {
    let count = format!("{shown} of {matched}");
    let width = widgets::measure(font, &count, 13.0);
    widgets::draw_text(
        d,
        font,
        &count,
        area.x + area.width - width,
        area.y + 8.0,
        13.0,
        color::ui_muted(),
    );
}

/// "Download and use", and the family it would fetch
/// (`lyrics_editor_ui.c:812-832`).
fn draw_action_row(
    widgets: &mut Widgets,
    d: &mut RaylibDrawHandle<'_>,
    font: &UiFonts,
    browser: &mut FontBrowser,
    layout: BrowserLayout,
    allowed: bool,
    track_present: bool,
) {
    let action = rect(layout.action);
    // Whether the selected family is still one this catalogue offers. The C
    // bounds-checks a stored *index* here (`:813-814`), which is the bug its own
    // header comment warns about; re-resolving the name is what that comment
    // asks for.
    let importable = browser
        .view
        .selected()
        .and_then(|family| browser.importer.catalogue().map(|list| list.find(family)))
        .flatten()
        .is_some();
    let pressed = widgets
        .text_button(
            d,
            font,
            widgets::widget_id(ID, 3),
            action,
            "Download and use",
            false,
            ButtonStyle::Neutral,
            Some(14.0),
        )
        .clicked;
    match browser.view.selected().filter(|_| importable) {
        None => widgets::draw_text(
            d,
            font,
            "Choose a family first.",
            action.x + action.width + 10.0,
            action.y + 8.0,
            13.0,
            color::ui_muted(),
        ),
        Some(family) => widgets::draw_text(
            d,
            font,
            family,
            action.x + action.width + 10.0,
            action.y + 8.0,
            13.0,
            color::ui_ink(),
        ),
    }
    if pressed && importable {
        // Consent, an idle job, a family and a track are all re-checked inside
        // `fetch`, which is where their sentences live. A press that cannot
        // start a download therefore explains itself in the failure body rather
        // than doing nothing.
        let family = browser.view.selected().unwrap_or_default().to_string();
        browser.importer.fetch(allowed, &family, track_present);
    }
}

/// `draw_font_consent` (`lyrics_editor_ui.c:617-648`).
///
/// The consequence is stated **before** the button that causes it, and what is
/// sent is named exactly. What leaves this machine is genuinely small, and
/// saying so is more useful than a vague warning.
fn draw_font_consent(
    widgets: &mut Widgets,
    d: &mut RaylibDrawHandle<'_>,
    font: &UiFonts,
    browser: &mut FontBrowser,
    area: UiRect,
) {
    widgets::draw_text(
        d,
        font,
        "Import a caption face from Google Fonts",
        area.x,
        area.y,
        16.0,
        color::ui_ink(),
    );
    const LINES: [&str; 6] = [
        "Musializer will contact fonts.google.com and fonts.gstatic.com to list",
        "and download faces, and raw.githubusercontent.com for the licence each",
        "face is distributed under.",
        "",
        "Only a family name is sent. No audio, lyrics, or project data leaves",
        "this machine. Musializer asks again next time it starts.",
    ];
    // The button is placed from the bottom and the explanation fills what is
    // left above it. A consent panel that keeps its prose and loses its button
    // is a question with no way to answer it.
    let allow = UiRect::new(area.x, area.y + area.height - 30.0, 200.0, 30.0);
    for (index, line) in LINES.iter().enumerate() {
        let y = area.y + 26.0 + index as f32 * 17.0;
        if y + 17.0 > allow.y - 6.0 {
            break;
        }
        widgets::draw_text(d, font, line, area.x, y, 13.0, color::ui_muted());
    }
    if widgets
        .text_button(
            d,
            font,
            widgets::widget_id(ID, 0),
            allow,
            "Allow and browse fonts",
            false,
            ButtonStyle::Neutral,
            Some(14.0),
        )
        .clicked
    {
        // Consent and the first request in one press, as the C does: somebody
        // who has just agreed to be contacted should not then have to ask again
        // for the thing they agreed to.
        browser.view.allow_network();
        browser.importer.browse(true);
    }
}

/// The three in-flight bodies (`lyrics_editor_ui.c:665-690`).
fn draw_font_progress(
    widgets: &mut Widgets,
    d: &mut RaylibDrawHandle<'_>,
    font: &UiFonts,
    browser: &mut FontBrowser,
    area: UiRect,
    panel: FontImportPanel,
) {
    let heading = match panel {
        FontImportPanel::Fetching => "Downloading the face...",
        FontImportPanel::Cancelling => "Stopping...",
        _ => "Fetching the family list...",
    };
    widgets::draw_text(d, font, heading, area.x, area.y, 16.0, color::ui_ink());
    let status = browser.importer.status().to_string();
    if !status.is_empty() {
        widgets::draw_text(
            d,
            font,
            &status,
            area.x,
            area.y + 24.0,
            13.0,
            color::ui_muted(),
        );
    }
    let cancel = UiRect::new(area.x, area.y + 52.0, 110.0, 30.0);
    // A cancel already in flight must not be offered again: the second press
    // would claim the id and do nothing visible.
    if panel != FontImportPanel::Cancelling
        && cancel.y + cancel.height <= area.y + area.height
        && widgets
            .text_button(
                d,
                font,
                widgets::widget_id(ID, 1),
                cancel,
                "Cancel",
                false,
                ButtonStyle::Neutral,
                Some(14.0),
            )
            .clicked
    {
        browser.importer.cancel();
    }
}

/// `FONT_IMPORT_PANEL_FAILED` (`lyrics_editor_ui.c:691-704`).
///
/// A failure is the one outcome that never clears itself, because the reason is
/// the point. It waits to be read, and offers the retry that would clear it.
fn draw_font_failure(
    widgets: &mut Widgets,
    d: &mut RaylibDrawHandle<'_>,
    font: &UiFonts,
    browser: &mut FontBrowser,
    area: UiRect,
    allowed: bool,
) {
    widgets::draw_text(
        d,
        font,
        "The font request did not finish",
        area.x,
        area.y,
        16.0,
        color::ui_ink(),
    );
    let status = browser.importer.status().to_string();
    widgets::draw_text(
        d,
        font,
        if status.is_empty() {
            "No further detail was reported."
        } else {
            &status
        },
        area.x,
        area.y + 24.0,
        13.0,
        color::ui_muted(),
    );
    let retry = UiRect::new(area.x, area.y + 52.0, 110.0, 30.0);
    if retry.y + retry.height <= area.y + area.height
        && widgets
            .text_button(
                d,
                font,
                widgets::widget_id(ID, 2),
                retry,
                "Try again",
                false,
                ButtonStyle::Neutral,
                Some(14.0),
            )
            .clicked
    {
        browser.importer.browse(allowed);
    }
}

/// The search field's keyboard (`font_query_input`,
/// `lyrics_editor_ui.c:599-615`).
///
/// The *policy* — what a character is allowed to be, what backspace does to a
/// multi-byte tail, what typing does to the scroll window — is `BrowserView`'s
/// and is tested there. This is only raylib's queue.
fn read_query_keys(d: &mut RaylibDrawHandle<'_>, view: &mut BrowserView) {
    use raylib::consts::KeyboardKey as Key;

    if !view.query_active {
        return;
    }
    if d.is_key_pressed(Key::KEY_BACKSPACE) {
        view.backspace();
    }
    if d.is_key_pressed(Key::KEY_ESCAPE) {
        view.query_active = false;
    }
    // A chord is a command, not text. Without this, Ctrl-C types a "c" — and the
    // queue still has to be drained either way, or the characters arrive on the
    // frame after the chord ends.
    let chord = d.is_key_down(Key::KEY_LEFT_CONTROL) || d.is_key_down(Key::KEY_RIGHT_CONTROL);
    while let Some(character) = d.get_char_pressed() {
        if !chord {
            view.type_char(character);
        }
    }
}

fn rect((x, y, width, height): (f32, f32, f32, f32)) -> UiRect {
    UiRect::new(x, y, width, height)
}

/// The stable name for a panel body, for [`FontBrowser::describe`].
fn panel_name(panel: FontImportPanel) -> &'static str {
    match panel {
        FontImportPanel::Consent => "consent",
        FontImportPanel::Loading => "loading",
        FontImportPanel::Browsing => "browsing",
        FontImportPanel::Fetching => "fetching",
        FontImportPanel::Cancelling => "cancelling",
        FontImportPanel::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory that removes itself, so a failed assertion does not
    /// leave a family list in `/tmp` for the next run to trip over.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "musializer-fontpane-{}-{label}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("scratch");
            Scratch(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_fresh_browser_asks_for_consent_and_has_nothing_loaded() {
        // The state `--ui-probe fonts=consent` photographs, and the state every
        // run starts in — including a run that imported a face last time.
        let browser = FontBrowser::new();
        let line = browser.describe();
        assert!(line.contains("consent=not asked"), "{line}");
        assert!(line.contains("families=0"), "{line}");
        assert!(line.contains("panel=consent"), "{line}");
    }

    #[test]
    fn a_local_family_list_reaches_the_browsing_body_without_granting_consent() {
        // `--ui-probe fonts=PATH`. A capture must reach the list without
        // contacting anybody, and must not answer the consent question as a side
        // effect — a probe that quietly granted it would make the consent panel
        // unreviewable, which is the failure this whole probe exists to prevent.
        let scratch = Scratch::new("catalogue");
        let path = scratch.0.join("families.tsv");
        std::fs::write(
            &path,
            "musializer.font-catalogue/v1\t2\n\
             Inter\tSans Serif\tlatin\n\
             Playfair Display\tSerif\tlatin,latin-ext\n",
        )
        .expect("write");

        let mut browser = FontBrowser::new();
        browser.load_catalogue_from_file(&path).expect("load");
        let line = browser.describe();
        assert!(line.contains("families=2"), "{line}");
        assert!(
            line.contains("consent=not asked"),
            "reading a file granted consent: {line}"
        );
        assert!(line.contains("panel=consent"), "{line}");

        browser.allow_network();
        assert!(browser.describe().contains("panel=browsing"));

        // A file that is not a catalogue is refused with a sentence rather than
        // emptying the list already loaded: a bad refresh must not break a
        // working picker.
        std::fs::write(&path, "not a catalogue\n").expect("write");
        let error = browser
            .load_catalogue_from_file(&path)
            .expect_err("refused");
        assert!(error.contains("header"), "{error}");
        assert!(browser.describe().contains("families=2"));
    }

    #[test]
    fn nothing_is_waiting_to_be_imported_until_a_download_verifies() {
        let mut browser = FontBrowser::new();
        assert!(browser.take_import().is_none());
        // Polling with no job is a no-op, which is what makes it safe to call
        // every frame from the application loop whether or not the pane is open.
        browser.poll(false);
        browser.poll(true);
        assert!(browser.take_import().is_none());
        browser.clear_selection();
    }

    #[test]
    fn the_panes_widget_ids_cannot_collide_with_the_shells() {
        // Ids are what claim a press, and a collision lets one widget release
        // another's — the failure `widgets`'s module comment records. The
        // namespace is checked rather than assumed, because this one is a local
        // constant rather than an entry in the shared `widgets::id`.
        let mut seen = std::collections::HashSet::new();
        for index in 0..4096u32 {
            assert!(seen.insert(widgets::widget_id(ID, index)));
        }
        for namespace in [
            widgets::id::TOOLBAR,
            widgets::id::SCENE_BROWSER,
            widgets::id::TRACKS,
            widgets::id::INSPECTOR,
            widgets::id::TIMELINE,
            widgets::id::WELCOME,
        ] {
            for index in 0..4096u32 {
                assert!(
                    !seen.contains(&widgets::widget_id(namespace, index)),
                    "the font pane collides with namespace {namespace}"
                );
            }
        }
    }

    #[test]
    fn every_panel_body_has_a_name_a_capture_can_assert_on() {
        // `describe` is the evidence line. A body with no name would report as
        // whatever the fallback arm said, and a capture would pass while showing
        // the wrong thing.
        let mut seen = std::collections::HashSet::new();
        for panel in [
            FontImportPanel::Consent,
            FontImportPanel::Loading,
            FontImportPanel::Browsing,
            FontImportPanel::Fetching,
            FontImportPanel::Cancelling,
            FontImportPanel::Failed,
        ] {
            assert!(seen.insert(panel_name(panel)), "{panel:?} shares a name");
        }
        assert_eq!(seen.len(), 6);
    }
}
