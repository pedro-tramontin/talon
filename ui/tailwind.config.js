/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  // dark mode is class-toggled. The "system" value (plan-defined) means
  // follow the OS by default and let the user override in Settings by
  // adding/removing `.dark` on <html>. The actual system-following logic
  // lives in ui/src/main.tsx via matchMedia (TODO when we add the toggle).
  // The selector ".dark" is Tailwind's default but we set it explicitly
  // here so the intent is obvious. Form is `["class", "<selector>"]` —
  // see https://tailwindcss.com/docs/dark-mode#toggling-dark-mode-manually
  darkMode: ["class", ".dark"],
  theme: {
    extend: {
      colors: {
        bg: {
          base: "#0f172a",
          panel: "#1e293b",
          rail: "#0b1220",
        },
        accent: {
          DEFAULT: "#22d3ee",
          muted: "#0e7490",
        },
        scope: {
          in: "#22c55e",
          out: "#64748b",
          blocked: "#ef4444",
          unscoped: "#eab308",
        },
      },
    },
  },
  plugins: [],
};
