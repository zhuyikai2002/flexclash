/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{vue,js,ts,jsx,tsx}'],
  theme: {
    extend: {
      colors: {
        brand: {
          500: '#6366f1',
          600: '#4f46e5',
        },
      },
      // Half-step sizes for icons inside 40x40 chips. Without these
      // Tailwind silently drops `h-4.5` and the icon collapses to its
      // intrinsic size (which is much larger than intended).
      spacing: {
        '4.5': '1.125rem',  // 18px
      },
      width: {
        '4.5': '1.125rem',
      },
      height: {
        '4.5': '1.125rem',
      },
    },
  },
  plugins: [],
}
