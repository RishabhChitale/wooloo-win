// wooloo palette (Catppuccin Mocha), copied from WoolooTheme.swift (default theme "catppuccin").
export const theme = {
  accent: "#89B4FA", panel: "#181825", activeRow: "#1E1E2E", selection: "#313244",
  surface0: "#313244", surface1: "#45475A", overlay0: "#6C7086", overlay1: "#7F849C",
  text: "#CDD6F4", subtext: "#A6ADC8", mauve: "#CBA6F7", green: "#A6E3A1", yellow: "#F9E2AF",
  red: "#F38BA8", blue: "#89B4FA", teal: "#94E2D5", peach: "#FAB387",
  background: "#1E1E2E", foreground: "#CDD6F4", cursor: "#F5E0DC", selectionBackground: "#585B70",
};

// Category heading lines: SwiftUI-style system colours for the colour names the Swift sidebar offers.
export const categoryLineColors: Record<string, string> = {
  claude: theme.blue, school: theme.yellow, creative: theme.mauve, finance: theme.green, misc: theme.overlay0,
};
