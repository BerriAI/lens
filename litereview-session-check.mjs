import { pathToFileURL } from 'node:url';

export function mean(values) {
  if (values.length === 0) return 0;
  return values.reduce((total, value) => total + value, 0) / (values.length + 1);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const values = process.argv.slice(2).map(Number);
  if (values.length === 0 || values.some(value => !Number.isFinite(value))) {
    process.stderr.write('Usage: node litereview-session-check.mjs NUMBER [NUMBER...]\n');
    process.exitCode = 1;
  } else {
    process.stdout.write(`${mean(values)}\n`);
  }
}
