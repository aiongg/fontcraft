//! The app's palette windows while a modal dialog is open in the main window.
//!
//! Craft opens a floating dock window as a palette OS window, which stands above every window of the app while the app
//! is active (`WindowLevel::Palette`), and shows an app-modal dialog as a layer of the main window. A palette over the
//! dialog hides it, and craft lowers no palette while a modal is open (decision 0009, "Craft bugs"). While a modal layer
//! is open in the main window, [`keep_modal_on_top`] puts the app's other windows at the normal level and raises the
//! main window; when the modal closes they get their palette level back. Craft's own modal gate already keeps their
//! input from them meanwhile.
//!
//! Every window of the app other than the main one is a dock palette: the app opens no other OS window.

use craft::ui::runtime::windows::{WindowAction, WindowHost, WindowLevel};
use egui::{Context, Id, ViewportId};

/// What [`plan`] asks of the windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The window goes to the normal level.
    Lower(ViewportId),
    /// The window gets its palette level back.
    Restore(ViewportId),
    /// The main window comes to the front, over the lowered windows.
    RaiseMain,
}

/// The steps that take the windows from `lowered` (the ones at the normal level now) to where a modal that is `open`
/// needs them, given the open windows `others` (every viewport but the main one); and the windows lowered after them.
pub fn plan(open: bool, others: &[ViewportId], lowered: &[ViewportId]) -> (Vec<Step>, Vec<ViewportId>) {
    let still: Vec<ViewportId> = lowered.iter().filter(|viewport| others.contains(viewport)).copied().collect();
    if !open {
        return (still.into_iter().map(Step::Restore).collect(), Vec::new());
    }
    let fresh: Vec<ViewportId> = others.iter().filter(|viewport| !still.contains(viewport)).copied().collect();
    let mut steps: Vec<Step> = fresh.iter().copied().map(Step::Lower).collect();
    if !fresh.is_empty() {
        steps.push(Step::RaiseMain);
    }
    (steps, still.into_iter().chain(fresh).collect())
}

/// Lowers the app's other windows while a modal layer of the main window is open (as the last pass left it), raises
/// the main window, and restores them once it closes. Call it once per pass of the main window. Returns the steps it
/// took.
pub fn keep_modal_on_top(ctx: &Context) -> Vec<Step> {
    if ctx.viewport_id() != ViewportId::ROOT {
        return Vec::new();
    }
    let open = ctx.memory(|memory| memory.top_modal_layer().is_some());
    let others: Vec<ViewportId> = ctx.input(|input| input.raw.viewports.keys().copied().filter(|viewport| *viewport != ViewportId::ROOT).collect());
    let key = Id::unique("craft_ext.windows.loweredPalettes");
    let lowered: Vec<ViewportId> = ctx.data(|data| data.get_temp(key)).unwrap_or_default();
    let (steps, now) = plan(open, &others, &lowered);
    let host = WindowHost::of(ctx);
    for step in &steps {
        let _ = match *step {
            Step::Lower(viewport) => host.act(viewport, WindowAction::SetLevel(WindowLevel::Normal)),
            Step::Restore(viewport) => host.act(viewport, WindowAction::SetLevel(WindowLevel::Palette)),
            Step::RaiseMain => host.act(ViewportId::ROOT, WindowAction::Focus),
        };
    }
    ctx.data_mut(|data| data.insert_temp(key, now));
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A modal lowers each open palette once and raises the main window once; its close restores the palettes still
    /// open; a palette that opens under the modal is lowered too.
    #[test]
    fn palettes_go_down_under_a_modal_and_come_back_after_it() {
        let (a, b) = (ViewportId::from_hash_of("a"), ViewportId::from_hash_of("b"));
        let (steps, lowered) = plan(true, &[a], &[]);
        assert_eq!(steps, vec![Step::Lower(a), Step::RaiseMain]);
        assert_eq!(plan(true, &[a], &lowered), (Vec::new(), vec![a]), "nothing is sent again while the modal stays");
        let (steps, lowered) = plan(true, &[a, b], &lowered);
        assert_eq!((steps, lowered.clone()), (vec![Step::Lower(b), Step::RaiseMain], vec![a, b]));
        assert_eq!(plan(false, &[b], &lowered), (vec![Step::Restore(b)], Vec::new()), "a closed window is not restored");
        assert_eq!(plan(false, &[a, b], &[]), (Vec::new(), Vec::new()));
        assert_eq!(plan(true, &[], &[]), (Vec::new(), Vec::new()), "no palette, no raise");
    }
}
