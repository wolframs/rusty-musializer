//! A pseudo-terminal running a shell, for the dock's terminal tab (WX-2).
//!
//! The VT state machine, the scrollback and the PTY are alacritty's own
//! terminal core (`alacritty_terminal`, Apache-2.0). Thirty years of escape
//! code edge cases are not something to rediscover. Drawing the grid is the
//! application's job (`ui/dock/terminal_view.rs`), so this module has no
//! toolkit in it. It owns three things:
//!
//! - the session: spawning the shell, writing input to it, resizing it, and
//!   ending it so that nothing it started outlives the window;
//! - the replies a terminal owes its program (cursor position, colour and size
//!   queries), which alacritty hands back as events rather than writing itself;
//! - the colour palette, because a colour query must answer with the same
//!   colour the view draws.
//!
//! **Ending a session.** The shell is a session leader (alacritty `setsid`s
//! it), so its pid is also its process group id. Closing sends `SIGHUP` to
//! the whole group, then asks the PTY thread to stop. The thread drops the
//! PTY, which sends the shell its own `SIGHUP` and reaps it. Nothing joins on
//! the UI thread, so a shell that ignores the hangup cannot freeze the window.
//! If the application exits first, the kernel closes the PTY master, and the
//! session's foreground group gets the same hangup.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

pub use alacritty_terminal;

use crate::process::process_group::{signal_group_or_process, Signal};

/// Lines kept above the screen. Alacritty's own default.
pub const SCROLLBACK_LINES: usize = 10_000;

/// What to run, where, and with which extra environment.
#[derive(Clone, Debug, Default)]
pub struct Spawn {
    /// `None` runs the user's login shell (`$SHELL`, then `passwd`).
    pub program: Option<(String, Vec<String>)>,
    pub working_directory: Option<PathBuf>,
    pub env: HashMap<String, String>,
}

/// The grid's size in cells, and one cell's size in pixels.
///
/// The pixel size is reported to programs that ask (`TIOCGWINSZ`, `CSI 14 t`)
/// and is otherwise unused; nothing draws from it here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub columns: u16,
    pub lines: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl GridSize {
    fn window(self) -> WindowSize {
        WindowSize {
            num_lines: self.lines.max(1),
            num_cols: self.columns.max(2),
            cell_width: self.cell_width.max(1),
            cell_height: self.cell_height.max(1),
        }
    }
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }
    fn screen_lines(&self) -> usize {
        usize::from(self.lines.max(1))
    }
    fn columns(&self) -> usize {
        usize::from(self.columns.max(2))
    }
}

/// What happened since the last [`Session::poll`], for the view to act on.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Activity {
    /// Something changed on screen.
    pub redraw: bool,
    pub bell: bool,
    /// The program asked to put this on the clipboard (OSC 52).
    pub clipboard: Option<String>,
}

/// The listener alacritty's PTY thread reports through. A channel rather than
/// a callback, so every reply is written from the UI thread in order.
#[derive(Clone)]
pub struct Listener(Sender<Event>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.0.send(event);
    }
}

pub type SharedTerm = Arc<FairMutex<Term<Listener>>>;

/// One running shell.
pub struct Session {
    term: SharedTerm,
    sender: EventLoopSender,
    events: Receiver<Event>,
    pid: u32,
    size: GridSize,
    title: Option<String>,
    exit: Option<Option<i32>>,
    closed: bool,
}

