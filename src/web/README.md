# Winderoo Web UI

Modern, lightweight web interface for the Winderoo watch winder.

## Stack

- **Preact** - 3KB React alternative
- **Vite** - Lightning fast builds with tree-shaking
- **TypeScript** - Type safety
- **CSS Variables** - Native theming, no framework bloat

## Development

```bash
# Install dependencies
npm install

# Start dev server
npm run dev

# Build for production
npm run build

# Build + gzip for ESP32
npm run build:esp32
```

## Build Output

The `build:esp32` script:
1. Builds optimized production bundle
2. Gzips all assets (HTML, JS, CSS)
3. Copies gzipped files to `data/` directory for flashing to ESP32

## Bundle Size

Expected production bundle sizes (gzipped):
- **JS**: ~8-12KB
- **CSS**: ~2-3KB
- **HTML**: ~400 bytes

**Total: ~12-15KB** vs Angular's ~200KB+

## Architecture

```
src/
├── api.ts           # API client
├── types.ts         # TypeScript types
├── App.tsx          # Root component
├── main.tsx         # Entry point
├── components/
│   ├── Header.tsx   # Header with power toggle
│   ├── Settings.tsx # Main settings panel
│   └── Icons.tsx    # Inline SVG icons
├── hooks/
│   ├── useStore.tsx # Global state
│   └── useClock.ts  # RTC clock hook
├── i18n/            # Translations (5 languages)
└── styles/
    └── global.css   # CSS variables & utilities
```

## i18n

Supported languages:
- English (en-US)
- German (de-DE)
- Spanish (es-ES)
- French (fr-FR)
- Portuguese (pt-BR)

Language preference is saved to localStorage.

## Features

- 🎨 Dark theme with accent colors
- 📱 Mobile-first responsive design
- 🌍 5 language support
- ⚡ Instant page loads
- 🔄 Real-time status updates
- 📊 Progress tracking
