/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  // follow OS by default; user override in Settings adds/removes `.dark`
  // class on <html>. The plan defines three values: "dark" | "light" | "system".
  darkMode: ["class", "media"],
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
