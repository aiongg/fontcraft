//! The hinting target: the rendering the programs are told about through `GETINFO`, and the
//! modes that change how some instructions behave. Mirrors skrifa 0.48's `outline::Target` and
//! its crate-private helpers (`src/outline/hint.rs`), which the interpreter reads.

/// The basic mode of a smooth (anti-aliased) target.
#[derive(Copy, Clone, PartialEq, Eq, Default, Debug)]
pub enum SmoothMode {
    #[default]
    Normal,
    Light,
    Lcd,
    VerticalLcd,
}

/// A hinting target.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Target {
    /// Aliased, monochrome rasterisation.
    Mono,
    /// Anti-aliased rasterisation.
    Smooth { mode: SmoothMode, symmetric_rendering: bool, preserve_linear_metrics: bool },
}

impl Default for Target {
    fn default() -> Self {
        Self::Smooth { mode: SmoothMode::Normal, symmetric_rendering: true, preserve_linear_metrics: false }
    }
}

impl Target {
    pub(crate) fn is_smooth(&self) -> bool {
        matches!(self, Self::Smooth { .. })
    }

    pub(crate) fn is_grayscale_cleartype(&self) -> bool {
        matches!(self, Self::Smooth { mode: SmoothMode::Normal | SmoothMode::Light, .. })
    }

    pub(crate) fn is_light(&self) -> bool {
        matches!(self, Self::Smooth { mode: SmoothMode::Light, .. })
    }

    pub(crate) fn is_lcd(&self) -> bool {
        matches!(self, Self::Smooth { mode: SmoothMode::Lcd, .. })
    }

    pub(crate) fn is_vertical_lcd(&self) -> bool {
        matches!(self, Self::Smooth { mode: SmoothMode::VerticalLcd, .. })
    }

    pub(crate) fn symmetric_rendering(&self) -> bool {
        matches!(self, Self::Smooth { symmetric_rendering: true, .. })
    }

    pub(crate) fn preserve_linear_metrics(&self) -> bool {
        matches!(self, Self::Smooth { preserve_linear_metrics: true, .. })
    }
}
