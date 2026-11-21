//! A completion list fed by a lookup that may answer later: the field asks for the text before its caret, the caller
//! runs the lookup however it likes (a command, a query of the app layer) and hands the answer back, and the list opens
//! once the answer matches what the field still shows.
//!
//! Meant for craft-ui-contract, `src/primitives/text_input.rs`, beside the `Completer` trait: craft's text part asks its
//! completer every pass while the user types and takes the candidates at once, which suits a list the view already
//! holds. This piece is a `Completer` whose state outlives the pass (kept in the context under an id, as craft keeps a
//! `Bind::kept` value), so an answer that comes a pass or more later still reaches the field.

use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};

use craft::ui::primitives::{Completer, Completion};
use egui::{Context, Id};

/// Most candidates one answer holds; the rest are dropped.
pub const MAX_COMPLETIONS: usize = 1_000;

/// What the field asked about: its text and the caret's byte offset in it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompletionQuery {
    pub text: String,
    pub caret: usize,
}

/// The answer to a query: the bytes of the text the candidates replace, and the candidates.
#[derive(Clone, Debug, Default)]
pub struct CompletionAnswer {
    pub range: Range<usize>,
    pub items: Vec<Completion>,
}

#[derive(Default)]
struct State {
    /// The query the field asked last.
    asked: Option<CompletionQuery>,
    /// The asked query, until the caller takes it.
    waiting: Option<CompletionQuery>,
    /// The last answer and the query it answers.
    answer: Option<(CompletionQuery, CompletionAnswer)>,
}

/// A completer whose candidates come from a lookup that may answer later. Clones share their state.
#[derive(Clone)]
pub struct AsyncCompleter {
    state: Arc<Mutex<State>>,
    ctx: Context,
}

impl AsyncCompleter {
    /// The completer kept under `id`: every pass that asks for it with the same id gets the same state.
    pub fn kept(ctx: &Context, id: Id) -> Self {
        let state = ctx.data_mut(|data| data.get_temp_mut_or_insert_with(id, || Arc::new(Mutex::new(State::default()))).clone());
        AsyncCompleter { state, ctx: ctx.clone() }
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut state)
    }

    /// The query the field asked since the last call, for the caller to look up; each query is handed out once.
    pub fn take_query(&self) -> Option<CompletionQuery> {
        self.with(|state| state.waiting.take())
    }

    /// Hands in the answer to `query`. The list shows it while the field's text and caret are still the query's; an
    /// answer to a query the field has moved past is kept but never shown. Asks for a repaint, so an answer that
    /// arrives between passes shows.
    pub fn answer(&self, query: CompletionQuery, mut answer: CompletionAnswer) {
        answer.items.truncate(MAX_COMPLETIONS);
        let changed = self.with(|state| {
            let same = state.answer.as_ref().is_some_and(|(q, a)| *q == query && a.range == answer.range && a.items.len() == answer.items.len());
            state.answer = Some((query, answer));
            !same
        });
        if changed {
            self.ctx.request_repaint();
        }
    }
}

impl Completer for AsyncCompleter {
    fn complete(&self, text: &str, caret: usize, out: &mut Vec<Completion>) -> Option<Range<usize>> {
        self.with(|state| {
            let query = CompletionQuery { text: text.to_owned(), caret };
            if let Some((answered, answer)) = &state.answer
                && *answered == query
            {
                // The range must lie on the text's character boundaries, or accepting a candidate would split one.
                let range = answer.range.clone();
                if range.start > range.end || text.get(range.clone()).is_none() {
                    return None;
                }
                out.extend(answer.items.iter().take(MAX_COMPLETIONS).cloned());
                return Some(range);
            }
            if state.asked.as_ref() != Some(&query) {
                state.waiting = Some(query.clone());
                state.asked = Some(query);
            }
            None
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A query is handed out once; its answer completes the same text and caret only, and a range that splits a
    /// character is refused.
    #[test]
    fn an_answer_completes_only_its_own_query() {
        let ctx = Context::default();
        let completer = AsyncCompleter::kept(&ctx, Id::new("names"));
        let mut out = Vec::new();
        assert_eq!(completer.complete("sub A", 5, &mut out), None);
        assert_eq!(completer.complete("sub A", 5, &mut out), None);
        let query = completer.take_query().expect("the field asked");
        assert_eq!(query, CompletionQuery { text: "sub A".into(), caret: 5 });
        assert_eq!(completer.take_query(), None, "a query is handed out once");
        // Kept under its id: the next pass's completer sees the answer.
        AsyncCompleter::kept(&ctx, Id::new("names"))
            .answer(query, CompletionAnswer { range: 4..5, items: vec![Completion::new("A.alt", "A.alt"), Completion::new("Aacute", "Aacute")] });
        assert_eq!(completer.complete("sub A", 5, &mut out), Some(4..5));
        assert_eq!(out.len(), 2);
        out.clear();
        assert_eq!(completer.complete("sub Aa", 6, &mut out), None, "the field moved past the answer");
        assert!(out.is_empty());
        let query = completer.take_query().expect("the new text is asked");
        completer.answer(query, CompletionAnswer { range: 5..6, items: vec![Completion::new("x", "x")] });
        assert_eq!(completer.complete("sub é", 6, &mut out), None);
        let query = completer.take_query().expect("asked");
        completer.answer(query, CompletionAnswer { range: 5..6, items: vec![Completion::new("x", "x")] });
        assert_eq!(completer.complete("sub é", 6, &mut out), None, "5 is inside é");
    }
}
