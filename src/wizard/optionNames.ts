// SPDX-License-Identifier: GPL-3.0-or-later
// Friendly names for firmware build options (Build API names such as USE_OSD_HD). The
// names are opaque to the UI (the mapping from answers to options is backend-only); a
// name without a translation under `buildOptions.<NAME>` is shown as-is.
type T = (key: string) => string;
type Te = (key: string) => boolean;

export function optionLabel(name: string, t: T, te: Te): string {
  const key = `buildOptions.${name}`;
  return /^[A-Z0-9_]+$/.test(name) && te(key) ? t(key) : name;
}

export function optionList(names: readonly string[], t: T, te: Te): string {
  return names.map((n) => optionLabel(n, t, te)).join(", ");
}
