export function formatTime(seconds: number, precision = 3): string {
  if (!Number.isFinite(seconds)) return `00:00.${'0'.repeat(precision)}`
  const safe = Math.max(0, seconds)
  const minutes = Math.floor(safe / 60)
  const remainder = safe - minutes * 60
  return `${minutes.toString().padStart(2, '0')}:${remainder
    .toFixed(precision)
    .padStart(2 + (precision ? precision + 1 : 0), '0')}`
}

export function clampTime(seconds: number, duration: number): number {
  return Math.min(Math.max(0, seconds), Number.isFinite(duration) ? duration : seconds)
}
