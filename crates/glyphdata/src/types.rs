//! The closed vocabularies of glyph data: category, subcategory, case and direction.
//!
//! Each value has the spelling the data files use ([`Category::as_str`]) and parses from it.
//! Scripts are an open set of lowercase ids (`latin`, `arabic`, …) and stay strings.

use std::fmt;

macro_rules! vocabulary {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $text:literal,)* }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $($variant,)*
        }

        impl $name {
            /// Every value, in declaration order (the order of the binary tables).
            pub const ALL: &'static [$name] = &[$($name::$variant,)*];

            /// The spelling used in glyph data files.
            pub fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $text,)*
                }
            }

            /// Parses the spelling used in glyph data files; `None` for anything else.
            pub fn parse(text: &str) -> Option<$name> {
                match text {
                    $($text => Some($name::$variant),)*
                    _ => None,
                }
            }

            /// Position in [`Self::ALL`], as stored in the binary tables.
            #[cfg(feature = "build")]
            pub(crate) fn index(self) -> u8 {
                self as u8
            }

            pub(crate) fn from_index(i: u8) -> Option<$name> {
                Self::ALL.get(usize::from(i)).copied()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

vocabulary! {
    /// Top-level class of a glyph. The published data uses the first six; readers also accept
    /// `Space` and `Other`.
    Category {
        Letter = "Letter",
        Mark = "Mark",
        Number = "Number",
        Punctuation = "Punctuation",
        Separator = "Separator",
        Symbol = "Symbol",
        Space = "Space",
        Other = "Other",
    }
}

vocabulary! {
    /// Finer class of a glyph. The last seven are accepted by readers but absent from the
    /// published data.
    SubCategory {
        Arrow = "Arrow",
        Compatibility = "Compatibility",
        Currency = "Currency",
        Dash = "Dash",
        DecimalDigit = "Decimal Digit",
        Emoji = "Emoji",
        Enclosing = "Enclosing",
        Format = "Format",
        Fraction = "Fraction",
        Geometry = "Geometry",
        Halfform = "Halfform",
        Jamo = "Jamo",
        Letter = "Letter",
        Ligature = "Ligature",
        Math = "Math",
        Matra = "Matra",
        Modifier = "Modifier",
        Nonspacing = "Nonspacing",
        Parenthesis = "Parenthesis",
        Quote = "Quote",
        Radical = "Radical",
        Small = "Small",
        Space = "Space",
        Spacing = "Spacing",
        SpacingCombining = "Spacing Combining",
        Superscript = "Superscript",
        Syllable = "Syllable",
        Composition = "Composition",
        Conjunct = "Conjunct",
        Lowercase = "Lowercase",
        Number = "Number",
        Other = "Other",
        Smallcaps = "Smallcaps",
        Uppercase = "Uppercase",
    }
}

vocabulary! {
    /// Letter case. A record without one has no case.
    Case {
        Upper = "upper",
        Lower = "lower",
        Minor = "minor",
        SmallCaps = "smallCaps",
    }
}

vocabulary! {
    /// Writing direction. Data files use `RTL` and `BIDI`; a missing value is left to right
    /// unless the script is written right to left. Source files may also store `VTR` and `VTL`.
    Direction {
        Ltr = "LTR",
        Rtl = "RTL",
        Bidi = "BIDI",
        Vtr = "VTR",
        Vtl = "VTL",
    }
}

vocabulary! {
    /// Unicode General_Category, by its two-letter alias.
    GeneralCategory {
        Lu = "Lu",
        Ll = "Ll",
        Lt = "Lt",
        Lm = "Lm",
        Lo = "Lo",
        Mn = "Mn",
        Mc = "Mc",
        Me = "Me",
        Nd = "Nd",
        Nl = "Nl",
        No = "No",
        Pc = "Pc",
        Pd = "Pd",
        Ps = "Ps",
        Pe = "Pe",
        Pi = "Pi",
        Pf = "Pf",
        Po = "Po",
        Sm = "Sm",
        Sc = "Sc",
        Sk = "Sk",
        So = "So",
        Zs = "Zs",
        Zl = "Zl",
        Zp = "Zp",
        Cc = "Cc",
        Cf = "Cf",
        Cs = "Cs",
        Co = "Co",
        Cn = "Cn",
    }
}

vocabulary! {
    /// Unicode Bidi_Class, by its alias.
    BidiClass {
        L = "L",
        R = "R",
        Al = "AL",
        En = "EN",
        Es = "ES",
        Et = "ET",
        An = "AN",
        Cs = "CS",
        Nsm = "NSM",
        Bn = "BN",
        B = "B",
        S = "S",
        Ws = "WS",
        On = "ON",
        Lre = "LRE",
        Lro = "LRO",
        Rle = "RLE",
        Rlo = "RLO",
        Pdf = "PDF",
        Lri = "LRI",
        Rli = "RLI",
        Fsi = "FSI",
        Pdi = "PDI",
    }
}

impl GeneralCategory {
    /// The category and subcategory a glyph gets from its character's general category when the
    /// database knows nothing better (05-font-view.md § 4.2, synthesis rule 5).
    pub fn glyph_category(self) -> (Category, Option<SubCategory>) {
        use GeneralCategory as G;
        match self {
            G::Lu | G::Ll | G::Lt | G::Lo => (Category::Letter, None),
            G::Lm => (Category::Letter, Some(SubCategory::Modifier)),
            G::Mn => (Category::Mark, Some(SubCategory::Nonspacing)),
            G::Mc => (Category::Mark, Some(SubCategory::SpacingCombining)),
            G::Me => (Category::Mark, Some(SubCategory::Enclosing)),
            G::Nd | G::No => (Category::Number, Some(SubCategory::DecimalDigit)),
            G::Nl => (Category::Number, None),
            G::Zs => (Category::Separator, Some(SubCategory::Space)),
            G::Zl | G::Zp | G::Cc => (Category::Separator, None),
            G::Cf => (Category::Separator, Some(SubCategory::Format)),
            G::Co => (Category::Letter, Some(SubCategory::Compatibility)),
            G::Pd => (Category::Punctuation, Some(SubCategory::Dash)),
            G::Ps | G::Pe => (Category::Punctuation, Some(SubCategory::Parenthesis)),
            G::Pc | G::Po => (Category::Punctuation, None),
            G::Pi | G::Pf => (Category::Punctuation, Some(SubCategory::Quote)),
            G::Sm => (Category::Symbol, Some(SubCategory::Math)),
            G::Sc => (Category::Symbol, Some(SubCategory::Currency)),
            G::Sk => (Category::Mark, Some(SubCategory::Spacing)),
            G::So | G::Cn => (Category::Symbol, None),
            // A surrogate never reaches here (glyph names cannot name one); treat it as unassigned.
            G::Cs => (Category::Symbol, None),
        }
    }
}

impl BidiClass {
    /// Strong right-to-left classes.
    pub fn is_rtl(self) -> bool {
        matches!(self, BidiClass::R | BidiClass::Al)
    }
}
