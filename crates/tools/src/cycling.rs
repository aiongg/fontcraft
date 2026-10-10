//! Tool letters and the toolbar buttons tools share (`tools.sharedButtonCycling`).
//!
//! Tools that share a button share a shortcut letter in [`TOOLS`]. The letter chooses the tool
//! of that button used last (the first one before any was used); `Shift` with the letter steps to
//! the next tool of the button, wrapping around. A letter that is already the active tool's keeps
//! it.

use crate::tool::{TOOLS, ToolInfo};

/// The tools a letter chooses among, in toolbar order.
fn with_letter(c: char) -> Vec<&'static ToolInfo> {
    let mut buf = [0u8; 4];
    let letter: &str = c.encode_utf8(&mut buf);
    TOOLS.iter().filter(|t| !t.shortcut.is_empty() && t.shortcut.eq_ignore_ascii_case(letter)).collect()
}

/// The tool the letter `c` (with `shift`) chooses while `active` is the chosen tool. `recent`
/// lists the tools chosen so far, the latest last. `None`: no tool has the letter, or `Shift`
/// was held and only one tool has it.
pub fn tool_for_letter(c: char, shift: bool, active: &str, recent: &[&'static str]) -> Option<&'static str> {
    let tools = with_letter(c.to_ascii_lowercase());
    let current =
        tools.iter().position(|t| t.id == active).or_else(|| recent.iter().rev().find_map(|id| tools.iter().position(|t| t.id == *id))).unwrap_or(0);
    if !shift {
        return tools.get(current).map(|t| t.id);
    }
    if tools.len() < 2 {
        return None;
    }
    tools.get((current + 1) % tools.len()).map(|t| t.id)
}

/// `recent` with `id` as the latest entry; each tool appears once.
pub fn remember(recent: &mut Vec<&'static str>, id: &'static str) {
    recent.retain(|r| *r != id);
    recent.push(id);
}
