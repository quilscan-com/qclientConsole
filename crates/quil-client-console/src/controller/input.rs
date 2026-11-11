use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::event::InputEvent;

pub(crate) fn map_key(key: KeyEvent) -> Option<InputEvent> {
    match key.code {
        KeyCode::Char('c' | 'C') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(InputEvent::Quit)
        }
        KeyCode::Up => Some(InputEvent::Up),
        KeyCode::Down => Some(InputEvent::Down),
        KeyCode::PageUp => Some(InputEvent::PageUp),
        KeyCode::PageDown => Some(InputEvent::PageDown),
        KeyCode::Left => Some(InputEvent::Left),
        KeyCode::Right => Some(InputEvent::Right),
        KeyCode::Enter => Some(InputEvent::Enter),
        KeyCode::Tab => Some(InputEvent::Character('\t')),
        KeyCode::BackTab => Some(InputEvent::BackTab),
        KeyCode::Esc => Some(InputEvent::Back),
        KeyCode::Char('r' | 'R') => Some(InputEvent::Refresh),
        KeyCode::Char('n' | 'N') => Some(InputEvent::OpenConfigPath),
        KeyCode::Char('a' | 'A') => Some(InputEvent::Character('a')),
        KeyCode::Char('x' | 'X') => Some(InputEvent::Character('x')),
        KeyCode::Char('t' | 'T') => Some(InputEvent::Character('t')),
        KeyCode::Char('h' | 'H') => Some(InputEvent::Character('h')),
        KeyCode::Char('m' | 'M') => Some(InputEvent::Character('m')),
        KeyCode::Char(character @ ('c' | 'C' | 'k' | 'K')) => {
            Some(InputEvent::Character(character))
        }
        KeyCode::Char('?') => Some(InputEvent::Help),
        KeyCode::Char('q' | 'Q') => Some(InputEvent::Back),
        _ => None,
    }
}

pub(crate) fn map_form_key(key: KeyEvent) -> Option<InputEvent> {
    match key.code {
        KeyCode::Char('c' | 'C') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(InputEvent::Quit)
        }
        KeyCode::Char(character)
            if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
        {
            Some(InputEvent::Character(character))
        }
        KeyCode::Backspace => Some(InputEvent::Backspace),
        KeyCode::Up => Some(InputEvent::Up),
        KeyCode::Down => Some(InputEvent::Down),
        KeyCode::Left => Some(InputEvent::Left),
        KeyCode::Right => Some(InputEvent::Right),
        KeyCode::Tab => Some(InputEvent::Character('\t')),
        KeyCode::BackTab => Some(InputEvent::BackTab),
        KeyCode::Enter => Some(InputEvent::Enter),
        KeyCode::Esc => Some(InputEvent::Back),
        _ => None,
    }
}
