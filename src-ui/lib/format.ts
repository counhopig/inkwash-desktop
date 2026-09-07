// Small formatting and normalisation helpers shared by the UI. These
// keep presentation logic out of components so the templates stay
// readable.

export function formatUtcOffset(offsetMinutes: number): string {
  const sign = offsetMinutes < 0 ? "-" : "+";
  const abs = Math.abs(offsetMinutes);
  const h = Math.floor(abs / 60);
  const m = abs % 60;
  return `UTC${sign}${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;
}

const COMMON_TZ: Array<{ name: string; offset: number }> = [
  { name: "Pacific/Honolulu", offset: -10 * 60 },
  { name: "America/Anchorage", offset: -9 * 60 },
  { name: "America/Los_Angeles", offset: -8 * 60 },
  { name: "America/Denver", offset: -7 * 60 },
  { name: "America/Chicago", offset: -6 * 60 },
  { name: "America/New_York", offset: -5 * 60 },
  { name: "America/Sao_Paulo", offset: -3 * 60 },
  { name: "Atlantic/Azores", offset: -1 * 60 },
  { name: "Europe/London", offset: 0 },
  { name: "Europe/Berlin", offset: 1 * 60 },
  { name: "Europe/Helsinki", offset: 2 * 60 },
  { name: "Europe/Moscow", offset: 3 * 60 },
  { name: "Asia/Dubai", offset: 4 * 60 },
  { name: "Asia/Karachi", offset: 5 * 60 },
  { name: "Asia/Kolkata", offset: 5 * 60 + 30 },
  { name: "Asia/Bangkok", offset: 7 * 60 },
  { name: "Asia/Shanghai", offset: 8 * 60 },
  { name: "Asia/Hong_Kong", offset: 8 * 60 },
  { name: "Asia/Singapore", offset: 8 * 60 },
  { name: "Asia/Tokyo", offset: 9 * 60 },
  { name: "Australia/Sydney", offset: 10 * 60 },
  { name: "Pacific/Auckland", offset: 12 * 60 },
];

export function listCommonTimezones(): Array<{ name: string; offset: number }> {
  return COMMON_TZ;
}

export function systemTimezoneOffsetMinutes(): number {
  return -new Date().getTimezoneOffset();
}

export function tzLabel(name: string, offset: number): string {
  return `${name} · ${formatUtcOffset(offset)}`;
}

export function redactSecret(value: string): string {
  const len = value.length;
  if (len <= 8) return "****";
  return `${value.slice(0, 4)}\u2026${value.slice(-4)}`;
}

export function isInsecureHttpUrl(value: string): boolean {
  return /^http:\/\//i.test(value.trim());
}

export function formatTimeShort(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

export function formatTimeWithDate(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}
