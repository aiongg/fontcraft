//! The Scale tool (`S`, `[[tool]] scale`).
//!
//! | Gesture | Result |
//! |---|---|
//! | click | places the reference point (`transform.setOrigin` with `point`) |
//! | drag | `transform.scale` of the selection about the reference point, or about the centre of the selection's box while none is placed |
//! | `Shift` while dragging | the same factor on both axes |
//! | `Escape` during a drag | cancel it |
//!
//! Each axis scales by the pointer's distance from the fixed point over the press's distance from
//! it, so the point under the press follows the pointer. An axis on which the press lies within
//! the drag threshold of the fixed point keeps its size. With nothing selected the drag scales
//! the whole layer, as the Palette does. The state machine is shared with the Rotate tool
//! ([`crate::bbox::Freehand`]).

use crate::bbox::{FreeKind, Freehand};
use crate::event::{Key, KeyEvent, Modifiers, PointerEvent};
use crate::output::{Output, Overlay};
use crate::tool::{Context, Tool};

/// The Scale tool.
#[derive(Clone, Debug)]
pub struct ScaleTool(Freehand);

impl Default for ScaleTool {
    fn default() -> Self {
        Self(Freehand::new(FreeKind::Scale))
    }
}

impl Tool for ScaleTool {
    fn id(&self) -> &'static str {
        "scale"
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        self.0.pointer(cx, ev)
    }

    fn key(&mut self, _cx: &Context<'_>, ev: &KeyEvent) -> Output {
        if ev.pressed && ev.key == Key::Escape && self.0.busy() {
            let mut out = self.0.cancel();
            out.handled = true;
            return out;
        }
        Output::default()
    }

    fn modifiers(&mut self, cx: &Context<'_>, mods: Modifiers) -> Output {
        self.0.modifiers(cx, mods)
    }

    fn cancel(&mut self) -> Output {
        self.0.cancel()
    }

    fn busy(&self) -> bool {
        self.0.busy()
    }

    fn overlays(&self) -> Vec<Overlay> {
        self.0.overlays()
    }
}
