import { test, expect } from 'vitest';
import { readFileSync } from 'node:fs';

for (const source of [
  'frontend/index.html',
  'mobile/index.html',
  'xiaomi_dlna/static/index.html',
]) {
  test(`${source} mounts a React module instead of containing an imperative UI`, () => {
    const html = readFileSync(source, 'utf8');
    expect(html).toContain('id="root"');
    expect(html).toContain('type="module"');
    expect(html).toContain('/src/main.jsx');
    expect(html).not.toMatch(/<(?:form|button|dialog)\b/);
  });
}
