/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ["./popup.tsx", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      borderRadius: {
        lg: "8px",
        md: "7px",
        sm: "6px"
      },
      colors: {
        background: "var(--reshape-background)",
        border: "var(--reshape-border)",
        input: "var(--reshape-input)",
        muted: "var(--reshape-muted)",
        primary: "var(--reshape-primary)",
        "primary-foreground": "var(--reshape-primary-text)",
        ring: "var(--reshape-ring)",
        surface: "var(--reshape-surface)",
        "surface-elevated": "var(--reshape-surface-elevated)",
        "surface-muted": "var(--reshape-surface-muted)",
        text: "var(--reshape-text)",
        warning: "var(--reshape-warning-surface)",
        "warning-foreground": "var(--reshape-warning-text)"
      },
      boxShadow: {
        bubble: "var(--reshape-shadow)"
      }
    }
  },
  plugins: []
}
