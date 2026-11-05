import type { Period } from "./api";

// Dates are shown in the Mac's local time and language.

export function clock(iso: string): string {
  return new Date(iso).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

function startOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

/** "2:41 PM" today, "Yesterday", or "Oct 28". */
export function when(iso: string, now = new Date()): string {
  const date = new Date(iso);
  const days = Math.round((startOfDay(now).getTime() - startOfDay(date).getTime()) / 86_400_000);
  if (days === 0) return clock(iso);
  if (days === 1) return `Yesterday, ${clock(iso)}`;
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export function dayHeading(iso: string, now = new Date()): string {
  const date = new Date(iso);
  const days = Math.round((startOfDay(now).getTime() - startOfDay(date).getTime()) / 86_400_000);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  return date.toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" });
}

export function longDate(now = new Date()): string {
  return now.toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" });
}

export function dateTime(iso: string): string {
  return new Date(iso).toLocaleString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

export type Range = "today" | "7d" | "30d";

/** A local-time period ending now: since midnight, or the last 7 or 30 days. */
export function period(range: Range, now = new Date()): Period {
  const from = startOfDay(now);
  if (range === "7d") from.setDate(from.getDate() - 6);
  if (range === "30d") from.setDate(from.getDate() - 29);
  return { from: from.toISOString(), to: now.toISOString() };
}

export const rangeWords: Record<Range, string> = {
  today: "today",
  "7d": "in the last 7 days",
  "30d": "in the last 30 days",
};

export function minutesToTime(minutes: number): string {
  const date = new Date(2000, 0, 1, Math.floor(minutes / 60), minutes % 60);
  return date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}
