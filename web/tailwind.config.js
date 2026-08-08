/** @type {import('tailwindcss').Config} */
export default {
  content: ["./src/**/*.{html,svelte,ts}"],
  theme: {
    extend: {
      colors: {
        mail: {
          sidebar: "#f5f5f7",
          list: "#ffffff",
          preview: "#fafafa",
          border: "#e5e5e5",
          accent: "#007aff",
          accentHover: "#0056cc",
          text: "#1d1d1f",
          textSecondary: "#6e6e73",
          danger: "#ff3b30",
          warning: "#ff9500",
          success: "#34c759",
        },
      },
      fontFamily: {
        sans: [
          "-apple-system",
          "BlinkMacSystemFont",
          '"SF Pro Text"',
          '"Helvetica Neue"',
          "sans-serif",
        ],
      },
      fontSize: {
        "2xs": ["0.6875rem", { lineHeight: "1rem" }],
      },
    },
  },
  plugins: [],
};
