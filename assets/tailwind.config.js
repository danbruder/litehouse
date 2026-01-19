// tailwind.config.js
/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        // Warm Infrastructure palette
        litehouse: {
          // neutrals
          bg: "#F7F6F3",
          surface: "#FFFFFF",
          text: "#2B2B2B",
          muted: "#6B6B6B",
          border: "#E3E1DC",

          // brand amber
          amber: "#F2A900",
          amberDeep: "#C98200",
          glow: "#FFE6B3",

          // supporting cool
          slateBlue: "#4A6FA5",
          mistBlue: "#E6EEF7",

          // status
          success: "#6FAE7B",
          warning: "#E0B15C",
          error: "#C96B6B",
        },
      },

      borderRadius: {
        // soft containers
        xl: "12px",
        "2xl": "16px",
      },

      boxShadow: {
        // gentle elevation
        soft: "0 1px 3px rgba(0,0,0,0.06)",
      },
    },
  },
  plugins: [],
};

