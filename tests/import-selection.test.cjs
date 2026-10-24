const { test } = require('node:test');
const assert = require('node:assert/strict');
const Selection = require('../frontend/import-selection.js');
const preview = (bvid, cids, selected_page = 1, id = bvid) => ({
  bvid, id, selected_page, parts: cids.map((cid, i) => ({ cid, page: i + 1 })),
});

test('single P and explicit multi P selection combine across video pages', () => {
  const selected = new Selection();
  selected.register(preview('single', [10]), true);
  selected.register(preview('multi', [20, 30, 40], 2), true);
  assert.equal(selected.count, 2);
  assert.deepEqual(selected.payload(), [{ preview_id: 'single', cids: [10] }, { preview_id: 'multi', cids: [30] }]);
  selected.setAll('multi', true);
  assert.equal(selected.count, 4);
  selected.set('multi', 20, false);
  assert.deepEqual(selected.payload()[1].cids, [30, 40]);
});

test('refresh renews native preview, keeps selected CIDs and drops removed parts', () => {
  const selected = new Selection();
  selected.register(preview('multi', [20, 30]));
  selected.setAll('multi', true);
  selected.register(preview('multi', [30, 40], 1, 'refreshed'));
  assert.deepEqual(selected.payload(), [{ preview_id: 'refreshed', cids: [30] }]);
  assert.throws(() => selected.set('multi', 20, true), /重新解析/);
});

test('partial batch success removes only accepted tracks and leaves recoverable selections', () => {
  const selected = new Selection();
  selected.register(preview('multi', [10, 20, 30]));
  selected.setAll('multi', true);
  selected.accept({ items: [
    { preview_id: 'multi', cid: 10, job: { id: 'job-1' } },
    { preview_id: 'multi', cid: 20, error: '已在曲库' },
    { preview_id: 'multi', cid: 30, error: '队列已满' },
  ] });
  assert.deepEqual(selected.payload(), [{ preview_id: 'multi', cids: [20, 30] }]);
  selected.clearSelection();
  assert.equal(selected.count, 0);
  assert.equal(selected.records.size, 1);
});

test('100 track limit rejects all-selection atomically and ignores duplicate selections', () => {
  const selected = new Selection();
  selected.register(preview('long', Array.from({ length: 101 }, (_, i) => i + 1)));
  assert.throws(() => selected.setAll('long', true), /超过 100/);
  assert.equal(selected.count, 0);
  for (let i = 1; i <= 100; i++) selected.set('long', i, true);
  selected.set('long', 1, true);
  assert.equal(selected.count, 100);
  assert.throws(() => selected.set('long', 101, true), /100/);
  assert.equal(selected.count, 100);
});
