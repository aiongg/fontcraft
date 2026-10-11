//! The web preview server (desktop only): serves the preview page and the hinted font to the local network on a free port while the preview-address dialog is open (owner decision M10 § 7, question 5). One page and one font, 4 connections at a time, 64 KB of request headers.
//!
//! Rows: `hint.tt.showPreviewAddress`. Package M10.30 fills this module.
