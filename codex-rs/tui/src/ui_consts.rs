//! Shared UI constants for layout and alignment within the TUI.

/// Width (in terminal columns) reserved for the left gutter/prefix used by
/// live cells and aligned widgets.
///
/// Semantics:
/// - Chat composer reserves this many columns for the left border + padding.
/// - Status indicator lines begin with this many spaces for alignment.
/// - User history lines account for this many columns (e.g., "▌ ") when wrapping.
pub(crate) const LIVE_PREFIX_COLS: u16 = 2;

/// Adaptive layout constants for responsive UI behavior
/// These constants help create a more polished and context-aware interface

/// Terminal width thresholds for different layout modes
pub(crate) const TERMINAL_WIDTH_COMPACT: u16 = 80;
pub(crate) const TERMINAL_WIDTH_STANDARD: u16 = 120;
pub(crate) const TERMINAL_WIDTH_WIDE: u16 = 160;

/// Minimum terminal dimensions for usable interface
pub(crate) const MIN_TERMINAL_WIDTH: u16 = 60;
pub(crate) const MIN_TERMINAL_HEIGHT: u16 = 10;

/// Adaptive spacing based on terminal size
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutMode {
    Compact,   // < 80 cols: minimal spacing, compact layout
    Standard,  // 80-120 cols: normal spacing
    Wide,      // 120-160 cols: generous spacing
    UltraWide, // > 160 cols: maximum spacing
}

impl LayoutMode {
    pub(crate) fn from_width(width: u16) -> Self {
        match width {
            w if w < TERMINAL_WIDTH_COMPACT => LayoutMode::Compact,
            w if w < TERMINAL_WIDTH_STANDARD => LayoutMode::Standard,
            w if w < TERMINAL_WIDTH_WIDE => LayoutMode::Wide,
            _ => LayoutMode::UltraWide,
        }
    }

    /// Get vertical spacing (number of blank lines) for this layout mode
    pub(crate) fn vertical_spacing(self) -> u16 {
        match self {
            LayoutMode::Compact => 0,
            LayoutMode::Standard => 1,
            LayoutMode::Wide => 1,
            LayoutMode::UltraWide => 2,
        }
    }

    /// Get horizontal padding for this layout mode
    pub(crate) fn horizontal_padding(self) -> u16 {
        match self {
            LayoutMode::Compact => 1,
            LayoutMode::Standard => 2,
            LayoutMode::Wide => 3,
            LayoutMode::UltraWide => 4,
        }
    }

    /// Get maximum content width for readable text
    pub(crate) fn max_content_width(self, available_width: u16) -> u16 {
        let padding = self.horizontal_padding() * 2;
        let max_readable = match self {
            LayoutMode::Compact => available_width.saturating_sub(padding),
            LayoutMode::Standard => (available_width.saturating_sub(padding)).min(100),
            LayoutMode::Wide => (available_width.saturating_sub(padding)).min(120),
            LayoutMode::UltraWide => (available_width.saturating_sub(padding)).min(140),
        };
        max_readable.max(MIN_TERMINAL_WIDTH.saturating_sub(padding))
    }

    /// Should show detailed information in this layout mode?
    pub(crate) fn show_detailed_info(self) -> bool {
        matches!(self, LayoutMode::Wide | LayoutMode::UltraWide)
    }

    /// Should use compact widget variants?
    pub(crate) fn use_compact_widgets(self) -> bool {
        matches!(self, LayoutMode::Compact)
    }
}

/// Context-aware spacing utilities
pub(crate) struct SmartSpacing;

impl SmartSpacing {
    /// Calculate adaptive bottom padding for panels
    pub(crate) fn bottom_padding(layout_mode: LayoutMode, is_focused: bool) -> u16 {
        let base_padding = layout_mode.vertical_spacing();
        if is_focused {
            base_padding.saturating_add(1) // Extra padding for focused elements
        } else {
            base_padding
        }
    }

    /// Calculate spacing between UI sections
    pub(crate) fn section_spacing(layout_mode: LayoutMode, has_content: bool) -> u16 {
        if !has_content {
            return 0;
        }
        match layout_mode {
            LayoutMode::Compact => 0,
            LayoutMode::Standard => 1,
            LayoutMode::Wide | LayoutMode::UltraWide => 2,
        }
    }

    /// Calculate minimum height for interactive elements
    pub(crate) fn min_interactive_height(layout_mode: LayoutMode) -> u16 {
        match layout_mode {
            LayoutMode::Compact => 3,
            LayoutMode::Standard => 4,
            LayoutMode::Wide => 5,
            LayoutMode::UltraWide => 6,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_mode_from_width() {
        assert_eq!(LayoutMode::from_width(60), LayoutMode::Compact);
        assert_eq!(LayoutMode::from_width(80), LayoutMode::Standard);
        assert_eq!(LayoutMode::from_width(120), LayoutMode::Wide);
        assert_eq!(LayoutMode::from_width(200), LayoutMode::UltraWide);
    }

    #[test]
    fn adaptive_spacing() {
        let compact = LayoutMode::Compact;
        let wide = LayoutMode::UltraWide;

        assert_eq!(compact.vertical_spacing(), 0);
        assert_eq!(wide.vertical_spacing(), 2);

        assert_eq!(compact.horizontal_padding(), 1);
        assert_eq!(wide.horizontal_padding(), 4);
    }

    #[test]
    fn content_width_limits() {
        let mode = LayoutMode::Wide;
        let max_width = mode.max_content_width(200);
        assert!(max_width <= 120); // Should limit readability
        assert!(max_width >= 50); // Should maintain minimum usability
    }

    #[test]
    fn smart_spacing_bottom_padding() {
        let padding_focused = SmartSpacing::bottom_padding(LayoutMode::Standard, true);
        let padding_unfocused = SmartSpacing::bottom_padding(LayoutMode::Standard, false);
        assert!(padding_focused > padding_unfocused);
    }
}
