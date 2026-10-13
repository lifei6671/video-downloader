// Display only: keep native paths unchanged for all filesystem commands.
export function displayPath(path: string | null | undefined): string {
  if (!path) return '';
  const prefix = '\\\\?\\';
  if (path.startsWith(prefix + 'UNC\\')) return '\\\\' + path.slice(8);
  if (path.startsWith(prefix) && /^[A-Za-z]:\\/.test(path.slice(4))) return path.slice(4);
  return path;
}
