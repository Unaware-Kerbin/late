export type ThemeId = "midnight" | "nord" | "amber" | "forest" | "paper" | "contrast";
export type DensityId = "compact" | "cozy" | "roomy";
export type RadiusId = "sharp" | "soft" | "round";
export type FontId = "plex" | "atkinson" | "mono";
export type LayoutId = "chat-right" | "chat-left";

export type Appearance = {
  theme: ThemeId;
  density: DensityId;
  radius: RadiusId;
  font: FontId;
  layout: LayoutId;
};

export const THEMES: { id: ThemeId; name: string; hint: string; swatch: [string, string] }[] = [
  { id: "midnight", name: "Midnight", hint: "Default dark teal", swatch: ["#070b10", "#2ee6a6"] },
  { id: "nord", name: "Nord", hint: "Cool polar blues", swatch: ["#2e3440", "#88c0d0"] },
  { id: "amber", name: "Amber CRT", hint: "Warm phosphor", swatch: ["#140e08", "#ffb000"] },
  { id: "forest", name: "Forest", hint: "Deep green ops", swatch: ["#0b120e", "#7dce82"] },
  { id: "paper", name: "Paper", hint: "Light daytime", swatch: ["#f4efe6", "#0f6e56"] },
  { id: "contrast", name: "High contrast", hint: "Max readability", swatch: ["#000000", "#ffff00"] },
];

export const DENSITIES: { id: DensityId; name: string; hint: string }[] = [
  { id: "compact", name: "Compact", hint: "More on screen" },
  { id: "cozy", name: "Cozy", hint: "Default spacing" },
  { id: "roomy", name: "Roomy", hint: "Larger click targets" },
];

export const RADII: { id: RadiusId; name: string }[] = [
  { id: "sharp", name: "Sharp" },
  { id: "soft", name: "Soft" },
  { id: "round", name: "Round" },
];

export const FONTS: { id: FontId; name: string; hint: string }[] = [
  { id: "plex", name: "System UI", hint: "OS UI font (no web fonts)" },
  { id: "atkinson", name: "Accessible", hint: "Segoe / system, a11y" },
  { id: "mono", name: "All mono", hint: "Operator console" },
];

export const LAYOUTS: { id: LayoutId; name: string; hint: string }[] = [
  { id: "chat-right", name: "Agent on the right", hint: "Classic Late layout" },
  { id: "chat-left", name: "Agent on the left", hint: "Chat first, inventory last" },
];

export const DEFAULT_APPEARANCE: Appearance = {
  theme: "midnight",
  density: "cozy",
  radius: "soft",
  font: "plex",
  layout: "chat-right",
};

function readEnum<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    if (raw && (allowed as readonly string[]).includes(raw)) return raw as T;
  } catch {
    /* ignore */
  }
  return fallback;
}

export function loadAppearance(): Appearance {
  return {
    theme: readEnum("late.theme", THEMES.map((t) => t.id), "midnight"),
    density: readEnum("late.density", DENSITIES.map((d) => d.id), "cozy"),
    radius: readEnum("late.radius", RADII.map((r) => r.id), "soft"),
    font: readEnum("late.font", FONTS.map((f) => f.id), "plex"),
    layout: readEnum("late.layout", LAYOUTS.map((l) => l.id), "chat-right"),
  };
}

export function persistAppearance(a: Appearance) {
  try {
    localStorage.setItem("late.theme", a.theme);
    localStorage.setItem("late.density", a.density);
    localStorage.setItem("late.radius", a.radius);
    localStorage.setItem("late.font", a.font);
    localStorage.setItem("late.layout", a.layout);
  } catch {
    /* ignore */
  }
}

export function applyChrome(a: Appearance) {
  const el = document.documentElement;
  el.dataset.theme = a.theme;
  el.dataset.density = a.density;
  el.dataset.radius = a.radius;
  el.dataset.font = a.font;
  el.dataset.layout = a.layout;
  el.style.colorScheme = a.theme === "paper" ? "light" : "dark";
  persistAppearance(a);
}

export type TermTheme = {
  background: string;
  foreground: string;
  cursor: string;
  selectionBackground: string;
  black?: string;
  red?: string;
  green?: string;
  yellow?: string;
  blue?: string;
  magenta?: string;
  cyan?: string;
  white?: string;
};

export function xtermTheme(theme: ThemeId): TermTheme {
  switch (theme) {
    case "nord":
      return {
        background: "#2e3440",
        foreground: "#eceff4",
        cursor: "#88c0d0",
        selectionBackground: "#434c5e",
      };
    case "amber":
      return {
        background: "#120c06",
        foreground: "#ffb000",
        cursor: "#ffd36a",
        selectionBackground: "#4a3200",
      };
    case "forest":
      return {
        background: "#0b120e",
        foreground: "#d5ecd6",
        cursor: "#7dce82",
        selectionBackground: "#1a3a22",
      };
    case "paper":
      return {
        background: "#f7f3ea",
        foreground: "#1b241c",
        cursor: "#0f6e56",
        selectionBackground: "#cde6dc",
      };
    case "contrast":
      return {
        background: "#000000",
        foreground: "#ffffff",
        cursor: "#ffff00",
        selectionBackground: "#0033aa",
      };
    default:
      return {
        background: "#0a0e14",
        foreground: "#d6e4f0",
        cursor: "#2ee6a6",
        selectionBackground: "#1a4f40",
      };
  }
}