impl Session {
    /// Starts the program on a new PTY of `size`.
    pub fn spawn(spawn: &Spawn, size: GridSize) -> std::io::Result<Self> {
        let options = tty::Options {
            shell: spawn
                .program
                .as_ref()
                .map(|(program, args)| tty::Shell::new(program.clone(), args.clone())),
            working_directory: spawn.working_directory.clone(),
            drain_on_exit: true,
            env: spawn.env.clone(),
        };
        let pty = tty::new(&options, size.window(), 0)?;
        let pid = pty.child().id();
        let (tx, rx) = mpsc::channel();
        let listener = Listener(tx);
        let config = Config {
            scrolling_history: SCROLLBACK_LINES,
            ..Config::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, listener.clone())));
        let event_loop = EventLoop::new(Arc::clone(&term), listener, pty, true, false)?;
        let sender = event_loop.channel();
        // Detached on purpose; see the module comment on ending a session.
        let _ = event_loop.spawn();
        Ok(Self {
            term,
            sender,
            events: rx,
            pid,
            size,
            title: None,
            exit: None,
            closed: false,
        })
    }

    #[must_use]
    pub fn term(&self) -> &SharedTerm {
        &self.term
    }

    #[must_use]
    pub fn pid(&self) -> u32 {
        self.pid
    }

    #[must_use]
    pub fn size(&self) -> GridSize {
        self.size
    }

    /// The title the program last set (OSC 0/2), if any.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// `Some(code)` once the shell has exited; the inner `None` is a signal.
    #[must_use]
    pub fn exit(&self) -> Option<Option<i32>> {
        self.exit
    }

    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        if !self.closed && self.exit.is_none() {
            let _ = self.sender.send(Msg::Input(bytes.into()));
        }
    }

    /// Resizes the grid and tells the program (`SIGWINCH`). A no-op when the
    /// size has not changed, so it is safe to call every frame.
    pub fn resize(&mut self, size: GridSize) {
        if size == self.size {
            return;
        }
        self.size = size;
        self.term.lock().resize(size);
        let _ = self.sender.send(Msg::Resize(size.window()));
    }

    /// Drains what the PTY thread reported, answering the queries a terminal
    /// is expected to answer.
    pub fn poll(&mut self) -> Activity {
        let mut activity = Activity::default();
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Wakeup | Event::MouseCursorDirty | Event::CursorBlinkingChange => {
                    activity.redraw = true;
                }
                Event::Title(title) => self.title = Some(title),
                Event::ResetTitle => self.title = None,
                Event::Bell => activity.bell = true,
                Event::PtyWrite(text) => self.write(text.into_bytes()),
                Event::ClipboardStore(_, text) => activity.clipboard = Some(text),
                // Reading the clipboard on a program's request is how a
                // malicious escape sequence exfiltrates it. Alacritty's own
                // default is to refuse, and so does this.
                Event::ClipboardLoad(..) => {}
                Event::ColorRequest(index, format) => {
                    let rgb =
                        self.term.lock().colors()[index].unwrap_or_else(|| default_rgb(index));
                    self.write(format(rgb).into_bytes());
                }
                Event::TextAreaSizeRequest(format) => {
                    self.write(format(self.size.window()).into_bytes());
                }
                Event::ChildExit(status) => {
                    self.exit = Some(status.code());
                    activity.redraw = true;
                }
                Event::Exit => {
                    if self.exit.is_none() {
                        self.exit = Some(None);
                    }
                    activity.redraw = true;
                }
            }
        }
        activity
    }

    /// Hangs up on the shell and everything in its process group.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        if self.exit.is_none() {
            let _ = signal_group_or_process(self.pid, Signal::Hangup);
        }
        let _ = self.sender.send(Msg::Shutdown);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}

/// The palette: the sixteen ANSI colours tuned to the application's dark
/// surfaces, the standard xterm 6x6x6 cube and grey ramp, then the named
/// slots alacritty keeps after index 255.
#[must_use]
pub fn default_rgb(index: usize) -> Rgb {
    const ANSI: [(u8, u8, u8); 16] = [
        (0x1b, 0x1e, 0x24), // black
        (0xd0, 0x6f, 0x63), // red
        (0x8f, 0xb8, 0x7c), // green
        (0xe3, 0xb5, 0x5a), // yellow
        (0x6f, 0x96, 0xc9), // blue
        (0xb4, 0x82, 0xc2), // magenta
        (0x6c, 0xb5, 0xb0), // cyan
        (0xc9, 0xc6, 0xbe), // white
        (0x5b, 0x60, 0x6a), // bright black
        (0xe8, 0x8a, 0x7d), // bright red
        (0xa9, 0xd3, 0x95), // bright green
        (0xf0, 0xc8, 0x75), // bright yellow
        (0x8c, 0xb1, 0xe0), // bright blue
        (0xcc, 0x9e, 0xd8), // bright magenta
        (0x8a, 0xd0, 0xca), // bright cyan
        (0xee, 0xeb, 0xe3), // bright white
    ];
    let rgb = |(r, g, b): (u8, u8, u8)| Rgb { r, g, b };
    match index {
        0..=15 => rgb(ANSI[index]),
        16..=231 => {
            let n = index - 16;
            let level = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
            rgb((level(n / 36), level((n / 6) % 6), level(n % 6)))
        }
        232..=255 => {
            let v = (8 + (index - 232) * 10) as u8;
            rgb((v, v, v))
        }
        i if i == NamedColor::Foreground as usize => rgb((0xdc, 0xd9, 0xd0)),
        i if i == NamedColor::Background as usize => rgb((0x0b, 0x0c, 0x0e)),
        i if i == NamedColor::Cursor as usize => rgb((0xf0, 0xb3, 0x47)),
        i if i == NamedColor::BrightForeground as usize => rgb((0xf4, 0xf1, 0xe9)),
        i if i == NamedColor::DimForeground as usize => rgb((0x91, 0x8f, 0x89)),
        // DimBlack..=DimWhite, in NamedColor's order after Cursor.
        i if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&i) => {
            let base = default_rgb(i - NamedColor::DimBlack as usize);
            dim(base)
        }
        _ => rgb((0xdc, 0xd9, 0xd0)),
    }
}

