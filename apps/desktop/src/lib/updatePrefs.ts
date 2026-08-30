export const UPDATE_CHECK_ON_START_KEY = "late.updateCheckOnStart";

export function loadUpdateCheckOnStart(): boolean {
  try {
    const raw = localStorage.getItem(UPDATE_CHECK_ON_START_KEY);
    if (raw == null) return true;
    return raw === "1" || raw === "true";
  } catch {
    return true;
  }
}

export function persistUpdateCheckOnStart(on: boolean): void {
  try {
    localStorage.setItem(UPDATE_CHECK_ON_START_KEY, on ? "1" : "0");
  } catch {
    /* ignore */
  }
}
