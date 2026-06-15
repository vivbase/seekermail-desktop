/** @type {import('tailwindcss').Config} */
// Tailwind theme maps onto the Seeker design tokens (07 §7). Utilities resolve to
// CSS variables from src/styles/tokens.css — components never hardcode hex/px.
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        // Parchment surface scale
        p1: "var(--p1)",
        p2: "var(--p2)",
        p3: "var(--p3)",
        p4: "var(--p4)",
        p5: "var(--p5)",
        p6: "var(--p6)",
        p7: "var(--p7)",
        p8: "var(--p8)",
        p9: "var(--p9)",
        p10: "var(--p10)",
        // Semantic surfaces / text
        surface: "var(--p1)",
        parchment: "var(--p3)",
        divider: "var(--p5)",
        // Account / card accents
        terra: "var(--terra)",
        slate: "var(--slate)",
        sage: "var(--sage)",
        amber: "var(--amber)",
        red: "var(--red)",
        green: "var(--green)",
      },
      fontFamily: {
        display: "var(--fd)",
        body: "var(--fb)",
        ui: "var(--fu)",
        mono: "var(--fm)",
      },
      borderRadius: {
        card: "var(--radius-card)",
        chip: "var(--radius-chip)",
        avatar: "var(--radius-avatar)",
      },
      boxShadow: {
        card: "var(--shadow-card)",
      },
    },
  },
  plugins: [],
};
