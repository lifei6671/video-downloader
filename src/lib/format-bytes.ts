export function bytes(value: number | null) {
  if (value === null) return '—';
  if (value < 1024) return value + ' B';
  if (value < 1048576) return (value / 1024).toFixed(1) + ' KB';
  return (value / 1048576).toFixed(1) + ' MB';
}
