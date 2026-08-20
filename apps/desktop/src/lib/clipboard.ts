export async function clipboardRead(): Promise<string> {
  try {
    if (window.lateRuntime?.clipboardRead) {
      return ((await window.lateRuntime.clipboardRead()) || "").slice(0, 2_000_000);
    }
    return await navigator.clipboard.readText();
  } catch {
    return "";
  }
}

export async function clipboardWrite(
  text: string,
  which: "clipboard" | "selection" = "clipboard",
): Promise<boolean> {
  try {
    if (window.lateRuntime?.clipboardWrite) {
      return Boolean(await window.lateRuntime.clipboardWrite(text, which));
    }
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
