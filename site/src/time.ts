// The API speaks UTC; the page shows every time in the reader's time zone, taken from the
// browser unless they pick another.

import { useEffect, useState } from "react";

const STORAGE_KEY = "timeZone";

export function browserTimeZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone;
}

/** Every zone the browser knows, with UTC and its own zone even where its list lacks them. */
export function timeZones(): string[] {
  const zones = new Set(Intl.supportedValuesOf?.("timeZone") ?? []);
  zones.add("UTC");
  zones.add(browserTimeZone());
  return [...zones].sort();
}

function isTimeZone(zone: string): boolean {
  try {
    new Intl.DateTimeFormat(undefined, { timeZone: zone });
    return true;
  } catch {
    return false;
  }
}

function storedTimeZone(): string | null {
  try {
    const zone = window.localStorage.getItem(STORAGE_KEY);
    return zone && isTimeZone(zone) ? zone : null;
  } catch {
    return null;
  }
}

/** The chosen zone and a setter; `null` returns to the browser's zone. */
export function useTimeZone(): [string, (zone: string | null) => void] {
  const [zone, setZone] = useState(() => storedTimeZone() ?? browserTimeZone());
  const choose = (next: string | null) => {
    try {
      if (next) window.localStorage.setItem(STORAGE_KEY, next);
      else window.localStorage.removeItem(STORAGE_KEY);
    } catch {
      // Without storage the choice lasts for this visit only.
    }
    setZone(next ?? browserTimeZone());
  };
  return [zone, choose];
}

/** The current time, updated every `intervalMs`. */
export function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), intervalMs);
    return () => window.clearInterval(timer);
  }, [intervalMs]);
  return now;
}

export function formatTime(utc: string, zone: string): string {
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    timeZone: zone,
  }).format(new Date(utc));
}

export function formatDay(utc: string, zone: string): string {
  return new Intl.DateTimeFormat(undefined, {
    weekday: "long",
    day: "numeric",
    month: "long",
    timeZone: zone,
  }).format(new Date(utc));
}

export function formatDateTime(utc: string, zone: string): string {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
    timeZone: zone,
  }).format(new Date(utc));
}

/** The zone's short name at `utc`, such as "CET" or "GMT+1". */
export function zoneName(utc: string, zone: string): string {
  const parts = new Intl.DateTimeFormat(undefined, {
    timeZone: zone,
    timeZoneName: "short",
  }).formatToParts(new Date(utc));
  return parts.find((part) => part.type === "timeZoneName")?.value ?? zone;
}

/** "in 12 minutes", "2 hours ago". */
export function relative(utc: string, now: number): string {
  const seconds = Math.round((new Date(utc).getTime() - now) / 1000);
  const format = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  const abs = Math.abs(seconds);
  if (abs < 60) return format.format(seconds, "second");
  if (abs < 3600) return format.format(Math.round(seconds / 60), "minute");
  if (abs < 86400) return format.format(Math.round(seconds / 3600), "hour");
  return format.format(Math.round(seconds / 86400), "day");
}

/** "3 d 4 h", "5 h 12 min", "42 min". */
export function duration(seconds: number): string {
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days > 0) return `${days} d ${hours} h`;
  if (hours > 0) return `${hours} h ${minutes} min`;
  return `${minutes} min`;
}
