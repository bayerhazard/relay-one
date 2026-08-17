/** @type {import('tailwindcss').Config} */
export default {
  content: ["./src/**/*.{html,svelte,ts}"],
  theme: {
    extend: {
      colors: {
        am: {
          navy: "#051729",
          navy2: "#0a2238",
          navy3: "#142e47",
          gold: "#caa960",
          goldtext: "#8c6c1f",
        },
      },
      fontFamily: {
        sans: ['"Geist"', "sans-serif"],
        mono: ['"Geist Mono"', "monospace"],
      },
      fontSize: {
        "2xs": ["0.6875rem", { lineHeight: "1rem" }],
      },
    },
  },
  plugins: [],
};
