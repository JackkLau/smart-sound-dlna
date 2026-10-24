const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');

test('desktop controls have unique IDs so folder import and favorite open cannot share handlers', () => {
  const ids = [...readFileSync('frontend/index.html', 'utf8').matchAll(/\bid="([^"]+)"/g)].map((match) => match[1]);
  const duplicates = ids.filter((id, i) => ids.indexOf(id) !== i);
  assert.deepEqual(duplicates, []);
});
