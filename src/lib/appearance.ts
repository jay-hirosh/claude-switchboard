import { create } from 'zustand';

export type AccentColor =
  | 'terracotta'
  | 'gold'
  | 'sage'
  | 'teal'
  | 'sky'
  | 'indigo'
  | 'violet'
  | 'plum'
  | 'berry'
  | 'graphite';
export type Density = 'comfortable' | 'compact';

const ACCENT_COLORS: readonly AccentColor[] = [
  'terracotta',
  'gold',
  'sage',
  'teal',
  'sky',
  'indigo',
  'violet',
  'plum',
  'berry',
  'graphite',
];

const ACCENT_KEY = 'accent-color';
const DENSITY_KEY = 'density';
const REDUCE_MOTION_KEY = 'reduce-motion';

export function readStoredAccentColor(): AccentColor {
  if (typeof localStorage === 'undefined') return 'terracotta';
  const raw = localStorage.getItem(ACCENT_KEY);
  return (ACCENT_COLORS as string[]).includes(raw ?? '') ? (raw as AccentColor) : 'terracotta';
}

export function readStoredDensity(): Density {
  if (typeof localStorage === 'undefined') return 'comfortable';
  const raw = localStorage.getItem(DENSITY_KEY);
  return raw === 'comfortable' || raw === 'compact' ? raw : 'comfortable';
}

export function readStoredReduceMotion(): boolean {
  if (typeof localStorage === 'undefined') return false;
  return localStorage.getItem(REDUCE_MOTION_KEY) === 'true';
}

interface AppearanceStore {
  accentColor: AccentColor;
  density: Density;
  reduceMotion: boolean;
  setAccentColor: (color: AccentColor) => void;
  setDensity: (density: Density) => void;
  setReduceMotion: (reduceMotion: boolean) => void;
}

function persist(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Safari private mode / quota-exceeded — preference still applies for
    // this session, just isn't persisted across launches. Same rationale as
    // theme.ts's setThemePreference.
  }
}

export const useAppearanceStore = create<AppearanceStore>((set) => ({
  accentColor: readStoredAccentColor(),
  density: readStoredDensity(),
  reduceMotion: readStoredReduceMotion(),
  setAccentColor: (color) => {
    persist(ACCENT_KEY, color);
    set({ accentColor: color });
  },
  setDensity: (density) => {
    persist(DENSITY_KEY, density);
    set({ density });
  },
  setReduceMotion: (reduceMotion) => {
    persist(REDUCE_MOTION_KEY, String(reduceMotion));
    set({ reduceMotion });
  },
}));
