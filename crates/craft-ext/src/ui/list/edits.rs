//! Edit commands reaching a focused collection: Copy, Cut, Paste and Select All fired while a `List` holds the
//! keyboard focus come back to the list's view as events, instead of running the app's Edit command on the document.
//!
//! Meant for craft-ui-contract: `src/contract/collection/` (a `CollectionEvent::Edit(EditCommand)` and an `edits`
//! setting on `CollectionProps`) with the collection's `KeyUse` taking those commands (`KeyUse::with_edits`), so the
//! key strip delivers them as `Intent::Edit` the way it does for a text field. A collection's `KeyUse` is fixed inside
//! craft, so the chord still fires the app's command; this stand-in lets the shell hand that command to the focused
//! list instead:
//!
//! 1. The list's view calls [`CollectionEdits::edit_commands`] after showing the list, every pass. While the list has
//!    the keyboard focus that records a claim naming the commands it takes, and it returns the commands handed to it.
//! 2. The shell, when the person fires an edit command, calls [`deliver`]. When a live claim takes the command it is
//!    queued for that list, a repaint is asked for, and `deliver` returns `true`: the shell then does not run its own
//!    command.
//!
//! The commands reach the view one pass after they were fired.

use craft::ui::contract::CollectionResponse;
use egui::{Context, Id, Response, Ui};

pub use craft::behavior::EditCommand;

/// Most commands queued for one collection; a flood past it is dropped.
const MAX_QUEUED: usize = 16;

/// The edit commands a collection asks for while it holds the keyboard focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditClaim {
    /// The collection's id: its response's.
    pub id: Id,
    /// The commands it takes.
    pub edits: Vec<EditCommand>,
    /// The pass it was made in.
    pass: u64,
}

impl EditClaim {
    /// Whether the claim takes `command`.
    pub fn takes(&self, command: EditCommand) -> bool {
        self.edits.contains(&command)
    }
}

fn claim_key() -> Id {
    Id::new("craft_ext.list.editClaim")
}

fn queue_key(id: Id) -> Id {
    id.with("craft_ext.list.editQueue")
}

/// Records that the collection of `response` takes `edits` while it has the keyboard focus. Nothing is recorded
/// without the focus.
pub fn claim_edits(ui: &Ui, response: &Response, edits: &[EditCommand]) {
    let ctx = ui.ctx();
    if edits.is_empty() || !ctx.memory(|m| m.has_focus(response.id)) {
        return;
    }
    let claim = EditClaim { id: response.id, edits: edits.to_vec(), pass: ctx.cumulative_pass_nr() };
    ctx.data_mut(|d| d.insert_temp(claim_key(), claim));
}

/// The claim of the collection that holds the keyboard focus now: one made in this pass or the one before, by a
/// collection that still has the focus.
pub fn focused_claim(ctx: &Context) -> Option<EditClaim> {
    let claim = ctx.data(|d| d.get_temp::<EditClaim>(claim_key()))?;
    let live = claim.pass.saturating_add(1) >= ctx.cumulative_pass_nr();
    (live && ctx.memory(|m| m.has_focus(claim.id))).then_some(claim)
}

/// Hands `command` to the focused collection when its claim takes it; `true` when it did, and the caller then runs
/// nothing of its own for the command.
pub fn deliver(ctx: &Context, command: EditCommand) -> bool {
    let Some(claim) = focused_claim(ctx).filter(|claim| claim.takes(command)) else { return false };
    ctx.data_mut(|d| {
        let queue = d.get_temp_mut_or_default::<Vec<EditCommand>>(queue_key(claim.id));
        if queue.len() < MAX_QUEUED {
            queue.push(command);
        }
    });
    ctx.request_repaint();
    true
}

/// The commands handed to the collection `id` since its last pass, in the order they were fired.
pub fn take_edits(ui: &Ui, id: Id) -> Vec<EditCommand> {
    ui.ctx().data_mut(|d| d.remove_temp::<Vec<EditCommand>>(queue_key(id))).unwrap_or_default()
}

/// Edit commands as events of a shown collection.
pub trait CollectionEdits {
    /// The commands handed to the collection since its last pass; and, while it has the keyboard focus, the claim
    /// that it takes `edits` from now on.
    fn edit_commands(&self, ui: &Ui, edits: &[EditCommand]) -> Vec<EditCommand>;
}

impl<K, E> CollectionEdits for CollectionResponse<K, E> {
    fn edit_commands(&self, ui: &Ui, edits: &[EditCommand]) -> Vec<EditCommand> {
        let handed = take_edits(ui, self.response.id);
        claim_edits(ui, &self.response, edits);
        handed
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::behavior::keyed;
    use craft::ui::contract::ShowCollection as _;
    use craft::ui::list::{List, ListRow};
    use craft::ui::testing::Fixture;

    use super::*;

    /// A focused list gets Copy and Paste handed to it, once each and in order; Cut, which it does not take, stays
    /// with the shell, and so does everything once the list has lost the focus.
    #[test]
    fn a_focused_list_takes_the_edit_commands_it_claims() {
        let got = Rc::new(RefCell::new(Vec::new()));
        let id = Rc::new(RefCell::new(None));
        let (seen, list_id) = (Rc::clone(&got), Rc::clone(&id));
        let mut fixture = Fixture::new(move |ui| {
            let positions: Vec<usize> = vec![0, 1];
            let source = keyed(&positions, |at| *at);
            let out = List::<usize>::new("Entries")
                .header(false)
                .natural_height(true)
                .show_items(ui, &source, &mut |at, icx| ListRow::new(format!("entry {at}")).state(icx.row.clone()));
            seen.borrow_mut().extend(out.edit_commands(ui, &[EditCommand::Copy, EditCommand::Paste]));
            *list_id.borrow_mut() = Some(out.response.id);
        });
        fixture.settle();
        let list = id.borrow().expect("the list showed");
        fixture.ctx().memory_mut(|m| m.request_focus(list));
        fixture.settle();
        let ctx = fixture.ctx().clone();
        assert!(deliver(&ctx, EditCommand::Copy));
        assert!(deliver(&ctx, EditCommand::Paste));
        assert!(!deliver(&ctx, EditCommand::Cut));
        fixture.settle();
        assert_eq!(*got.borrow(), vec![EditCommand::Copy, EditCommand::Paste]);

        fixture.ctx().memory_mut(|m| m.surrender_focus(list));
        fixture.settle();
        assert!(!deliver(&ctx, EditCommand::Copy));
    }
}
