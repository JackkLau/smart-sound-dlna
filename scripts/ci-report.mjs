export function reportFailure(title, error) {
  const raw = String(error.stderr || error.message || error).slice(-2500);
  const escaped = raw.replaceAll('%', '%25').replaceAll('\r', '%0D').replaceAll('\n', '%0A');
  process.stderr.write(process.env.GITHUB_ACTIONS === 'true' ? `::error title=${title}::${escaped}\n` : `${raw}\n`);
  process.exitCode = 1;
}
