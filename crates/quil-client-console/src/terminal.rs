use std::io::{self, Stdout};

use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

pub type ConsoleTerminal = Terminal<CrosstermBackend<Stdout>>;

pub struct TerminalGuard {
    terminal: Option<ConsoleTerminal>,
}

impl TerminalGuard {
    pub fn enter() -> anyhow::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        let terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => terminal,
            Err(error) => {
                restore_terminal();
                return Err(error.into());
            }
        };
        Ok(Self {
            terminal: Some(terminal),
        })
    }

    pub fn terminal_mut(&mut self) -> &mut ConsoleTerminal {
        self.terminal
            .as_mut()
            .expect("terminal is available while guard lives")
    }

    pub fn restore(&mut self) {
        if let Some(terminal) = self.terminal.as_mut() {
            let _ = disable_raw_mode();
            let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
            let _ = terminal.show_cursor();
        }
        self.terminal = None;
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

pub fn install_panic_restore_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous(info);
    }));
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_is_idempotent_without_terminal() {
        let mut guard = TerminalGuard { terminal: None };
        guard.restore();
        guard.restore();
        assert!(guard.terminal.is_none());
    }
}