/// Two thirds of the way to black; what `SGR 2` (faint) means here.
#[must_use]
pub fn dim(rgb: Rgb) -> Rgb {
    let f = |v: u8| ((u16::from(v) * 2) / 3) as u8;
    Rgb {
        r: f(rgb.r),
        g: f(rgb.g),
        b: f(rgb.b),
    }
}

/// Resolves a cell colour against the program's overrides (OSC 4/10/11) and
/// then the palette.
#[must_use]
pub fn resolve(color: Color, overrides: &alacritty_terminal::term::color::Colors) -> Rgb {
    match color {
        Color::Spec(rgb) => rgb,
        Color::Named(named) => {
            let index = named as usize;
            overrides[index].unwrap_or_else(|| default_rgb(index))
        }
        Color::Indexed(index) => {
            let index = usize::from(index);
            overrides[index].unwrap_or_else(|| default_rgb(index))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn screen_text(session: &Session) -> String {
        let term = session.term().lock();
        let grid = term.grid();
        let mut out = String::new();
        for line in 0..grid.screen_lines() {
            let row = &grid[alacritty_terminal::index::Line(line as i32)];
            for column in 0..grid.columns() {
                out.push(row[alacritty_terminal::index::Column(column)].c);
            }
            out.push('\n');
        }
        out
    }

    fn wait_for(session: &mut Session, needle: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            session.poll();
            if screen_text(session).contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn size() -> GridSize {
        GridSize {
            columns: 60,
            lines: 8,
            cell_width: 8,
            cell_height: 16,
        }
    }

    #[test]
    fn a_program_draws_into_the_grid_with_its_environment_and_directory() {
        let dir = std::env::temp_dir();
        let mut spawn = Spawn {
            program: Some((
                "/bin/sh".into(),
                vec![
                    "-c".into(),
                    "printf 'dock:%s:%s\\n' \"$MZ_DOCK_TEST\" \"$(pwd)\"; sleep 5".into(),
                ],
            )),
            working_directory: Some(dir.clone()),
            env: HashMap::new(),
        };
        spawn.env.insert("MZ_DOCK_TEST".into(), "hello".into());
        let mut session = Session::spawn(&spawn, size()).expect("spawn");
        let expected = format!("dock:hello:{}", dir.canonicalize().unwrap().display());
        assert!(
            wait_for(&mut session, &expected),
            "screen never showed {expected:?}:\n{}",
            screen_text(&session)
        );
    }

    #[test]
    fn input_reaches_the_program_and_a_resize_reaches_it_too() {
        let spawn = Spawn {
            program: Some(("/bin/sh".into(), vec![])),
            ..Spawn::default()
        };
        let mut session = Session::spawn(&spawn, size()).expect("spawn");
        session.write(b"echo typed-$((6*7))\n".to_vec());
        assert!(
            wait_for(&mut session, "typed-42"),
            "{}",
            screen_text(&session)
        );
        session.resize(GridSize {
            columns: 70,
            ..size()
        });
        session.write(b"stty size\n".to_vec());
        assert!(wait_for(&mut session, "8 70"), "{}", screen_text(&session));
    }

    #[test]
    fn closing_hangs_up_on_the_whole_process_group() {
        // A child in the shell's group that would otherwise outlive it: the
        // shape of an agent CLI started from the dock.
        let marker = std::env::temp_dir().join(format!("mz-dock-hup-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let script = format!(
            "sleep 30 & echo $! > {}; echo started; wait",
            marker.display()
        );
        let spawn = Spawn {
            program: Some(("/bin/sh".into(), vec!["-c".into(), script])),
            ..Spawn::default()
        };
        let mut session = Session::spawn(&spawn, size()).expect("spawn");
        assert!(wait_for(&mut session, "started"));
        let child: i32 = std::fs::read_to_string(&marker)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        drop(session);
        let deadline = Instant::now() + Duration::from_secs(5);
        let alive = |pid: i32| {
            std::path::Path::new(&format!("/proc/{pid}")).exists()
                && !std::fs::read_to_string(format!("/proc/{pid}/stat"))
                    .unwrap_or_default()
                    .contains(") Z ")
        };
        while alive(child) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = std::fs::remove_file(&marker);
        assert!(!alive(child), "the background child survived the hangup");
    }

    #[test]
    fn the_palette_answers_every_index_alacritty_can_ask_for() {
        for index in 0..alacritty_terminal::term::color::COUNT {
            let _ = default_rgb(index);
        }
        assert_eq!(
            default_rgb(NamedColor::DimRed as usize),
            dim(default_rgb(1))
        );
        // The cube's corners are the xterm values, which programs assume.
        assert_eq!(default_rgb(16), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(
            default_rgb(231),
            Rgb {
                r: 255,
                g: 255,
                b: 255
            }
        );
        assert_eq!(default_rgb(232), Rgb { r: 8, g: 8, b: 8 });
    }
}
